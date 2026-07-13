#![forbid(unsafe_code)]

use forge_cards::runtime::{
    bind_activated_effect_actions, bind_triggered_ability_actions, compile_card_program,
    ActivatedAbilityProgram, ActivatedEffectProgram, CardProgram, EffectProgram, ExecutionBindings,
    StaticAbilityProgram, TriggeredEventProgram,
};
use forge_core::{
    apply, auto_payment_plan, Action, ActivationCondition, ActivationTiming, AttackDeclaration,
    BaseCreatureCharacteristics, BaseObjectCharacteristics, BlockDeclaration, CardId,
    CombatRestriction, CombatRestrictionSubject, CounterKind, GameState, ManaKind, ManaPool,
    ObjectColors, ObjectSupertypes, ObjectTargetPredicate, ObjectTypes, Outcome, PlayerRule,
    RestrictionEffect, StateError, Step, TargetChoice, TargetControllerPredicate, TargetKind,
    TargetRequirement, TriggerCondition, TriggerObjectFilter, ZoneId, ZoneKind,
};
use forge_testkit::runtime_smoke::{run_translated_card_runtime_smoke, RuntimeSmokeResult};
use serde_json::json;
use std::{env, fs, process::ExitCode};

fn mana_label(pool: ManaPool) -> String {
    let mut label = String::new();
    for (kind, symbol) in [
        (ManaKind::White, "W"),
        (ManaKind::Blue, "U"),
        (ManaKind::Black, "B"),
        (ManaKind::Red, "R"),
        (ManaKind::Green, "G"),
        (ManaKind::Colorless, "C"),
    ] {
        for _ in 0..pool.get(kind) {
            label.push('{');
            label.push_str(symbol);
            label.push('}');
        }
    }
    label
}

fn one_required_type(types: ObjectTypes) -> ObjectTypes {
    if types.artifact() {
        ObjectTypes::none().with_artifact()
    } else if types.creature() {
        ObjectTypes::none().with_creature()
    } else if types.enchantment() {
        ObjectTypes::none().with_enchantment()
    } else if types.instant() {
        ObjectTypes::none().with_instant()
    } else if types.land() {
        ObjectTypes::none().with_land()
    } else if types.planeswalker() {
        ObjectTypes::none().with_planeswalker()
    } else if types.sorcery() {
        ObjectTypes::none().with_sorcery()
    } else {
        ObjectTypes::none()
    }
}

fn condition_base(predicate: ObjectTargetPredicate) -> Option<BaseObjectCharacteristics> {
    let mut types = predicate.required_types();
    if predicate.required_any_types() != ObjectTypes::none() {
        types = types.union(one_required_type(predicate.required_any_types()));
    }
    if types == ObjectTypes::none() || types.intersects(predicate.forbidden_types()) {
        return None;
    }
    Some(
        BaseObjectCharacteristics::new(types, ObjectColors::none())
            .with_subtypes(predicate.required_subtypes()),
    )
}

fn setup_activation_condition(
    state: &mut GameState,
    controller: forge_core::PlayerId,
    source: forge_core::ObjectId,
    condition: ActivationCondition,
    matching_count: u32,
    salt: u32,
) -> bool {
    let ActivationCondition::ControllerControlsAtLeast { predicate, .. } = condition;
    let Some(base) = condition_base(predicate) else {
        return false;
    };
    for index in 0..matching_count {
        let object = if index == 0 {
            source
        } else {
            match apply(
                state,
                Action::CreateObject {
                    card: CardId::new(salt.wrapping_add(index)),
                    owner: controller,
                    controller,
                    zone: ZoneId::new(None, ZoneKind::Battlefield),
                },
            ) {
                Outcome::ObjectCreated(object) => object,
                _ => return false,
            }
        };
        if !matches!(
            apply(state, Action::SetBaseObjectCharacteristics { object, base }),
            Outcome::Applied
        ) {
            return false;
        }
    }
    true
}

fn condition_minimum(program: ActivatedAbilityProgram) -> Option<u32> {
    program.condition().map(|condition| match condition {
        ActivationCondition::ControllerControlsAtLeast { count, .. } => count,
    })
}

fn rejects_below_activation_condition(program: ActivatedAbilityProgram, salt: u32) -> Option<bool> {
    let condition = program.condition()?;
    let minimum = condition_minimum(program)?;
    let output = *program.output_choices().options().first()?;
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(false),
    };
    let source = match apply(
        &mut state,
        Action::CreateObject {
            card: CardId::new(salt),
            owner: controller,
            controller,
            zone: ZoneId::new(None, ZoneKind::Battlefield),
        },
    ) {
        Outcome::ObjectCreated(object) => object,
        _ => return Some(false),
    };
    if !setup_activation_condition(
        &mut state,
        controller,
        source,
        condition,
        minimum.saturating_sub(1),
        salt.wrapping_add(10),
    ) {
        return Some(false);
    }
    let Some(definition) = program.bind_selected(controller, source, output) else {
        return Some(false);
    };
    let ability = match apply(&mut state, Action::RegisterActivatedAbility { definition }) {
        Outcome::ActivatedAbilityRegistered(ability) => ability,
        _ => return Some(false),
    };
    let Some(payment) = auto_payment_plan(ManaPool::empty(), program.cost().mana())
        .ok()
        .flatten()
    else {
        return Some(false);
    };
    let outcome = apply(
        &mut state,
        Action::ActivateAbility {
            player: controller,
            ability,
            payment,
        },
    );
    Some(
        matches!(
            outcome,
            Outcome::Failed(StateError::ActivationConditionNotMet(failed)) if failed == ability
        ) && state.mana_pool(controller).ok() == Some(ManaPool::empty())
            && state.object(source).is_some_and(|record| !record.tapped()),
    )
}

fn replay_mana_ability(program: ActivatedAbilityProgram, salt: u32) -> serde_json::Value {
    let legal_outputs = program
        .output_choices()
        .options()
        .iter()
        .copied()
        .map(mana_label)
        .collect::<Vec<_>>();
    let mut replayed_outputs = Vec::new();
    for (index, output) in program
        .output_choices()
        .options()
        .iter()
        .copied()
        .enumerate()
    {
        let mut state = GameState::new();
        let controller = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            _ => continue,
        };
        let source = match apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(salt.wrapping_add(index as u32)),
                owner: controller,
                controller,
                zone: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ) {
            Outcome::ObjectCreated(object) => object,
            _ => continue,
        };
        if let Some(condition) = program.condition() {
            let Some(minimum) = condition_minimum(program) else {
                continue;
            };
            if !setup_activation_condition(
                &mut state,
                controller,
                source,
                condition,
                minimum,
                salt.wrapping_add(50),
            ) {
                continue;
            }
        }
        let Some(definition) = program.bind_selected(controller, source, output) else {
            continue;
        };
        let ability = match apply(&mut state, Action::RegisterActivatedAbility { definition }) {
            Outcome::ActivatedAbilityRegistered(ability) => ability,
            _ => continue,
        };
        let Some(payment) = auto_payment_plan(ManaPool::empty(), program.cost().mana())
            .ok()
            .flatten()
        else {
            continue;
        };
        let before_life = state.players()[controller.index()].life();
        let outcome = apply(
            &mut state,
            Action::ActivateAbility {
                player: controller,
                ability,
                payment,
            },
        );
        let after_life = state.players()[controller.index()].life();
        let mana_matches = state.mana_pool(controller).ok() == Some(output);
        let life_matches = before_life.saturating_sub(after_life)
            == i32::try_from(program.damage_to_controller()).unwrap_or(i32::MAX);
        if matches!(outcome, Outcome::Applied) && mana_matches && life_matches {
            replayed_outputs.push(mana_label(output));
        }
    }
    json!({
        "legal_outputs": legal_outputs,
        "replayed_outputs": replayed_outputs,
        "damage_to_controller": program.damage_to_controller(),
        "all_outputs_replayed": replayed_outputs == legal_outputs,
        "minimum_matching_permanents": condition_minimum(program),
        "condition_rejected_below_threshold": rejects_below_activation_condition(
            program,
            salt.wrapping_add(80),
        ),
    })
}

fn token_mana_programs(program: &CardProgram) -> Vec<ActivatedAbilityProgram> {
    program
        .effects()
        .iter()
        .chain(
            program
                .activated_effects()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .chain(
            program
                .triggered_abilities()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .filter_map(|effect| match effect {
            EffectProgram::CreateTokens {
                mana_ability: Some(program),
                ..
            } => Some(*program),
            _ => None,
        })
        .collect()
}

fn token_subtype_sets(program: &CardProgram) -> Vec<Vec<String>> {
    program
        .effects()
        .iter()
        .chain(
            program
                .activated_effects()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .chain(
            program
                .triggered_abilities()
                .iter()
                .flat_map(|ability| ability.effects()),
        )
        .filter_map(|effect| match effect {
            EffectProgram::CreateTokens { base_object, .. } => Some(
                base_object
                    .subtypes()
                    .as_slice()
                    .iter()
                    .map(|subtype| String::from_utf8_lossy(subtype.as_bytes()).into_owned())
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn no_maximum_hand_size_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let abilities = program
        .static_abilities()
        .iter()
        .filter(|ability| {
            matches!(
                ability,
                StaticAbilityProgram::PlayerRule {
                    rule: PlayerRule::NoMaximumHandSize
                }
            )
        })
        .collect::<Vec<_>>();
    if abilities.is_empty() {
        return None;
    }

    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source = match apply(
        &mut state,
        Action::CreateObject {
            card: CardId::new(9_300_000),
            owner: controller,
            controller,
            zone: ZoneId::new(None, ZoneKind::Battlefield),
        },
    ) {
        Outcome::ObjectCreated(object) => object,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let registered = abilities
        .into_iter()
        .flat_map(|ability| ability.bind_actions(controller, source))
        .all(|action| matches!(apply(&mut state, action), Outcome::RestrictionRegistered(_)));
    let active_for_controller = state.effective_max_hand_size(controller).ok() == Some(None);
    let opponent_unaffected = state.effective_max_hand_size(opponent).ok() == Some(Some(7));
    let moved = matches!(
        apply(
            &mut state,
            Action::MoveObject {
                object: source,
                to: ZoneId::new(Some(controller), ZoneKind::Graveyard),
            },
        ),
        Outcome::Applied
    );
    let expired_off_battlefield = state.effective_max_hand_size(controller).ok() == Some(Some(7));
    Some(json!({
        "setup_succeeded": true,
        "registered": registered,
        "active_for_controller": active_for_controller,
        "opponent_unaffected": opponent_unaffected,
        "moved_source_to_graveyard": moved,
        "expired_off_battlefield": expired_off_battlefield,
    }))
}

fn create_probe_object(
    state: &mut GameState,
    card: u32,
    owner: forge_core::PlayerId,
    controller: forge_core::PlayerId,
    zone: ZoneId,
    base_object: BaseObjectCharacteristics,
    base_creature: Option<BaseCreatureCharacteristics>,
) -> Option<forge_core::ObjectId> {
    let object = match apply(
        state,
        Action::CreateObject {
            card: CardId::new(card),
            owner,
            controller,
            zone,
        },
    ) {
        Outcome::ObjectCreated(object) => object,
        _ => return None,
    };
    if !matches!(
        apply(
            state,
            Action::SetBaseObjectCharacteristics {
                object,
                base: base_object,
            },
        ),
        Outcome::Applied
    ) {
        return None;
    }
    if let Some(base) = base_creature {
        if !matches!(
            apply(
                state,
                Action::SetBaseCreatureCharacteristics { object, base },
            ),
            Outcome::Applied
        ) {
            return None;
        }
    }
    Some(object)
}

fn creature_probe(
    state: &GameState,
    object: forge_core::ObjectId,
    controller: forge_core::PlayerId,
    opponent: forge_core::PlayerId,
) -> Option<serde_json::Value> {
    let characteristics = state.creature_characteristics(object).ok()?;
    let requirement = [TargetRequirement::new(TargetKind::Permanent)];
    let target = [TargetChoice::Object(object)];
    Some(json!({
        "power": characteristics.power(),
        "toughness": characteristics.toughness(),
        "haste": characteristics.keywords().haste(),
        "controller_targetable": state
            .validate_target_choices(controller, None, &requirement, &target)
            .is_ok(),
        "opponent_targetable": state
            .validate_target_choices(opponent, None, &requirement, &target)
            .is_ok(),
    }))
}

fn bind_equip_actions(
    state: &GameState,
    ability: &ActivatedEffectProgram,
    controller: forge_core::PlayerId,
    opponent: forge_core::PlayerId,
    source: forge_core::ObjectId,
    target: forge_core::ObjectId,
) -> Option<Vec<Action>> {
    let bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_source(source)
        .with_targets(vec![TargetChoice::Object(target)]);
    bind_activated_effect_actions(state, ability, &bindings)
        .ok()
        .map(|actions| actions.iter().map(|bound| bound.action().clone()).collect())
}

fn pay_and_equip(
    state: &mut GameState,
    ability: &ActivatedEffectProgram,
    controller: forge_core::PlayerId,
    opponent: forge_core::PlayerId,
    source: forge_core::ObjectId,
    target: forge_core::ObjectId,
) -> (bool, bool, bool) {
    let Some(actions) = bind_equip_actions(state, ability, controller, opponent, source, target)
    else {
        return (false, false, false);
    };
    let source_bound_attach = actions.len() == 1
        && matches!(
            actions.first(),
            Some(Action::AttachObject {
                attachment,
                target: Some(bound_target),
            }) if *attachment == source && *bound_target == target
        );
    let cleared = matches!(
        apply(state, Action::ClearManaPool { player: controller }),
        Outcome::Applied
    );
    let exact_payment = ability.exact_payment();
    let funded = exact_payment == ManaPool::empty()
        || matches!(
            apply(
                state,
                Action::AddManaToPool {
                    player: controller,
                    mana: exact_payment,
                },
            ),
            Outcome::Applied
        );
    let paid = auto_payment_plan(exact_payment, ability.mana_cost())
        .ok()
        .flatten()
        .is_some_and(|plan| {
            matches!(
                apply(
                    state,
                    Action::PayMana {
                        player: controller,
                        cost: ability.mana_cost(),
                        plan,
                    },
                ),
                Outcome::Applied
            )
        });
    let payment_consumed =
        cleared && funded && paid && state.mana_pool(controller).ok() == Some(ManaPool::empty());
    let actions_applied = actions
        .into_iter()
        .all(|action| matches!(apply(state, action), Outcome::Applied));
    let attached = state.object(source).and_then(|record| record.attached_to()) == Some(target);
    (
        source_bound_attach,
        payment_consumed,
        actions_applied && attached,
    )
}

fn attached_attack_trigger_probe(
    state: &mut GameState,
    program: &CardProgram,
    controller: forge_core::PlayerId,
    opponent: forge_core::PlayerId,
    source: forge_core::ObjectId,
) -> Option<serde_json::Value> {
    let abilities = program
        .triggered_abilities()
        .iter()
        .filter(|ability| ability.event() == TriggeredEventProgram::AttachedObjectAttacks)
        .collect::<Vec<_>>();
    if abilities.is_empty() {
        return None;
    }
    let ability = abilities[0];
    let definition = ability.bind(controller, source);
    let source_bound_condition = definition.source() == Some(source)
        && matches!(
            definition.condition(),
            TriggerCondition::AttackDeclared {
                attacker: TriggerObjectFilter::AttachedToSource,
            }
        );
    let registered = matches!(
        apply(state, Action::RegisterTriggeredAbility { definition },),
        Outcome::TriggerRegistered(_)
    );

    let basic_land = create_probe_object(
        state,
        9_400_100,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Library),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_land(), ObjectColors::none())
            .with_supertypes(ObjectSupertypes::none().with_basic()),
        None,
    )?;
    let nonbasic_land = create_probe_object(
        state,
        9_400_101,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Library),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_land(), ObjectColors::none()),
        None,
    )?;
    let invalid_bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_source(source)
        .with_object_choices(vec![vec![nonbasic_land]])
        .with_optional_effect_choices(vec![true]);
    let nonbasic_choice_rejected =
        bind_triggered_ability_actions(state, ability, &invalid_bindings).is_err();
    let valid_bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_source(source)
        .with_object_choices(vec![vec![basic_land]])
        .with_optional_effect_choices(vec![true]);
    let actions = bind_triggered_ability_actions(state, ability, &valid_bindings).ok()?;
    let move_action_present = actions.iter().any(|bound| {
        matches!(
            bound.action(),
            Action::MoveObject { object, to }
                if *object == basic_land
                    && *to == ZoneId::new(None, ZoneKind::Battlefield)
        )
    });
    let tap_action_present = actions.iter().any(|bound| {
        matches!(
            bound.action(),
            Action::SetObjectTapped { object, tapped: true } if *object == basic_land
        )
    });
    let shuffle_action_present = actions.iter().any(|bound| {
        matches!(
            bound.action(),
            Action::ShuffleLibrary { player } if *player == controller
        )
    });
    let action_count = actions.len();
    let all_actions_applied = actions
        .iter()
        .all(|bound| matches!(apply(state, bound.action().clone()), Outcome::Applied));

    Some(json!({
        "count": abilities.len(),
        "source_bound_condition": source_bound_condition,
        "registered": registered,
        "choice_slots": ability.object_choice_requirements().len(),
        "optional_choices": ability.optional_choice_count(),
        "nonbasic_choice_rejected": nonbasic_choice_rejected,
        "basic_choice_bound": true,
        "bound_action_count": action_count,
        "move_action_present": move_action_present,
        "tap_action_present": tap_action_present,
        "shuffle_action_present": shuffle_action_present,
        "all_actions_applied": all_actions_applied,
        "basic_land_moved_to_battlefield": state.object_zone(basic_land)
            == Some(ZoneId::new(None, ZoneKind::Battlefield)),
        "basic_land_tapped": state.object(basic_land).is_some_and(|record| record.tapped()),
        "nonbasic_land_remained_in_library": state.object_zone(nonbasic_land)
            == Some(ZoneId::new(Some(controller), ZoneKind::Library)),
    }))
}

fn equipment_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let equip_abilities = program
        .activated_effects()
        .iter()
        .filter(|ability| {
            ability
                .effects()
                .iter()
                .any(|effect| matches!(effect, EffectProgram::AttachSourceToTarget { .. }))
        })
        .collect::<Vec<_>>();
    if equip_abilities.is_empty() {
        return None;
    }
    let ability = equip_abilities[0];
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let source = create_probe_object(
        &mut state,
        9_400_000,
        controller,
        controller,
        battlefield,
        program.base_object(),
        None,
    )?;
    let creature_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none());
    let creature_stats = Some(BaseCreatureCharacteristics::new(2, 2));
    let first = create_probe_object(
        &mut state,
        9_400_001,
        controller,
        controller,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    let second = create_probe_object(
        &mut state,
        9_400_002,
        controller,
        controller,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    let opponent_creature = create_probe_object(
        &mut state,
        9_400_003,
        opponent,
        opponent,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    let noncreature = create_probe_object(
        &mut state,
        9_400_004,
        controller,
        controller,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;

    let controlled_creature_target_bound =
        bind_equip_actions(&state, ability, controller, opponent, source, first).is_some();
    let opponent_creature_target_rejected = bind_equip_actions(
        &state,
        ability,
        controller,
        opponent,
        source,
        opponent_creature,
    )
    .is_none();
    let noncreature_target_rejected =
        bind_equip_actions(&state, ability, controller, opponent, source, noncreature).is_none();

    let mut static_registration_count = 0_usize;
    let mut static_registered = true;
    for static_ability in program.static_abilities() {
        for action in static_ability.bind_actions(controller, source) {
            static_registration_count = static_registration_count.saturating_add(1);
            static_registered &= matches!(
                apply(&mut state, action),
                Outcome::ContinuousEffectRegistered(_) | Outcome::RestrictionRegistered(_)
            );
        }
    }

    let (first_source_bound, first_payment, attached_to_first) =
        pay_and_equip(&mut state, ability, controller, opponent, source, first);
    let first_attachment = creature_probe(&state, first, controller, opponent)?;
    let (second_source_bound, second_payment, attached_to_second) =
        pay_and_equip(&mut state, ability, controller, opponent, source, second);
    let first_after_reattachment = creature_probe(&state, first, controller, opponent)?;
    let second_after_reattachment = creature_probe(&state, second, controller, opponent)?;
    let attack_trigger =
        attached_attack_trigger_probe(&mut state, program, controller, opponent, source);
    let source_moved_to_graveyard = matches!(
        apply(
            &mut state,
            Action::MoveObject {
                object: source,
                to: ZoneId::new(Some(controller), ZoneKind::Graveyard),
            },
        ),
        Outcome::Applied
    );
    let second_after_expiration = creature_probe(&state, second, controller, opponent)?;

    Some(json!({
        "setup_succeeded": true,
        "equip_ability_count": equip_abilities.len(),
        "generic_mana_cost": ability.mana_cost().generic_total().ok(),
        "colored_mana_cost": ability.mana_cost().colored_pool().total(),
        "exact_payment_total": ability.exact_payment().total(),
        "timing": match ability.timing() {
            ActivationTiming::Instant => "instant",
            ActivationTiming::Sorcery => "sorcery",
        },
        "target_slots": ability.target_requirements().len(),
        "optional_choices": ability.optional_choice_count(),
        "static_registration_count": static_registration_count,
        "static_registered": static_registered,
        "controlled_creature_target_bound": controlled_creature_target_bound,
        "opponent_creature_target_rejected": opponent_creature_target_rejected,
        "noncreature_target_rejected": noncreature_target_rejected,
        "source_bound_attach_actions": first_source_bound && second_source_bound,
        "payments_consumed": first_payment && second_payment,
        "attached_to_first": attached_to_first,
        "first_attachment": first_attachment,
        "attached_to_second": attached_to_second,
        "first_after_reattachment": first_after_reattachment,
        "second_after_reattachment": second_after_reattachment,
        "attached_attack_trigger": attack_trigger,
        "source_moved_to_graveyard": source_moved_to_graveyard,
        "second_after_expiration": second_after_expiration,
    }))
}

fn matches_sacrifice_predicate(
    state: &GameState,
    controller: forge_core::PlayerId,
    predicate: ObjectTargetPredicate,
    object: forge_core::ObjectId,
) -> bool {
    let requirement =
        [TargetRequirement::new(TargetKind::Permanent).with_object_predicate(predicate)];
    state
        .validate_target_choices(
            controller,
            None,
            &requirement,
            &[TargetChoice::Object(object)],
        )
        .is_ok()
}

fn advance_to_declare_attackers(state: &mut GameState, active: forge_core::PlayerId) -> bool {
    if !matches!(
        apply(
            state,
            Action::CreateObject {
                card: CardId::new(9_500_100),
                owner: active,
                controller: active,
                zone: ZoneId::new(Some(active), ZoneKind::Library),
            },
        ),
        Outcome::ObjectCreated(_)
    ) || !matches!(
        apply(
            state,
            Action::StartTurn {
                active_player: active
            }
        ),
        Outcome::Applied
    ) {
        return false;
    }
    for _ in 0..12 {
        if state.current_step() == Some(Step::DeclareAttackers) {
            return true;
        }
        if !matches!(apply(state, Action::AdvanceStep), Outcome::StepAdvanced(_)) {
            return false;
        }
    }
    false
}

fn sacrifice_counter_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let abilities = program
        .activated_effects()
        .iter()
        .filter(|ability| {
            ability.sacrifice_cost().is_some()
                && ability
                    .effects()
                    .iter()
                    .any(|effect| matches!(effect, EffectProgram::AddCountersToSource { .. }))
        })
        .collect::<Vec<_>>();
    if abilities.is_empty() {
        return None;
    }
    let ability = abilities[0];
    let (predicate, sacrifice_count) = ability.sacrifice_cost()?;
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let source = create_probe_object(
        &mut state,
        9_500_000,
        controller,
        controller,
        battlefield,
        program.base_object(),
        program.base_creature(),
    )?;
    let creature_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none());
    let creature_stats = Some(BaseCreatureCharacteristics::new(1, 1));
    let fodder = create_probe_object(
        &mut state,
        9_500_001,
        controller,
        controller,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    let opponent_creature = create_probe_object(
        &mut state,
        9_500_002,
        opponent,
        opponent,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    let noncreature = create_probe_object(
        &mut state,
        9_500_003,
        controller,
        controller,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let attacker = create_probe_object(
        &mut state,
        9_500_004,
        opponent,
        opponent,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;

    let fodder_matches = matches_sacrifice_predicate(&state, controller, predicate, fodder);
    let opponent_creature_rejected =
        !matches_sacrifice_predicate(&state, controller, predicate, opponent_creature);
    let noncreature_rejected =
        !matches_sacrifice_predicate(&state, controller, predicate, noncreature);
    let fodder_sacrificed = matches!(
        apply(
            &mut state,
            Action::MoveObject {
                object: fodder,
                to: ZoneId::new(Some(controller), ZoneKind::Graveyard),
            },
        ),
        Outcome::Applied
    ) && state.object_zone(fodder)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));
    let bindings = ExecutionBindings::new(controller, vec![opponent]).with_source(source);
    let actions = bind_activated_effect_actions(&state, ability, &bindings).ok()?;
    let source_bound_counter_action = actions.len() == 1
        && matches!(
            actions.first().map(|bound| bound.action()),
            Some(Action::AddObjectCounters {
                object,
                kind: CounterKind::PlusOnePlusOne,
                amount: 1,
            }) if *object == source
        );
    let counter_action_applied = actions
        .iter()
        .all(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let source_characteristics = state.creature_characteristics(source).ok()?;
    let source_remained_battlefield =
        state.object_zone(source) == Some(ZoneId::new(None, ZoneKind::Battlefield));

    let combat_setup_succeeded = advance_to_declare_attackers(&mut state, opponent)
        && matches!(
            apply(
                &mut state,
                Action::DeclareAttackers {
                    player: opponent,
                    attacks: vec![AttackDeclaration::new(attacker, controller)],
                },
            ),
            Outcome::Applied
        )
        && matches!(
            apply(&mut state, Action::AdvanceStep),
            Outcome::StepAdvanced(Step::DeclareBlockers)
        );
    let block = BlockDeclaration::new(source, attacker);
    let could_block_before_restriction =
        combat_setup_succeeded && state.can_block(controller, block);
    let mut static_registration_count = 0_usize;
    let mut restriction_registered = true;
    for static_ability in program.static_abilities() {
        for action in static_ability.bind_actions(controller, source) {
            static_registration_count = static_registration_count.saturating_add(1);
            restriction_registered &=
                matches!(apply(&mut state, action), Outcome::RestrictionRegistered(_));
        }
    }
    let source_bound_cannot_block_definition = state.restrictions().any(|(_, definition)| {
        definition.source() == Some(source)
            && matches!(
                definition.effect(),
                RestrictionEffect::Combat {
                    subject: CombatRestrictionSubject::Object(object),
                    restriction: CombatRestriction::CannotBlock,
                } if object == source
            )
    });
    let can_block_after_restriction = state.can_block(controller, block);

    Some(json!({
        "setup_succeeded": true,
        "ability_count": abilities.len(),
        "generic_mana_cost": ability.mana_cost().generic_total().ok(),
        "colored_mana_cost": ability.mana_cost().colored_pool().total(),
        "timing": match ability.timing() {
            ActivationTiming::Instant => "instant",
            ActivationTiming::Sorcery => "sorcery",
        },
        "target_slots": ability.target_requirements().len(),
        "object_choice_slots": ability.object_choice_requirements().len(),
        "optional_choices": ability.optional_choice_count(),
        "sacrifice_count": sacrifice_count,
        "sacrifice_requires_creature": predicate.required_types().creature(),
        "sacrifice_requires_controller":
            predicate.controller() == TargetControllerPredicate::You,
        "fodder_matches": fodder_matches,
        "opponent_creature_rejected": opponent_creature_rejected,
        "noncreature_rejected": noncreature_rejected,
        "fodder_sacrificed": fodder_sacrificed,
        "source_remained_battlefield": source_remained_battlefield,
        "source_bound_counter_action": source_bound_counter_action,
        "counter_action_applied": counter_action_applied,
        "plus_one_counters": state.object_counter_count(source, CounterKind::PlusOnePlusOne),
        "power_after_counter": source_characteristics.power(),
        "toughness_after_counter": source_characteristics.toughness(),
        "combat_setup_succeeded": combat_setup_succeeded,
        "could_block_before_restriction": could_block_before_restriction,
        "static_registration_count": static_registration_count,
        "restriction_registered": restriction_registered,
        "source_bound_cannot_block_definition": source_bound_cannot_block_definition,
        "can_block_after_restriction": can_block_after_restriction,
    }))
}

fn semantic_probe(program: &CardProgram) -> serde_json::Value {
    let base_subtypes = program
        .base_object()
        .subtypes()
        .as_slice()
        .iter()
        .map(|subtype| String::from_utf8_lossy(subtype.as_bytes()).into_owned())
        .collect::<Vec<_>>();
    let mana_abilities = program
        .activated_abilities()
        .iter()
        .copied()
        .enumerate()
        .map(|(index, ability)| replay_mana_ability(ability, 9_100_000 + index as u32 * 100))
        .collect::<Vec<_>>();
    let token_mana_abilities = token_mana_programs(program)
        .into_iter()
        .enumerate()
        .map(|(index, ability)| replay_mana_ability(ability, 9_200_000 + index as u32 * 100))
        .collect::<Vec<_>>();
    json!({
        "base_subtypes": base_subtypes,
        "mana_abilities": mana_abilities,
        "token_mana_abilities": token_mana_abilities,
        "token_subtypes": token_subtype_sets(program),
        "no_maximum_hand_size": no_maximum_hand_size_probe(program),
        "equipment": equipment_probe(program),
        "sacrifice_counter": sacrifice_counter_probe(program),
    })
}

fn main() -> ExitCode {
    let mut failed = false;
    for path in env::args().skip(1) {
        let entry = match fs::read_to_string(&path) {
            Ok(source) => match forge_cardc::parse_card_named(&path, &source) {
                Ok(definition) => {
                    let semantic_probe = compile_card_program(&definition)
                        .ok()
                        .map(|program| semantic_probe(&program));
                    let report = run_translated_card_runtime_smoke(&definition);
                    match report.result() {
                        RuntimeSmokeResult::Passed(pass) => json!({
                            "path": path,
                            "oracle_id": report.oracle_id(),
                            "card_name": report.card_name(),
                            "disposition": "passed",
                            "capabilities": pass
                                .capabilities()
                                .iter()
                                .map(|capability| capability.as_str())
                                .collect::<Vec<_>>(),
                            "effect_actions": pass.effect_actions(),
                            "production_actions": pass.production_actions(),
                            "final_life_totals": pass.final_life_totals(),
                            "destination": pass.destination(),
                            "final_hash": pass.final_hash().to_string(),
                            "semantic_probe": semantic_probe,
                        }),
                        RuntimeSmokeResult::UnsupportedSetup(result) => json!({
                            "path": path,
                            "oracle_id": report.oracle_id(),
                            "card_name": report.card_name(),
                            "disposition": "unsupported_setup",
                            "code": result.code().as_str(),
                            "detail": result.detail(),
                            "semantic_probe": semantic_probe,
                        }),
                        RuntimeSmokeResult::Failed(result) => {
                            failed = true;
                            json!({
                                "path": path,
                                "oracle_id": report.oracle_id(),
                                "card_name": report.card_name(),
                                "disposition": "failed",
                                "code": result.code().as_str(),
                                "phase": result.phase(),
                                "detail": result.detail(),
                            })
                        }
                    }
                }
                Err(error) => {
                    failed = true;
                    json!({
                        "path": path,
                        "disposition": "compiler_invalid",
                        "detail": error.to_string(),
                    })
                }
            },
            Err(error) => {
                failed = true;
                json!({
                    "path": path,
                    "disposition": "read_error",
                    "detail": error.to_string(),
                })
            }
        };
        match serde_json::to_string(&entry) {
            Ok(line) => println!("{line}"),
            Err(error) => {
                eprintln!("could not serialize runtime probe entry: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

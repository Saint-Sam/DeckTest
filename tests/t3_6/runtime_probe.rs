#![forbid(unsafe_code)]

use forge_cards::runtime::{
    bind_activated_effect_actions, bind_program_actions, bind_triggered_ability_actions,
    compile_card_program, execute_program, ActivatedAbilityProgram, ActivatedEffectProgram,
    AlternateCostCondition, CardProgram, EffectProgram, ExecutionBindings, ExecutionDiagnosticCode,
    PlayerBinding, StaticAbilityProgram, TriggeredEventProgram,
};
use forge_core::{
    apply, auto_payment_plan, AbilityPlayer, Action, ActivatedAbilityDefinition,
    ActivatedAbilityEffect, ActivationCondition, ActivationCost, ActivationTiming,
    AttackDeclaration, BaseCreatureCharacteristics, BaseObjectCharacteristics, BlockDeclaration,
    CardId, CastSpellRequest, CombatRestriction, CombatRestrictionSubject,
    ContinuousEffectDuration, CounterKind, GameState, ManaCost, ManaKind, ManaPool, ObjectColors,
    ObjectSupertypes, ObjectTargetPredicate, ObjectTypes, Outcome, PlayerRule, PriorityOutcome,
    ResolutionOutcome, RestrictionEffect, SpellTiming, StackObjectKind, StateError, Step,
    TargetChoice, TargetControllerPredicate, TargetKind, TargetRequirement, TargetRestriction,
    TargetRestrictionSubject, TriggerCondition, TriggerObjectFilter, ZoneId, ZoneKind,
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

fn object_targetable(
    state: &GameState,
    player: forge_core::PlayerId,
    object: forge_core::ObjectId,
) -> bool {
    state
        .validate_target_choices(
            player,
            None,
            &[TargetRequirement::new(TargetKind::Permanent)],
            &[TargetChoice::Object(object)],
        )
        .is_ok()
}

fn has_hexproof_restriction(state: &GameState, object: forge_core::ObjectId) -> bool {
    state.restrictions().any(|(_, definition)| {
        definition.duration() == ContinuousEffectDuration::UntilEndOfTurn
            && matches!(
                definition.effect(),
                RestrictionEffect::Targeting {
                    subject: TargetRestrictionSubject::Object(candidate),
                    restriction: TargetRestriction::Hexproof,
                } if candidate == object
            )
    })
}

fn has_indestructible_restriction(state: &GameState, object: forge_core::ObjectId) -> bool {
    state.restrictions().any(|(_, definition)| {
        definition.duration() == ContinuousEffectDuration::UntilEndOfTurn
            && matches!(
                definition.effect(),
                RestrictionEffect::Indestructible { object: candidate } if candidate == object
            )
    })
}

fn advance_to_cleanup(state: &mut GameState, active: forge_core::PlayerId) -> bool {
    if !matches!(
        apply(
            state,
            Action::CreateObject {
                card: CardId::new(9_600_100),
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
                active_player: active,
            },
        ),
        Outcome::Applied
    ) {
        return false;
    }
    for _ in 0..20 {
        if state.current_step() == Some(Step::Cleanup) {
            return true;
        }
        if !matches!(apply(state, Action::AdvanceStep), Outcome::StepAdvanced(_)) {
            return false;
        }
    }
    false
}

fn commander_alternate_cost_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let alternate = program
        .alternate_costs()
        .iter()
        .copied()
        .find(|cost| cost.condition() == AlternateCostCondition::ControllerControlsCommander)?;
    let alternate_cost_count = program
        .alternate_costs()
        .iter()
        .filter(|cost| cost.condition() == AlternateCostCondition::ControllerControlsCommander)
        .count();
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
    let creature_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none());
    let opponent_commander = create_probe_object(
        &mut state,
        9_610_000,
        opponent,
        opponent,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let opponent_designated = matches!(
        apply(
            &mut state,
            Action::DesignateCommander {
                object: opponent_commander,
                color_identity: ObjectColors::none(),
            },
        ),
        Outcome::Applied
    );
    let available_without_controlled_battlefield_commander =
        alternate.is_available(&state, controller, None);
    let opponent_commander_does_not_enable =
        opponent_designated && !alternate.is_available(&state, controller, None);

    let controlled_creature = create_probe_object(
        &mut state,
        9_610_001,
        controller,
        controller,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let undesignated_controlled_creature_does_not_enable =
        !alternate.is_available(&state, controller, None);
    let controlled_designated = matches!(
        apply(
            &mut state,
            Action::DesignateCommander {
                object: controlled_creature,
                color_identity: ObjectColors::none(),
            },
        ),
        Outcome::Applied
    );
    let available_with_controlled_battlefield_commander =
        controlled_designated && alternate.is_available(&state, controller, None);
    let moved_to_command_zone = matches!(
        apply(
            &mut state,
            Action::MoveObject {
                object: controlled_creature,
                to: ZoneId::new(None, ZoneKind::Command),
            },
        ),
        Outcome::Applied
    );
    let unavailable_in_command_zone =
        moved_to_command_zone && !alternate.is_available(&state, controller, None);
    let returned_to_battlefield = matches!(
        apply(
            &mut state,
            Action::MoveObject {
                object: controlled_creature,
                to: battlefield,
            },
        ),
        Outcome::Applied
    );
    let available_after_return_to_battlefield =
        returned_to_battlefield && alternate.is_available(&state, controller, None);
    let zero_payment_plan_available =
        auto_payment_plan(alternate.exact_payment(), alternate.mana_cost())
            .ok()
            .flatten()
            .is_some();

    Some(json!({
        "setup_succeeded": true,
        "alternate_cost_count": alternate_cost_count,
        "condition_is_controller_controls_commander": matches!(
            alternate.condition(),
            AlternateCostCondition::ControllerControlsCommander
        ),
        "printed_generic_mana": program.mana_cost().generic_total().ok(),
        "printed_colored_mana": program.mana_cost().colored_pool().total(),
        "alternate_generic_mana": alternate.mana_cost().generic_total().ok(),
        "alternate_colored_mana": alternate.mana_cost().colored_pool().total(),
        "exact_payment_total": alternate.exact_payment().total(),
        "zero_payment_plan_available": zero_payment_plan_available,
        "available_without_controlled_battlefield_commander":
            available_without_controlled_battlefield_commander,
        "opponent_commander_does_not_enable": opponent_commander_does_not_enable,
        "undesignated_controlled_creature_does_not_enable":
            undesignated_controlled_creature_does_not_enable,
        "available_with_controlled_battlefield_commander":
            available_with_controlled_battlefield_commander,
        "unavailable_in_command_zone": unavailable_in_command_zone,
        "available_after_return_to_battlefield": available_after_return_to_battlefield,
    }))
}

fn pass_empty_stack_round(state: &mut GameState) -> bool {
    for _ in 0..state.players().len() {
        let Some(player) = state.priority_player() else {
            return false;
        };
        match apply(state, Action::PassPriority { player }) {
            Outcome::Priority(PriorityOutcome::StepComplete) => return true,
            Outcome::Priority(PriorityOutcome::PassedTo(_)) => {}
            _ => return false,
        }
    }
    false
}

fn advance_to_precombat_main(state: &mut GameState, active: forge_core::PlayerId) -> bool {
    matches!(
        apply(
            state,
            Action::StartTurn {
                active_player: active,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(state, Action::AdvanceStep),
        Outcome::StepAdvanced(Step::Upkeep)
    ) && pass_empty_stack_round(state)
        && state.current_step() == Some(Step::Draw)
        && pass_empty_stack_round(state)
        && state.current_step() == Some(Step::PrecombatMain)
}

fn resolve_expected_stack_entry(state: &mut GameState, expected: forge_core::StackEntryId) -> bool {
    for _ in 0..state.players().len() {
        let Some(player) = state.priority_player() else {
            return false;
        };
        match apply(state, Action::PassPriority { player }) {
            Outcome::Priority(PriorityOutcome::PassedTo(_)) => {}
            Outcome::Priority(PriorityOutcome::Resolved(entry)) if entry == expected => {
                return true;
            }
            _ => return false,
        }
    }
    false
}

fn invalid_choice_rejected_without_mutation(
    state: &mut GameState,
    program: &CardProgram,
    controller: forge_core::PlayerId,
    opponent: forge_core::PlayerId,
    choices: Vec<forge_core::ObjectId>,
) -> bool {
    let before = state.deterministic_hash();
    let result = execute_program(
        state,
        program,
        &ExecutionBindings::new(controller, vec![opponent]).with_object_choices(vec![choices]),
    );
    matches!(
        result,
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && state.deterministic_hash() == before
}

fn flashback_looting_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let alternate = program
        .alternate_costs()
        .iter()
        .copied()
        .find(|cost| cost.condition() == AlternateCostCondition::SourceInControllerGraveyard)?;
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let hand = ZoneId::new(Some(controller), ZoneKind::Hand);
    let graveyard = ZoneId::new(Some(controller), ZoneKind::Graveyard);
    let source = create_probe_object(
        &mut state,
        9_640_000,
        controller,
        controller,
        hand,
        program.base_object(),
        program.base_creature(),
    )?;
    let unavailable_from_hand = !alternate.is_available(&state, controller, Some(source));
    let source_moved_to_graveyard = matches!(
        apply(
            &mut state,
            Action::MoveObject {
                object: source,
                to: graveyard,
            },
        ),
        Outcome::Applied
    );
    let available_from_graveyard =
        source_moved_to_graveyard && alternate.is_available(&state, controller, Some(source));

    let card_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none());
    let first_choice = create_probe_object(
        &mut state, 9_640_001, controller, controller, hand, card_base, None,
    )?;
    let second_choice = create_probe_object(
        &mut state, 9_640_002, controller, controller, hand, card_base, None,
    )?;
    let retained_card = create_probe_object(
        &mut state, 9_640_003, controller, controller, hand, card_base, None,
    )?;
    let out_of_zone = create_probe_object(
        &mut state, 9_640_004, controller, controller, graveyard, card_base, None,
    )?;
    for card in 9_640_010..9_640_014 {
        create_probe_object(
            &mut state,
            card,
            controller,
            controller,
            ZoneId::new(Some(controller), ZoneKind::Library),
            card_base,
            None,
        )?;
    }
    let cast_window_ready = advance_to_precombat_main(&mut state, controller);

    let mana_added = matches!(
        apply(
            &mut state,
            Action::AddManaToPool {
                player: controller,
                mana: alternate.exact_payment(),
            },
        ),
        Outcome::Applied
    );
    let payment = auto_payment_plan(alternate.exact_payment(), alternate.mana_cost())
        .ok()
        .flatten()?;
    let cast = apply(
        &mut state,
        Action::CastSpell {
            player: controller,
            object: source,
            request: CastSpellRequest::new(
                StackObjectKind::SorcerySpell,
                SpellTiming::Sorcery,
                alternate.mana_cost(),
                payment,
            )
            .with_flashback(alternate.mana_cost()),
        },
    );
    let stack_entry = match cast {
        Outcome::StackEntryAdded(entry) => entry,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source_on_stack = state.object_zone(source) == Some(ZoneId::new(None, ZoneKind::Stack));
    let stack_entry_marked_flashback = state
        .stack_entries()
        .iter()
        .find(|entry| entry.id() == stack_entry)
        .is_some_and(|entry| entry.flashback());
    let flashback_cost_consumed =
        mana_added && state.mana_pool(controller).ok() == Some(ManaPool::empty());
    let stack_resolved = resolve_expected_stack_entry(&mut state, stack_entry);
    let source_exiled_on_resolution =
        state.object_zone(source) == Some(ZoneId::new(None, ZoneKind::Exile));
    let resolution_recorded = state.resolution_log().last().is_some_and(|record| {
        record.stack_entry() == stack_entry
            && record.object() == Some(source)
            && record.outcome() == ResolutionOutcome::Resolved
            && record.flashback()
    });

    let requirements = program.object_choice_requirements();
    let choice_slot_count = requirements.len();
    let requirement = *requirements.first()?;
    let undersized_choice_rejected_before_mutation = invalid_choice_rejected_without_mutation(
        &mut state,
        program,
        controller,
        opponent,
        vec![first_choice],
    );
    let duplicate_choice_rejected_before_mutation = invalid_choice_rejected_without_mutation(
        &mut state,
        program,
        controller,
        opponent,
        vec![first_choice, first_choice],
    );
    let out_of_zone_choice_rejected_before_mutation = invalid_choice_rejected_without_mutation(
        &mut state,
        program,
        controller,
        opponent,
        vec![first_choice, out_of_zone],
    );

    let bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_object_choices(vec![vec![first_choice, second_choice]]);
    let actions = bind_program_actions(&state, program, &bindings).ok()?;
    let bound_action_count = actions.len();
    let draw_action_exact = matches!(
        actions.first().map(|bound| bound.action()),
        Some(Action::DrawCards { player, count: 2 }) if *player == controller
    );
    let discard_actions_exact = matches!(
        actions.get(1).map(|bound| bound.action()),
        Some(Action::MoveObject { object, to })
            if *object == first_choice && *to == graveyard
    ) && matches!(
        actions.get(2).map(|bound| bound.action()),
        Some(Action::MoveObject { object, to })
            if *object == second_choice && *to == graveyard
    );
    let hand_before_effect = state
        .zone_objects(hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let library = ZoneId::new(Some(controller), ZoneKind::Library);
    let library_before_effect = state
        .zone_objects(library)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let draw_applied = actions
        .first()
        .is_some_and(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let hand_after_draw = state
        .zone_objects(hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let library_after_draw = state
        .zone_objects(library)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let discard_actions_applied = actions
        .iter()
        .skip(1)
        .all(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let final_hand_size = state
        .zone_objects(hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let exactly_two_cards_drawn = draw_applied
        && hand_after_draw == hand_before_effect.saturating_add(2)
        && library_after_draw.saturating_add(2) == library_before_effect;
    let exactly_two_explicit_choices_discarded = discard_actions_applied
        && final_hand_size == hand_before_effect
        && state.object_zone(first_choice) == Some(graveyard)
        && state.object_zone(second_choice) == Some(graveyard);
    let retained_card_remained_in_hand = state.object_zone(retained_card) == Some(hand);
    let out_of_zone_card_unchanged = state.object_zone(out_of_zone) == Some(graveyard);

    Some(json!({
        "setup_succeeded": true,
        "alternate_cost_count": program.alternate_costs().len(),
        "condition_is_source_in_controller_graveyard": matches!(
            alternate.condition(),
            AlternateCostCondition::SourceInControllerGraveyard
        ),
        "printed_generic_mana": program.mana_cost().generic_total().ok(),
        "printed_red_mana": program.mana_cost().colored_pool().get(ManaKind::Red),
        "flashback_generic_mana": alternate.mana_cost().generic_total().ok(),
        "flashback_red_mana": alternate.mana_cost().colored_pool().get(ManaKind::Red),
        "flashback_exact_payment_total": alternate.exact_payment().total(),
        "unavailable_from_hand": unavailable_from_hand,
        "available_from_graveyard": available_from_graveyard,
        "cast_window_ready": cast_window_ready,
        "source_on_stack": source_on_stack,
        "stack_entry_marked_flashback": stack_entry_marked_flashback,
        "flashback_cost_consumed": flashback_cost_consumed,
        "stack_resolved": stack_resolved,
        "source_exiled_on_resolution": source_exiled_on_resolution,
        "resolution_recorded": resolution_recorded,
        "choice_slot_count": choice_slot_count,
        "choice_player_is_controller": requirement.player() == PlayerBinding::Controller,
        "choice_zone_is_hand": requirement.zone() == ZoneKind::Hand,
        "choice_minimum": requirement.minimum(),
        "choice_maximum": requirement.maximum(),
        "undersized_choice_rejected_before_mutation":
            undersized_choice_rejected_before_mutation,
        "duplicate_choice_rejected_before_mutation":
            duplicate_choice_rejected_before_mutation,
        "out_of_zone_choice_rejected_before_mutation":
            out_of_zone_choice_rejected_before_mutation,
        "bound_action_count": bound_action_count,
        "draw_action_exact": draw_action_exact,
        "discard_actions_exact": discard_actions_exact,
        "exactly_two_cards_drawn": exactly_two_cards_drawn,
        "exactly_two_explicit_choices_discarded": exactly_two_explicit_choices_discarded,
        "retained_card_remained_in_hand": retained_card_remained_in_hand,
        "out_of_zone_card_unchanged": out_of_zone_card_unchanged,
    }))
}

fn prepare_stack_priority(state: &mut GameState, active: forge_core::PlayerId) -> bool {
    matches!(
        apply(
            state,
            Action::CreateObject {
                card: CardId::new(9_620_100),
                owner: active,
                controller: active,
                zone: ZoneId::new(Some(active), ZoneKind::Library),
            },
        ),
        Outcome::ObjectCreated(_)
    ) && matches!(
        apply(
            state,
            Action::StartTurn {
                active_player: active,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(state, Action::AdvanceStep),
        Outcome::StepAdvanced(Step::Upkeep)
    )
}

fn split_second_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if !program.split_second() {
        return None;
    }
    let requirement = *program.target_requirements().first()?;
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let controller_hand = ZoneId::new(Some(controller), ZoneKind::Hand);
    let opponent_hand = ZoneId::new(Some(opponent), ZoneKind::Hand);
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let controller_graveyard = ZoneId::new(Some(controller), ZoneKind::Graveyard);
    let source = create_probe_object(
        &mut state,
        9_650_000,
        controller,
        controller,
        controller_hand,
        program.base_object(),
        program.base_creature(),
    )?;
    let artifact = create_probe_object(
        &mut state,
        9_650_001,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let enchantment = create_probe_object(
        &mut state,
        9_650_002,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(
            ObjectTypes::none().with_enchantment(),
            ObjectColors::none(),
        ),
        None,
    )?;
    let creature = create_probe_object(
        &mut state,
        9_650_003,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none()),
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let blocked_spell = create_probe_object(
        &mut state,
        9_650_004,
        opponent,
        opponent,
        opponent_hand,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_instant(), ObjectColors::none()),
        None,
    )?;
    let follow_up_spell = create_probe_object(
        &mut state,
        9_650_005,
        controller,
        controller,
        controller_hand,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_instant(), ObjectColors::none()),
        None,
    )?;
    let non_mana_source = create_probe_object(
        &mut state,
        9_650_006,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let mana_source = create_probe_object(
        &mut state,
        9_650_007,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;

    let zero_cost = ManaCost::new(0, 0, 0, 0, 0, 0);
    let zero_payment = auto_payment_plan(ManaPool::empty(), zero_cost)
        .ok()
        .flatten()?;
    let non_mana_ability = match apply(
        &mut state,
        Action::RegisterActivatedAbility {
            definition: ActivatedAbilityDefinition::new(
                opponent,
                Some(non_mana_source),
                ActivationTiming::Instant,
                ActivationCost::new(zero_cost),
                ActivatedAbilityEffect::GainLife {
                    player: AbilityPlayer::Controller,
                    amount: 1,
                },
            ),
        },
    ) {
        Outcome::ActivatedAbilityRegistered(ability) => ability,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let mana_ability = match apply(
        &mut state,
        Action::RegisterActivatedAbility {
            definition: ActivatedAbilityDefinition::new(
                opponent,
                Some(mana_source),
                ActivationTiming::Instant,
                ActivationCost::new(zero_cost).with_tap_source(),
                ActivatedAbilityEffect::AddMana {
                    player: AbilityPlayer::Controller,
                    mana: ManaPool::of(ManaKind::Green, 1),
                },
            )
            .as_mana_ability(),
        },
    ) {
        Outcome::ActivatedAbilityRegistered(ability) => ability,
        _ => return Some(json!({"setup_succeeded": false})),
    };

    let artifact_target = TargetChoice::Object(artifact);
    let enchantment_target = TargetChoice::Object(enchantment);
    let creature_target = TargetChoice::Object(creature);
    let artifact_target_accepted =
        state.can_target(controller, Some(source), requirement, artifact_target);
    let enchantment_target_accepted =
        state.can_target(controller, Some(source), requirement, enchantment_target);
    let creature_target_rejected =
        !state.can_target(controller, Some(source), requirement, creature_target);
    let artifact_actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_source(source)
            .with_targets(vec![artifact_target]),
    )
    .ok()?;
    let enchantment_binding_accepted = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_source(source)
            .with_targets(vec![enchantment_target]),
    )
    .is_ok();
    let creature_binding_rejected = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_source(source)
            .with_targets(vec![creature_target]),
    )
    .is_err();
    let destroy_action_exact = matches!(
        artifact_actions.as_slice(),
        [bound] if matches!(bound.action(), Action::DestroyPermanent { object } if *object == artifact)
    );

    let priority_ready = prepare_stack_priority(&mut state, controller);
    let generic_mana = program.mana_cost().generic_total().ok()?;
    let exact_pool = program
        .mana_cost()
        .colored_pool()
        .checked_add(ManaPool::of(ManaKind::Colorless, generic_mana))?;
    let funded = matches!(
        apply(
            &mut state,
            Action::AddManaToPool {
                player: controller,
                mana: exact_pool,
            },
        ),
        Outcome::Applied
    );
    let payment = auto_payment_plan(exact_pool, program.mana_cost())
        .ok()
        .flatten()?;
    let split_entry = match apply(
        &mut state,
        Action::CastSpell {
            player: controller,
            object: source,
            request: CastSpellRequest::new(
                StackObjectKind::InstantSpell,
                SpellTiming::Instant,
                program.mana_cost(),
                payment,
            )
            .with_targets(vec![requirement], vec![artifact_target])
            .with_split_second(),
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source_on_stack = state.object_zone(source) == Some(ZoneId::new(None, ZoneKind::Stack));
    let stack_entry_marked_split_second = state
        .stack_entries()
        .iter()
        .any(|entry| entry.id() == split_entry && entry.split_second());
    let cast_cost_consumed = state.mana_pool(controller).ok() == Some(ManaPool::empty());
    let priority_passed_to_responder = matches!(
        apply(
            &mut state,
            Action::PassPriority { player: controller },
        ),
        Outcome::Priority(PriorityOutcome::PassedTo(player)) if player == opponent
    );

    let ordinary_request = CastSpellRequest::new(
        StackObjectKind::InstantSpell,
        SpellTiming::Instant,
        zero_cost,
        zero_payment,
    );
    let before_spell = state.deterministic_hash();
    let responder_spell_rejected_before_mutation = matches!(
        apply(
            &mut state,
            Action::CastSpell {
                player: opponent,
                object: blocked_spell,
                request: ordinary_request.clone(),
            },
        ),
        Outcome::Failed(StateError::SplitSecondActionForbidden)
    ) && state.deterministic_hash() == before_spell;
    let before_non_mana = state.deterministic_hash();
    let responder_non_mana_ability_rejected_before_mutation = matches!(
        apply(
            &mut state,
            Action::ActivateAbility {
                player: opponent,
                ability: non_mana_ability,
                payment: zero_payment,
            },
        ),
        Outcome::Failed(StateError::SplitSecondActionForbidden)
    ) && state.deterministic_hash()
        == before_non_mana;
    let responder_mana_ability_allowed = matches!(
        apply(
            &mut state,
            Action::ActivateAbility {
                player: opponent,
                ability: mana_ability,
                payment: zero_payment,
            },
        ),
        Outcome::Applied
    );
    let responder_green_mana_added =
        state.mana_pool(opponent).ok() == Some(ManaPool::of(ManaKind::Green, 1));
    let mana_source_tapped = state
        .object(mana_source)
        .is_some_and(|record| record.tapped());
    let stack_resolved = matches!(
        apply(&mut state, Action::PassPriority { player: opponent }),
        Outcome::Priority(PriorityOutcome::Resolved(entry)) if entry == split_entry
    );
    let source_moved_to_owner_graveyard = state.object_zone(source) == Some(controller_graveyard);
    let resolution_recorded_split_second = state.resolution_log().last().is_some_and(|record| {
        record.stack_entry() == split_entry
            && record.outcome() == ResolutionOutcome::Resolved
            && record.split_second()
    });
    let destroy_action_applied = artifact_actions
        .iter()
        .all(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let artifact_destroyed_to_owner_graveyard =
        state.object_zone(artifact) == Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard));
    let ordinary_cast_available_after_resolution = matches!(
        apply(
            &mut state,
            Action::CastSpell {
                player: controller,
                object: follow_up_spell,
                request: ordinary_request,
            },
        ),
        Outcome::StackEntryAdded(_)
    );

    Some(json!({
        "setup_succeeded": priority_ready && funded,
        "split_second_compiled": program.split_second(),
        "target_slot_count": program.target_requirements().len(),
        "artifact_target_accepted": artifact_target_accepted,
        "enchantment_target_accepted": enchantment_target_accepted,
        "creature_target_rejected": creature_target_rejected,
        "enchantment_binding_accepted": enchantment_binding_accepted,
        "creature_binding_rejected": creature_binding_rejected,
        "bound_action_count": artifact_actions.len(),
        "destroy_action_exact": destroy_action_exact,
        "printed_generic_mana": generic_mana,
        "printed_green_mana": program.mana_cost().colored_pool().get(ManaKind::Green),
        "cast_payment_total": payment.paid().total(),
        "source_on_stack": source_on_stack,
        "stack_entry_marked_split_second": stack_entry_marked_split_second,
        "cast_cost_consumed": cast_cost_consumed,
        "priority_passed_to_responder": priority_passed_to_responder,
        "responder_spell_rejected_before_mutation": responder_spell_rejected_before_mutation,
        "responder_non_mana_ability_rejected_before_mutation":
            responder_non_mana_ability_rejected_before_mutation,
        "responder_mana_ability_allowed": responder_mana_ability_allowed,
        "responder_green_mana_added": responder_green_mana_added,
        "mana_source_tapped": mana_source_tapped,
        "stack_resolved": stack_resolved,
        "source_moved_to_owner_graveyard": source_moved_to_owner_graveyard,
        "resolution_recorded_split_second": resolution_recorded_split_second,
        "destroy_action_applied": destroy_action_applied,
        "artifact_destroyed_to_owner_graveyard": artifact_destroyed_to_owner_graveyard,
        "ordinary_cast_available_after_resolution": ordinary_cast_available_after_resolution,
    }))
}

fn noncreature_counter_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.alternate_costs().is_empty()
        || !program
            .effects()
            .iter()
            .any(|effect| matches!(effect, EffectProgram::CounterStackEntry { .. }))
    {
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
    let creature_object = create_probe_object(
        &mut state,
        9_620_000,
        opponent,
        opponent,
        ZoneId::new(Some(opponent), ZoneKind::Hand),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none()),
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let noncreature_object = create_probe_object(
        &mut state,
        9_620_001,
        opponent,
        opponent,
        ZoneId::new(Some(opponent), ZoneKind::Hand),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_instant(), ObjectColors::none()),
        None,
    )?;
    let priority_ready = prepare_stack_priority(&mut state, controller);
    let creature_entry = match apply(
        &mut state,
        Action::PutSpellOnStack {
            player: controller,
            object: creature_object,
            kind: StackObjectKind::PermanentSpell,
            hold_priority: true,
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let noncreature_entry = match apply(
        &mut state,
        Action::PutSpellOnStack {
            player: controller,
            object: noncreature_object,
            kind: StackObjectKind::InstantSpell,
            hold_priority: true,
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let target_slots = program.target_requirements().len();
    let requirement = *program.target_requirements().first()?;
    let requirement_is_stack_entry = requirement.kind() == TargetKind::StackEntry;
    let creature_stack_target_rejected = !state.can_target(
        controller,
        None,
        requirement,
        TargetChoice::StackEntry(creature_entry),
    );
    let noncreature_stack_target_accepted = state.can_target(
        controller,
        None,
        requirement,
        TargetChoice::StackEntry(noncreature_entry),
    );
    let creature_binding_rejected = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_targets(vec![TargetChoice::StackEntry(creature_entry)]),
    )
    .is_err();
    let legal_actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_targets(vec![TargetChoice::StackEntry(noncreature_entry)]),
    )
    .ok()?;
    let source_bound_counter_action = legal_actions.len() == 1
        && matches!(
            legal_actions.first().map(|bound| bound.action()),
            Some(Action::CounterStackEntry { entry }) if *entry == noncreature_entry
        );
    let counter_action_applied = legal_actions
        .iter()
        .all(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let noncreature_countered_to_owner_graveyard = state.object_zone(noncreature_object)
        == Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard));
    let creature_remained_on_stack = state
        .stack_entries()
        .iter()
        .any(|entry| entry.id() == creature_entry)
        && state.object_zone(creature_object) == Some(ZoneId::new(None, ZoneKind::Stack));

    Some(json!({
        "setup_succeeded": priority_ready,
        "target_slots": target_slots,
        "requirement_is_stack_entry": requirement_is_stack_entry,
        "creature_stack_target_rejected": creature_stack_target_rejected,
        "noncreature_stack_target_accepted": noncreature_stack_target_accepted,
        "creature_binding_rejected": creature_binding_rejected,
        "bound_action_count": legal_actions.len(),
        "source_bound_counter_action": source_bound_counter_action,
        "counter_action_applied": counter_action_applied,
        "noncreature_countered_to_owner_graveyard":
            noncreature_countered_to_owner_graveyard,
        "creature_remained_on_stack": creature_remained_on_stack,
    }))
}

fn temporary_creature_protection_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.alternate_costs().is_empty()
        || !program.effects().iter().any(|effect| {
            matches!(
                effect,
                EffectProgram::GrantIndestructible {
                    duration: ContinuousEffectDuration::UntilEndOfTurn,
                    ..
                }
            )
        })
    {
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
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let creature_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none());
    let destroy_creature = create_probe_object(
        &mut state,
        9_630_000,
        controller,
        controller,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let lethal_creature = create_probe_object(
        &mut state,
        9_630_001,
        controller,
        controller,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let controlled_artifact = create_probe_object(
        &mut state,
        9_630_002,
        controller,
        controller,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let opponent_creature = create_probe_object(
        &mut state,
        9_630_003,
        opponent,
        opponent,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent]),
    )
    .ok()?;
    let bound_actions_are_restrictions = actions
        .iter()
        .all(|bound| matches!(bound.action(), Action::RegisterRestriction { .. }));
    let bound_action_count = actions.len();
    let all_actions_applied = actions.iter().all(|bound| {
        matches!(
            apply(&mut state, bound.action().clone()),
            Outcome::RestrictionRegistered(_)
        )
    });
    let restriction_count = state.restrictions().count();
    let destroy_creature_protected = has_indestructible_restriction(&state, destroy_creature);
    let lethal_creature_protected = has_indestructible_restriction(&state, lethal_creature);
    let controlled_noncreature_unprotected =
        !has_indestructible_restriction(&state, controlled_artifact);
    let opponent_creature_unprotected = !has_indestructible_restriction(&state, opponent_creature);
    let protected_creature_survived_destroy = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: destroy_creature,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(destroy_creature)
        == Some(battlefield);
    let protected_creature_survived_lethal_damage = matches!(
        apply(
            &mut state,
            Action::MarkDamageOnObject {
                object: lethal_creature,
                amount: 2,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(&mut state, Action::CheckStateBasedActions),
        Outcome::StateBasedActions(report) if report.actions_performed() == 0
    ) && state.object_zone(lethal_creature)
        == Some(battlefield);
    let controlled_noncreature_destroyed_while_effect_active = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: controlled_artifact,
            },
        ),
        Outcome::Applied
    ) && state
        .object_zone(controlled_artifact)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));

    let cleanup_reached = advance_to_cleanup(&mut state, controller);
    let expired_restriction_count = state.last_cleanup_report().expired_until_end_of_turn();
    let restrictions_removed_at_cleanup = state.restrictions().next().is_none();
    let creature_destroyed_after_cleanup = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: destroy_creature,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(destroy_creature)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));
    let creature_died_to_lethal_damage_after_cleanup = matches!(
        apply(
            &mut state,
            Action::MarkDamageOnObject {
                object: lethal_creature,
                amount: 2,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(&mut state, Action::CheckStateBasedActions),
        Outcome::StateBasedActions(report) if report.actions_performed() == 1
    ) && state.object_zone(lethal_creature)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));

    Some(json!({
        "setup_succeeded": true,
        "bound_action_count": bound_action_count,
        "bound_actions_are_restrictions": bound_actions_are_restrictions,
        "all_actions_applied": all_actions_applied,
        "restriction_count": restriction_count,
        "destroy_creature_protected": destroy_creature_protected,
        "lethal_creature_protected": lethal_creature_protected,
        "controlled_noncreature_unprotected": controlled_noncreature_unprotected,
        "opponent_creature_unprotected": opponent_creature_unprotected,
        "protected_creature_survived_destroy": protected_creature_survived_destroy,
        "protected_creature_survived_lethal_damage":
            protected_creature_survived_lethal_damage,
        "controlled_noncreature_destroyed_while_effect_active":
            controlled_noncreature_destroyed_while_effect_active,
        "cleanup_reached": cleanup_reached,
        "expired_restriction_count": expired_restriction_count,
        "restrictions_removed_at_cleanup": restrictions_removed_at_cleanup,
        "creature_destroyed_after_cleanup": creature_destroyed_after_cleanup,
        "creature_died_to_lethal_damage_after_cleanup":
            creature_died_to_lethal_damage_after_cleanup,
    }))
}

fn temporary_protection_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let has_hexproof_program = program.effects().iter().any(|effect| {
        matches!(
            effect,
            EffectProgram::GrantTargetingRestriction {
                restriction: TargetRestriction::Hexproof,
                duration: ContinuousEffectDuration::UntilEndOfTurn,
                ..
            }
        )
    });
    let has_indestructible_program = program.effects().iter().any(|effect| {
        matches!(
            effect,
            EffectProgram::GrantIndestructible {
                duration: ContinuousEffectDuration::UntilEndOfTurn,
                ..
            }
        )
    });
    if !has_hexproof_program || !has_indestructible_program {
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
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let creature_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none());
    let artifact_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none());
    let controlled_creature = create_probe_object(
        &mut state,
        9_600_000,
        controller,
        controller,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let controlled_artifact = create_probe_object(
        &mut state,
        9_600_001,
        controller,
        controller,
        battlefield,
        artifact_base,
        None,
    )?;
    let opponent_creature = create_probe_object(
        &mut state,
        9_600_002,
        opponent,
        opponent,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let opponent_artifact = create_probe_object(
        &mut state,
        9_600_003,
        opponent,
        opponent,
        battlefield,
        artifact_base,
        None,
    )?;

    let bindings = ExecutionBindings::new(controller, vec![opponent]);
    let actions = bind_program_actions(&state, program, &bindings).ok()?;
    let bound_actions_are_restrictions = actions
        .iter()
        .all(|bound| matches!(bound.action(), Action::RegisterRestriction { .. }));
    let bound_action_count = actions.len();
    let all_actions_applied = actions.iter().all(|bound| {
        matches!(
            apply(&mut state, bound.action().clone()),
            Outcome::RestrictionRegistered(_)
        )
    });
    let restriction_count = state.restrictions().count();
    let all_until_end_of_turn = state
        .restrictions()
        .all(|(_, definition)| definition.duration() == ContinuousEffectDuration::UntilEndOfTurn);
    let controlled_creature_has_hexproof = has_hexproof_restriction(&state, controlled_creature);
    let controlled_artifact_has_hexproof = has_hexproof_restriction(&state, controlled_artifact);
    let controlled_creature_has_indestructible =
        has_indestructible_restriction(&state, controlled_creature);
    let controlled_artifact_has_indestructible =
        has_indestructible_restriction(&state, controlled_artifact);
    let opponent_creature_unprotected = !has_hexproof_restriction(&state, opponent_creature)
        && !has_indestructible_restriction(&state, opponent_creature);
    let opponent_artifact_unprotected = !has_hexproof_restriction(&state, opponent_artifact)
        && !has_indestructible_restriction(&state, opponent_artifact);
    let opponent_cannot_target_controlled_creature =
        !object_targetable(&state, opponent, controlled_creature);
    let opponent_cannot_target_controlled_artifact =
        !object_targetable(&state, opponent, controlled_artifact);
    let controller_can_target_controlled_creature =
        object_targetable(&state, controller, controlled_creature);
    let controller_can_target_controlled_artifact =
        object_targetable(&state, controller, controlled_artifact);

    let protected_artifact_survived_destroy = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: controlled_artifact,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(controlled_artifact)
        == Some(battlefield);
    let protected_creature_survived_destroy = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: controlled_creature,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(controlled_creature)
        == Some(battlefield);
    let protected_creature_survived_lethal_damage = matches!(
        apply(
            &mut state,
            Action::MarkDamageOnObject {
                object: controlled_creature,
                amount: 2,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(&mut state, Action::CheckStateBasedActions),
        Outcome::StateBasedActions(report) if report.actions_performed() == 0
    ) && state.object_zone(controlled_creature)
        == Some(battlefield);

    let cleanup_reached = advance_to_cleanup(&mut state, controller);
    let expired_restriction_count = state.last_cleanup_report().expired_until_end_of_turn();
    let restrictions_removed_at_cleanup = state.restrictions().next().is_none();
    let opponent_can_target_creature_after_cleanup =
        object_targetable(&state, opponent, controlled_creature);
    let opponent_can_target_artifact_after_cleanup =
        object_targetable(&state, opponent, controlled_artifact);
    let artifact_destroyed_after_cleanup = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: controlled_artifact,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(controlled_artifact)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));
    let creature_died_to_lethal_damage_after_cleanup = matches!(
        apply(
            &mut state,
            Action::MarkDamageOnObject {
                object: controlled_creature,
                amount: 2,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(&mut state, Action::CheckStateBasedActions),
        Outcome::StateBasedActions(report) if report.actions_performed() == 1
    ) && state.object_zone(controlled_creature)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));

    Some(json!({
        "setup_succeeded": true,
        "bound_action_count": bound_action_count,
        "bound_actions_are_restrictions": bound_actions_are_restrictions,
        "all_actions_applied": all_actions_applied,
        "restriction_count": restriction_count,
        "all_until_end_of_turn": all_until_end_of_turn,
        "controlled_creature_has_hexproof": controlled_creature_has_hexproof,
        "controlled_artifact_has_hexproof": controlled_artifact_has_hexproof,
        "controlled_creature_has_indestructible": controlled_creature_has_indestructible,
        "controlled_artifact_has_indestructible": controlled_artifact_has_indestructible,
        "opponent_creature_unprotected": opponent_creature_unprotected,
        "opponent_artifact_unprotected": opponent_artifact_unprotected,
        "opponent_cannot_target_controlled_creature":
            opponent_cannot_target_controlled_creature,
        "opponent_cannot_target_controlled_artifact":
            opponent_cannot_target_controlled_artifact,
        "controller_can_target_controlled_creature": controller_can_target_controlled_creature,
        "controller_can_target_controlled_artifact": controller_can_target_controlled_artifact,
        "protected_creature_survived_destroy": protected_creature_survived_destroy,
        "protected_artifact_survived_destroy": protected_artifact_survived_destroy,
        "protected_creature_survived_lethal_damage":
            protected_creature_survived_lethal_damage,
        "cleanup_reached": cleanup_reached,
        "expired_restriction_count": expired_restriction_count,
        "restrictions_removed_at_cleanup": restrictions_removed_at_cleanup,
        "opponent_can_target_creature_after_cleanup":
            opponent_can_target_creature_after_cleanup,
        "opponent_can_target_artifact_after_cleanup":
            opponent_can_target_artifact_after_cleanup,
        "artifact_destroyed_after_cleanup": artifact_destroyed_after_cleanup,
        "creature_died_to_lethal_damage_after_cleanup":
            creature_died_to_lethal_damage_after_cleanup,
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
        "temporary_protection": temporary_protection_probe(program),
        "commander_alternate_cost": commander_alternate_cost_probe(program),
        "flashback_looting": flashback_looting_probe(program),
        "split_second": split_second_probe(program),
        "noncreature_counter": noncreature_counter_probe(program),
        "temporary_creature_protection": temporary_creature_protection_probe(program),
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

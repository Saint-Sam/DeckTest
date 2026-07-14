#![forbid(unsafe_code)]
//! Production-path semantic probe for the frozen T3.6 Commander set.

use forge_cards::runtime::{
    bind_activated_effect_actions, bind_program_actions, bind_triggered_ability_actions,
    compile_card_program, execute_program, ActivatedAbilityProgram, ActivatedEffectProgram,
    AlternateCostCondition, AlternateCostKind, AmountProgram, CardProgram, EffectProgram,
    ExecutionBindings, ExecutionDiagnosticCode, ObjectSetProgram, PlayerBinding, ProgramKind,
    StaticAbilityProgram, TriggeredEventProgram,
};
use forge_core::{
    apply, auto_payment_plan, AbilityPlayer, Action, ActivatedAbilityDefinition,
    ActivatedAbilityEffect, ActivationCondition, ActivationCost, ActivationTiming,
    AttackDeclaration, BaseCreatureCharacteristics, BaseObjectCharacteristics, BlockDeclaration,
    CardId, CastSpellRequest, CombatDamageAssignment, CombatDamageAssignmentRequest,
    CombatDamageTarget, CombatRestriction, CombatRestrictionSubject, ContinuousEffectCondition,
    ContinuousEffectDuration, ContinuousEffectOperation, ContinuousEffectTarget, CounterKind,
    CreatureKeywords, GameEvent, GameState, ManaCost, ManaKind, ManaPool, ObjectColors,
    ObjectSupertypes, ObjectTargetPredicate, ObjectTypes, Outcome, PlayerRule, PriorityOutcome,
    ResolutionOutcome, RestrictionDefinition, RestrictionEffect, SpellTiming, StackObjectKind,
    StateError, Step, TargetChoice, TargetControllerPredicate, TargetKind, TargetRequirement,
    TargetRestriction, TargetRestrictionSubject, TriggerCondition, TriggerObjectFilter,
    TriggerPlayerFilter, ZoneId, ZoneKind,
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

struct OverloadFixture {
    state: GameState,
    controller: forge_core::PlayerId,
    opponents: [forge_core::PlayerId; 2],
    source: forge_core::ObjectId,
    eligible: [forge_core::ObjectId; 3],
    friendly_nonland: forge_core::ObjectId,
    opponent_land: forge_core::ObjectId,
}

fn create_overload_fixture(program: &CardProgram, salt: u32) -> Option<OverloadFixture> {
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return None,
    };
    let first_opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return None,
    };
    let second_opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return None,
    };
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let source = create_probe_object(
        &mut state,
        salt,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Hand),
        program.base_object(),
        program.base_creature(),
    )?;
    let first = create_probe_object(
        &mut state,
        salt.wrapping_add(1),
        first_opponent,
        first_opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let second = create_probe_object(
        &mut state,
        salt.wrapping_add(2),
        second_opponent,
        second_opponent,
        battlefield,
        BaseObjectCharacteristics::new(
            ObjectTypes::none().with_enchantment(),
            ObjectColors::none(),
        ),
        None,
    )?;
    let stolen = create_probe_object(
        &mut state,
        salt.wrapping_add(3),
        controller,
        first_opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let friendly_nonland = create_probe_object(
        &mut state,
        salt.wrapping_add(4),
        controller,
        controller,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let opponent_land = create_probe_object(
        &mut state,
        salt.wrapping_add(5),
        second_opponent,
        second_opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_land(), ObjectColors::none()),
        None,
    )?;
    Some(OverloadFixture {
        state,
        controller,
        opponents: [first_opponent, second_opponent],
        source,
        eligible: [first, second, stolen],
        friendly_nonland,
        opponent_land,
    })
}

fn overload_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if !program.overload() {
        return None;
    }
    let alternate = program
        .alternate_costs()
        .iter()
        .copied()
        .find(|cost| cost.kind() == AlternateCostKind::Overload)?;
    let requirement = *program.target_requirements().first()?;

    let mut ordinary = create_overload_fixture(program, 9_660_000)?;
    let controller_hand = ZoneId::new(Some(ordinary.controller), ZoneKind::Hand);
    let controller_graveyard = ZoneId::new(Some(ordinary.controller), ZoneKind::Graveyard);
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let available_in_hand =
        alternate.is_available(&ordinary.state, ordinary.controller, Some(ordinary.source));
    let moved_outside_hand = matches!(
        apply(
            &mut ordinary.state,
            Action::MoveObject {
                object: ordinary.source,
                to: controller_graveyard,
            },
        ),
        Outcome::Applied
    );
    let unavailable_outside_hand = moved_outside_hand
        && !alternate.is_available(&ordinary.state, ordinary.controller, Some(ordinary.source));
    let returned_to_hand = matches!(
        apply(
            &mut ordinary.state,
            Action::MoveObject {
                object: ordinary.source,
                to: controller_hand,
            },
        ),
        Outcome::Applied
    );
    let available_after_return_to_hand = returned_to_hand
        && alternate.is_available(&ordinary.state, ordinary.controller, Some(ordinary.source));
    let ordinary_target = TargetChoice::Object(ordinary.eligible[0]);
    let friendly_target = TargetChoice::Object(ordinary.friendly_nonland);
    let land_target = TargetChoice::Object(ordinary.opponent_land);
    let opponent_nonland_target_accepted = ordinary.state.can_target(
        ordinary.controller,
        Some(ordinary.source),
        requirement,
        ordinary_target,
    );
    let controller_nonland_target_rejected = !ordinary.state.can_target(
        ordinary.controller,
        Some(ordinary.source),
        requirement,
        friendly_target,
    );
    let opponent_land_target_rejected = !ordinary.state.can_target(
        ordinary.controller,
        Some(ordinary.source),
        requirement,
        land_target,
    );
    let ordinary_binding_without_target_rejected = bind_program_actions(
        &ordinary.state,
        program,
        &ExecutionBindings::new(ordinary.controller, ordinary.opponents.to_vec())
            .with_source(ordinary.source),
    )
    .is_err();
    let ordinary_friendly_binding_rejected = bind_program_actions(
        &ordinary.state,
        program,
        &ExecutionBindings::new(ordinary.controller, ordinary.opponents.to_vec())
            .with_source(ordinary.source)
            .with_targets(vec![friendly_target]),
    )
    .is_err();
    let ordinary_land_binding_rejected = bind_program_actions(
        &ordinary.state,
        program,
        &ExecutionBindings::new(ordinary.controller, ordinary.opponents.to_vec())
            .with_source(ordinary.source)
            .with_targets(vec![land_target]),
    )
    .is_err();
    let ordinary_actions = bind_program_actions(
        &ordinary.state,
        program,
        &ExecutionBindings::new(ordinary.controller, ordinary.opponents.to_vec())
            .with_source(ordinary.source)
            .with_targets(vec![ordinary_target]),
    )
    .ok()?;
    let ordinary_action_exact = matches!(
        ordinary_actions.as_slice(),
        [bound]
            if matches!(
                bound.action(),
                Action::MoveObject { object, to }
                    if *object == ordinary.eligible[0]
                        && *to == ZoneId::new(Some(ordinary.opponents[0]), ZoneKind::Hand)
            )
    );
    let ordinary_priority_ready = prepare_stack_priority(&mut ordinary.state, ordinary.controller);
    let ordinary_generic = program.mana_cost().generic_total().ok()?;
    let ordinary_pool = program
        .mana_cost()
        .colored_pool()
        .checked_add(ManaPool::of(ManaKind::Colorless, ordinary_generic))?;
    let ordinary_funded = matches!(
        apply(
            &mut ordinary.state,
            Action::AddManaToPool {
                player: ordinary.controller,
                mana: ordinary_pool,
            },
        ),
        Outcome::Applied
    );
    let ordinary_payment = auto_payment_plan(ordinary_pool, program.mana_cost())
        .ok()
        .flatten()?;
    let before_missing_target_cast = ordinary.state.deterministic_hash();
    let ordinary_cast_without_target_rejected_before_mutation =
        matches!(
            apply(
                &mut ordinary.state,
                Action::CastSpell {
                    player: ordinary.controller,
                    object: ordinary.source,
                    request: CastSpellRequest::new(
                        StackObjectKind::InstantSpell,
                        SpellTiming::Instant,
                        program.mana_cost(),
                        ordinary_payment,
                    )
                    .with_targets(vec![requirement], Vec::new()),
                },
            ),
            Outcome::Failed(_)
        ) && ordinary.state.deterministic_hash() == before_missing_target_cast;
    let ordinary_entry = match apply(
        &mut ordinary.state,
        Action::CastSpell {
            player: ordinary.controller,
            object: ordinary.source,
            request: CastSpellRequest::new(
                StackObjectKind::InstantSpell,
                SpellTiming::Instant,
                program.mana_cost(),
                ordinary_payment,
            )
            .with_targets(vec![requirement], vec![ordinary_target]),
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        outcome => {
            return Some(json!({
                "setup_succeeded": false,
                "phase": "ordinary_cast",
                "outcome": format!("{outcome:?}"),
            }))
        }
    };
    let ordinary_source_on_stack =
        ordinary.state.object_zone(ordinary.source) == Some(ZoneId::new(None, ZoneKind::Stack));
    let ordinary_stack_target_exact = ordinary
        .state
        .stack_entries()
        .iter()
        .find(|entry| entry.id() == ordinary_entry)
        .is_some_and(|entry| {
            entry.targets().len() == 1 && entry.targets()[0].choice() == ordinary_target
        });
    let ordinary_cost_consumed =
        ordinary.state.mana_pool(ordinary.controller).ok() == Some(ManaPool::empty());
    let ordinary_stack_resolved = resolve_expected_stack_entry(&mut ordinary.state, ordinary_entry);
    let ordinary_source_moved_to_graveyard =
        ordinary.state.object_zone(ordinary.source) == Some(controller_graveyard);
    let ordinary_action_applied = ordinary_actions.iter().all(|bound| {
        matches!(
            apply(&mut ordinary.state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let ordinary_target_returned_to_owner_hand = ordinary.state.object_zone(ordinary.eligible[0])
        == Some(ZoneId::new(Some(ordinary.opponents[0]), ZoneKind::Hand));
    let ordinary_other_opponent_nonlands_unchanged = ordinary
        .eligible
        .iter()
        .skip(1)
        .all(|object| ordinary.state.object_zone(*object) == Some(battlefield));
    let ordinary_friendly_and_land_unchanged =
        ordinary.state.object_zone(ordinary.friendly_nonland) == Some(battlefield)
            && ordinary.state.object_zone(ordinary.opponent_land) == Some(battlefield);

    let mut overloaded = create_overload_fixture(program, 9_661_000)?;
    let overload_available_in_hand = alternate.is_available(
        &overloaded.state,
        overloaded.controller,
        Some(overloaded.source),
    );
    let overload_priority_ready =
        prepare_stack_priority(&mut overloaded.state, overloaded.controller);
    let overload_funded = matches!(
        apply(
            &mut overloaded.state,
            Action::AddManaToPool {
                player: overloaded.controller,
                mana: alternate.exact_payment(),
            },
        ),
        Outcome::Applied
    );
    let overload_payment = auto_payment_plan(alternate.exact_payment(), alternate.mana_cost())
        .ok()
        .flatten()?;
    let overload_entry = match apply(
        &mut overloaded.state,
        Action::CastSpell {
            player: overloaded.controller,
            object: overloaded.source,
            request: CastSpellRequest::new(
                StackObjectKind::InstantSpell,
                SpellTiming::Instant,
                alternate.mana_cost(),
                overload_payment,
            )
            .with_targets(
                program
                    .target_requirements_for_alternate(Some(AlternateCostKind::Overload))
                    .to_vec(),
                Vec::new(),
            ),
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        outcome => {
            return Some(json!({
                "setup_succeeded": false,
                "phase": "overload_cast",
                "outcome": format!("{outcome:?}"),
            }))
        }
    };
    let overload_cast_without_targets_succeeded =
        overloaded.state.object_zone(overloaded.source) == Some(ZoneId::new(None, ZoneKind::Stack));
    let overload_stack_has_no_targets = overloaded
        .state
        .stack_entries()
        .iter()
        .find(|entry| entry.id() == overload_entry)
        .is_some_and(|entry| entry.targets().is_empty());
    let overload_cost_consumed =
        overloaded.state.mana_pool(overloaded.controller).ok() == Some(ManaPool::empty());
    let overload_stack_resolved =
        resolve_expected_stack_entry(&mut overloaded.state, overload_entry);
    let overload_source_moved_to_graveyard = overloaded.state.object_zone(overloaded.source)
        == Some(ZoneId::new(
            Some(overloaded.controller),
            ZoneKind::Graveyard,
        ));
    let overload_explicit_target_rejected = bind_program_actions(
        &overloaded.state,
        program,
        &ExecutionBindings::new(overloaded.controller, overloaded.opponents.to_vec())
            .with_source(overloaded.source)
            .with_alternate_cost(AlternateCostKind::Overload)
            .with_targets(vec![TargetChoice::Object(overloaded.eligible[0])]),
    )
    .is_err();
    let overload_actions = bind_program_actions(
        &overloaded.state,
        program,
        &ExecutionBindings::new(overloaded.controller, overloaded.opponents.to_vec())
            .with_source(overloaded.source)
            .with_alternate_cost(AlternateCostKind::Overload),
    )
    .ok()?;
    let expected_overload_moves = [
        (
            overloaded.eligible[0],
            ZoneId::new(Some(overloaded.opponents[0]), ZoneKind::Hand),
        ),
        (
            overloaded.eligible[1],
            ZoneId::new(Some(overloaded.opponents[1]), ZoneKind::Hand),
        ),
        (
            overloaded.eligible[2],
            ZoneId::new(Some(overloaded.controller), ZoneKind::Hand),
        ),
    ];
    let overload_actions_exact = overload_actions.iter().zip(expected_overload_moves).all(
        |(bound, (expected_object, expected_zone))| {
            matches!(
                bound.action(),
                Action::MoveObject { object, to }
                    if *object == expected_object && *to == expected_zone
            )
        },
    ) && overload_actions.len() == expected_overload_moves.len();
    let overload_actions_applied = overload_actions.iter().all(|bound| {
        matches!(
            apply(&mut overloaded.state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let overload_each_opponent_nonland_returned =
        overloaded.state.object_zone(overloaded.eligible[0])
            == Some(ZoneId::new(Some(overloaded.opponents[0]), ZoneKind::Hand))
            && overloaded.state.object_zone(overloaded.eligible[1])
                == Some(ZoneId::new(Some(overloaded.opponents[1]), ZoneKind::Hand));
    let overload_stolen_permanent_returned_to_owner =
        overloaded.state.object_zone(overloaded.eligible[2])
            == Some(ZoneId::new(Some(overloaded.controller), ZoneKind::Hand));
    let overload_friendly_nonland_unchanged =
        overloaded.state.object_zone(overloaded.friendly_nonland) == Some(battlefield);
    let overload_opponent_land_unchanged =
        overloaded.state.object_zone(overloaded.opponent_land) == Some(battlefield);

    let contract = json!({
        "overload_compiled": program.overload(),
        "alternate_cost_count": program.alternate_costs().len(),
        "alternate_kind_is_overload": alternate.kind() == AlternateCostKind::Overload,
        "condition_is_source_in_controller_hand": matches!(
            alternate.condition(),
            AlternateCostCondition::SourceInControllerHand
        ),
        "printed_generic_mana": ordinary_generic,
        "printed_blue_mana": program.mana_cost().colored_pool().get(ManaKind::Blue),
        "overload_generic_mana": alternate.mana_cost().generic_total().ok(),
        "overload_blue_mana": alternate.mana_cost().colored_pool().get(ManaKind::Blue),
        "overload_exact_payment_total": alternate.exact_payment().total(),
        "available_in_hand": available_in_hand,
        "unavailable_outside_hand": unavailable_outside_hand,
        "available_after_return_to_hand": available_after_return_to_hand,
        "ordinary_target_slot_count": program.target_requirements_for_alternate(None).len(),
        "overload_target_slot_count": program
            .target_requirements_for_alternate(Some(AlternateCostKind::Overload))
            .len(),
        "opponent_nonland_target_accepted": opponent_nonland_target_accepted,
        "controller_nonland_target_rejected": controller_nonland_target_rejected,
        "opponent_land_target_rejected": opponent_land_target_rejected,
    });
    let ordinary_result = json!({
        "binding_without_target_rejected": ordinary_binding_without_target_rejected,
        "friendly_binding_rejected": ordinary_friendly_binding_rejected,
        "land_binding_rejected": ordinary_land_binding_rejected,
        "bound_action_count": ordinary_actions.len(),
        "action_exact": ordinary_action_exact,
        "cast_without_target_rejected_before_mutation":
            ordinary_cast_without_target_rejected_before_mutation,
        "cast_payment_total": ordinary_payment.paid().total(),
        "source_on_stack": ordinary_source_on_stack,
        "stack_target_exact": ordinary_stack_target_exact,
        "cost_consumed": ordinary_cost_consumed,
        "stack_resolved": ordinary_stack_resolved,
        "source_moved_to_graveyard": ordinary_source_moved_to_graveyard,
        "action_applied": ordinary_action_applied,
        "target_returned_to_owner_hand": ordinary_target_returned_to_owner_hand,
        "other_opponent_nonlands_unchanged": ordinary_other_opponent_nonlands_unchanged,
        "friendly_and_land_unchanged": ordinary_friendly_and_land_unchanged,
    });
    let overload_result = json!({
        "available_in_hand": overload_available_in_hand,
        "cast_payment_total": overload_payment.paid().total(),
        "cast_without_targets_succeeded": overload_cast_without_targets_succeeded,
        "stack_has_no_targets": overload_stack_has_no_targets,
        "cost_consumed": overload_cost_consumed,
        "stack_resolved": overload_stack_resolved,
        "source_moved_to_graveyard": overload_source_moved_to_graveyard,
        "explicit_target_rejected": overload_explicit_target_rejected,
        "bound_action_count": overload_actions.len(),
        "actions_exact": overload_actions_exact,
        "actions_applied": overload_actions_applied,
        "each_opponent_nonland_returned": overload_each_opponent_nonland_returned,
        "stolen_permanent_returned_to_owner": overload_stolen_permanent_returned_to_owner,
        "friendly_nonland_unchanged": overload_friendly_nonland_unchanged,
        "opponent_land_unchanged": overload_opponent_land_unchanged,
    });
    Some(json!({
        "setup_succeeded": ordinary_priority_ready
            && ordinary_funded
            && overload_priority_ready
            && overload_funded,
        "contract": contract,
        "ordinary": ordinary_result,
        "overload": overload_result,
    }))
}

struct EvokeFixture {
    state: GameState,
    controller: forge_core::PlayerId,
    opponent: forge_core::PlayerId,
    source: forge_core::ObjectId,
}

fn create_evoke_fixture(program: &CardProgram, salt: u32) -> Option<EvokeFixture> {
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return None,
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return None,
    };
    let source = create_probe_object(
        &mut state,
        salt,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Hand),
        program.base_object(),
        program.base_creature(),
    )?;
    for offset in 1..=4 {
        if !matches!(
            apply(
                &mut state,
                Action::CreateObject {
                    card: CardId::new(salt.wrapping_add(offset)),
                    owner: controller,
                    controller,
                    zone: ZoneId::new(Some(controller), ZoneKind::Library),
                },
            ),
            Outcome::ObjectCreated(_)
        ) {
            return None;
        }
    }
    Some(EvokeFixture {
        state,
        controller,
        opponent,
        source,
    })
}

fn evoke_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let alternate = program
        .alternate_costs()
        .iter()
        .copied()
        .find(|cost| cost.kind() == AlternateCostKind::Evoke)?;
    let [draw_trigger, sacrifice_trigger] = program.triggered_abilities() else {
        return Some(json!({"setup_succeeded": false}));
    };
    let draw_trigger_is_unconditional = draw_trigger.required_alternate_cost().is_none();
    let sacrifice_trigger_requires_evoke =
        sacrifice_trigger.required_alternate_cost() == Some(AlternateCostKind::Evoke);
    let both_triggers_are_source_enters = draw_trigger.event()
        == TriggeredEventProgram::SourceEnters
        && sacrifice_trigger.event() == TriggeredEventProgram::SourceEnters;
    let normal_applicable_trigger_count = program
        .triggered_abilities()
        .iter()
        .filter(|ability| ability.required_alternate_cost().is_none())
        .count();
    let evoke_applicable_trigger_count = program
        .triggered_abilities()
        .iter()
        .filter(|ability| {
            ability
                .required_alternate_cost()
                .map_or(true, |required| required == AlternateCostKind::Evoke)
        })
        .count();

    let mut normal = create_evoke_fixture(program, 9_670_000)?;
    let normal_cast_window_ready = advance_to_precombat_main(&mut normal.state, normal.controller);
    let printed_generic = program.mana_cost().generic_total().ok()?;
    let printed_pool = program
        .mana_cost()
        .colored_pool()
        .checked_add(ManaPool::of(ManaKind::Colorless, printed_generic))?;
    let normal_funded = matches!(
        apply(
            &mut normal.state,
            Action::AddManaToPool {
                player: normal.controller,
                mana: printed_pool,
            },
        ),
        Outcome::Applied
    );
    let normal_payment = auto_payment_plan(printed_pool, program.mana_cost())
        .ok()
        .flatten()?;
    let normal_entry = match apply(
        &mut normal.state,
        Action::CastSpell {
            player: normal.controller,
            object: normal.source,
            request: CastSpellRequest::new(
                StackObjectKind::PermanentSpell,
                SpellTiming::Sorcery,
                program.mana_cost(),
                normal_payment,
            ),
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        outcome => {
            return Some(json!({
                "setup_succeeded": false,
                "phase": "normal_cast",
                "outcome": format!("{outcome:?}"),
            }))
        }
    };
    let normal_source_on_stack =
        normal.state.object_zone(normal.source) == Some(ZoneId::new(None, ZoneKind::Stack));
    let normal_cost_consumed =
        normal.state.mana_pool(normal.controller).ok() == Some(ManaPool::empty());
    let normal_stack_resolved = resolve_expected_stack_entry(&mut normal.state, normal_entry);
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let normal_source_entered_battlefield =
        normal.state.object_zone(normal.source) == Some(battlefield);
    let normal_draw_actions = bind_triggered_ability_actions(
        &normal.state,
        draw_trigger,
        &ExecutionBindings::new(normal.controller, vec![normal.opponent])
            .with_source(normal.source),
    )
    .ok()?;
    let normal_draw_action_exact = matches!(
        normal_draw_actions.as_slice(),
        [bound]
            if matches!(
                bound.action(),
                Action::DrawCards { player, count }
                    if *player == normal.controller && *count == 2
            )
    );
    let normal_hand = ZoneId::new(Some(normal.controller), ZoneKind::Hand);
    let normal_hand_before_draw = normal
        .state
        .zone_objects(normal_hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let normal_draw_action_applied = normal_draw_actions.iter().all(|bound| {
        matches!(
            apply(&mut normal.state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let normal_hand_after_draw = normal
        .state
        .zone_objects(normal_hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let normal_exactly_two_drawn = normal_draw_action_applied
        && normal_hand_after_draw == normal_hand_before_draw.saturating_add(2);
    let normal_source_remained_battlefield_after_draw =
        normal.state.object_zone(normal.source) == Some(battlefield);
    let normal_evoke_trigger_excluded = normal_applicable_trigger_count == 1
        && sacrifice_trigger.required_alternate_cost() == Some(AlternateCostKind::Evoke);

    let mut evoked = create_evoke_fixture(program, 9_671_000)?;
    let evoke_available_in_hand =
        alternate.is_available(&evoked.state, evoked.controller, Some(evoked.source));
    let evoke_cast_window_ready = advance_to_precombat_main(&mut evoked.state, evoked.controller);
    let evoke_funded = matches!(
        apply(
            &mut evoked.state,
            Action::AddManaToPool {
                player: evoked.controller,
                mana: alternate.exact_payment(),
            },
        ),
        Outcome::Applied
    );
    let evoke_payment = auto_payment_plan(alternate.exact_payment(), alternate.mana_cost())
        .ok()
        .flatten()?;
    let evoke_entry = match apply(
        &mut evoked.state,
        Action::CastSpell {
            player: evoked.controller,
            object: evoked.source,
            request: CastSpellRequest::new(
                StackObjectKind::PermanentSpell,
                SpellTiming::Sorcery,
                alternate.mana_cost(),
                evoke_payment,
            ),
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        outcome => {
            return Some(json!({
                "setup_succeeded": false,
                "phase": "evoke_cast",
                "outcome": format!("{outcome:?}"),
            }))
        }
    };
    let evoke_source_on_stack =
        evoked.state.object_zone(evoked.source) == Some(ZoneId::new(None, ZoneKind::Stack));
    let evoke_cost_consumed =
        evoked.state.mana_pool(evoked.controller).ok() == Some(ManaPool::empty());
    let evoke_stack_resolved = resolve_expected_stack_entry(&mut evoked.state, evoke_entry);
    let evoke_source_entered_before_triggers =
        evoked.state.object_zone(evoked.source) == Some(battlefield);
    let evoke_bindings =
        ExecutionBindings::new(evoked.controller, vec![evoked.opponent]).with_source(evoked.source);
    let evoke_draw_actions =
        bind_triggered_ability_actions(&evoked.state, draw_trigger, &evoke_bindings).ok()?;
    let evoke_sacrifice_actions =
        bind_triggered_ability_actions(&evoked.state, sacrifice_trigger, &evoke_bindings).ok()?;
    let evoke_draw_action_exact = matches!(
        evoke_draw_actions.as_slice(),
        [bound]
            if matches!(
                bound.action(),
                Action::DrawCards { player, count }
                    if *player == evoked.controller && *count == 2
            )
    );
    let evoke_graveyard = ZoneId::new(Some(evoked.controller), ZoneKind::Graveyard);
    let evoke_sacrifice_action_exact = matches!(
        evoke_sacrifice_actions.as_slice(),
        [bound]
            if matches!(
                bound.action(),
                Action::MoveObject { object, to }
                    if *object == evoked.source && *to == evoke_graveyard
            )
    );
    let before_missing_source = evoked.state.deterministic_hash();
    let evoke_missing_source_rejected_without_mutation = bind_triggered_ability_actions(
        &evoked.state,
        sacrifice_trigger,
        &ExecutionBindings::new(evoked.controller, vec![evoked.opponent]),
    )
    .is_err()
        && evoked.state.deterministic_hash() == before_missing_source;
    let evoke_hand = ZoneId::new(Some(evoked.controller), ZoneKind::Hand);
    let evoke_hand_before_draw = evoked
        .state
        .zone_objects(evoke_hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let evoke_draw_action_applied = evoke_draw_actions.iter().all(|bound| {
        matches!(
            apply(&mut evoked.state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let evoke_hand_after_draw = evoked
        .state
        .zone_objects(evoke_hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let evoke_exactly_two_drawn = evoke_draw_action_applied
        && evoke_hand_after_draw == evoke_hand_before_draw.saturating_add(2);
    let evoke_source_remained_battlefield_after_draw =
        evoked.state.object_zone(evoked.source) == Some(battlefield);
    let evoke_sacrifice_action_applied = evoke_sacrifice_actions.iter().all(|bound| {
        matches!(
            apply(&mut evoked.state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let evoke_source_moved_to_owner_graveyard =
        evoked.state.object_zone(evoked.source) == Some(evoke_graveyard);
    let evoke_draw_then_sacrificed = evoke_exactly_two_drawn
        && evoke_source_remained_battlefield_after_draw
        && evoke_sacrifice_action_applied
        && evoke_source_moved_to_owner_graveyard;

    let contract = json!({
        "alternate_cost_count": program.alternate_costs().len(),
        "alternate_kind_is_evoke": alternate.kind() == AlternateCostKind::Evoke,
        "condition_is_source_in_controller_hand": matches!(
            alternate.condition(),
            AlternateCostCondition::SourceInControllerHand
        ),
        "trigger_count": program.triggered_abilities().len(),
        "draw_trigger_is_unconditional": draw_trigger_is_unconditional,
        "sacrifice_trigger_requires_evoke": sacrifice_trigger_requires_evoke,
        "both_triggers_are_source_enters": both_triggers_are_source_enters,
        "printed_generic_mana": printed_generic,
        "printed_blue_mana": program.mana_cost().colored_pool().get(ManaKind::Blue),
        "printed_payment_total": printed_pool.total(),
        "evoke_generic_mana": alternate.mana_cost().generic_total().ok(),
        "evoke_blue_mana": alternate.mana_cost().colored_pool().get(ManaKind::Blue),
        "evoke_exact_payment_total": alternate.exact_payment().total(),
        "evoke_available_in_hand": evoke_available_in_hand,
        "normal_applicable_trigger_count": normal_applicable_trigger_count,
        "evoke_applicable_trigger_count": evoke_applicable_trigger_count,
    });
    let normal_result = json!({
        "cast_window_ready": normal_cast_window_ready,
        "cast_payment_total": normal_payment.paid().total(),
        "source_on_stack": normal_source_on_stack,
        "cost_consumed": normal_cost_consumed,
        "stack_resolved": normal_stack_resolved,
        "source_entered_battlefield": normal_source_entered_battlefield,
        "draw_bound_action_count": normal_draw_actions.len(),
        "draw_action_exact": normal_draw_action_exact,
        "draw_action_applied": normal_draw_action_applied,
        "exactly_two_drawn": normal_exactly_two_drawn,
        "source_remained_battlefield_after_draw": normal_source_remained_battlefield_after_draw,
        "evoke_trigger_excluded": normal_evoke_trigger_excluded,
    });
    let evoke_result = json!({
        "cast_window_ready": evoke_cast_window_ready,
        "cast_payment_total": evoke_payment.paid().total(),
        "source_on_stack": evoke_source_on_stack,
        "cost_consumed": evoke_cost_consumed,
        "stack_resolved": evoke_stack_resolved,
        "source_entered_before_triggers": evoke_source_entered_before_triggers,
        "draw_bound_action_count": evoke_draw_actions.len(),
        "sacrifice_bound_action_count": evoke_sacrifice_actions.len(),
        "draw_action_exact": evoke_draw_action_exact,
        "sacrifice_action_exact": evoke_sacrifice_action_exact,
        "missing_source_rejected_without_mutation":
            evoke_missing_source_rejected_without_mutation,
        "draw_action_applied": evoke_draw_action_applied,
        "exactly_two_drawn": evoke_exactly_two_drawn,
        "source_remained_battlefield_after_draw": evoke_source_remained_battlefield_after_draw,
        "sacrifice_action_applied": evoke_sacrifice_action_applied,
        "source_moved_to_owner_graveyard": evoke_source_moved_to_owner_graveyard,
        "draw_then_sacrificed": evoke_draw_then_sacrificed,
    });
    Some(json!({
        "setup_succeeded": normal_funded && evoke_funded,
        "contract": contract,
        "normal": normal_result,
        "evoke": evoke_result,
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

fn boros_charm_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.name() != "Boros Charm" {
        return None;
    }
    let modes = program.spell_modes();
    if modes.len() != 3 {
        return Some(json!({
            "setup_succeeded": false,
            "observed_mode_count": modes.len(),
        }));
    }

    let target_counts = modes
        .iter()
        .map(|mode| mode.target_requirements().len())
        .collect::<Vec<_>>();
    let modes_have_no_object_choices = modes
        .iter()
        .all(|mode| mode.object_choice_requirements().is_empty());
    let modes_have_no_optional_choices = modes.iter().all(|mode| mode.optional_choice_count() == 0);
    let damage_program_exact = matches!(
        modes[0].effects(),
        [EffectProgram::DealDamageToTarget {
            target: 0,
            amount: AmountProgram::Literal(4),
        }]
    );
    let indestructible_program_exact = matches!(
        modes[1].effects(),
        [EffectProgram::GrantIndestructible {
            objects: ObjectSetProgram::Battlefield(predicate),
            duration: ContinuousEffectDuration::UntilEndOfTurn,
        }] if predicate.controller() == TargetControllerPredicate::You
            && predicate.required_types() == ObjectTypes::none()
    );
    let double_strike_program_exact = matches!(
        modes[2].effects(),
        [EffectProgram::GrantKeywords {
            objects: ObjectSetProgram::Target(0),
            keywords,
            duration: ContinuousEffectDuration::UntilEndOfTurn,
        }] if *keywords == CreatureKeywords::none().with_double_strike()
    );

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
    let controlled_creature = create_probe_object(
        &mut state,
        9_660_000,
        controller,
        controller,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let controlled_artifact = create_probe_object(
        &mut state,
        9_660_001,
        controller,
        controller,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let opponent_creature = create_probe_object(
        &mut state,
        9_660_002,
        opponent,
        opponent,
        battlefield,
        creature_base,
        Some(BaseCreatureCharacteristics::new(3, 3)),
    )?;
    let opponent_artifact = create_probe_object(
        &mut state,
        9_660_003,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let planeswalker = create_probe_object(
        &mut state,
        9_660_004,
        opponent,
        opponent,
        battlefield,
        BaseObjectCharacteristics::new(
            ObjectTypes::none().with_planeswalker(),
            ObjectColors::none(),
        ),
        None,
    )?;
    if !matches!(
        apply(
            &mut state,
            Action::SetObjectLoyalty {
                object: planeswalker,
                loyalty: Some(7),
            },
        ),
        Outcome::Applied
    ) {
        return Some(json!({"setup_succeeded": false}));
    }

    let before_no_mode = state.deterministic_hash();
    let no_mode_rejected_before_mutation = matches!(
        bind_program_actions(
            &state,
            program,
            &ExecutionBindings::new(controller, vec![opponent]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::MissingChoice
    ) && state.deterministic_hash() == before_no_mode;
    let before_invalid_mode = state.deterministic_hash();
    let out_of_range_mode_rejected_before_mutation = matches!(
        bind_program_actions(
            &state,
            program,
            &ExecutionBindings::new(controller, vec![opponent]).with_spell_mode(3),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::MissingChoice
    ) && state.deterministic_hash()
        == before_invalid_mode;
    let before_mode_one_target = state.deterministic_hash();
    let targetless_mode_rejects_extra_target = matches!(
        bind_program_actions(
            &state,
            program,
            &ExecutionBindings::new(controller, vec![opponent])
                .with_spell_mode(1)
                .with_targets(vec![TargetChoice::Object(controlled_creature)]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && state.deterministic_hash()
        == before_mode_one_target;

    let damage_requirement_is_player_or_permanent = modes[0]
        .target_requirements()
        .first()
        .is_some_and(|requirement| requirement.kind() == TargetKind::PlayerOrPermanent);
    let player_actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_spell_mode(0)
            .with_targets(vec![TargetChoice::Player(opponent)]),
    )
    .ok()?;
    let player_target_accepted = player_actions.len() == 1;
    let player_bound_action_exact = player_actions.len() == 1
        && matches!(
            player_actions[0].action(),
            Action::DealDamage {
                source: None,
                target: CombatDamageTarget::Player(player),
                amount: 4,
            } if *player == opponent
        );
    let player_action_applied = matches!(
        apply(&mut state, player_actions[0].action().clone()),
        Outcome::Applied
    );
    let opponent_life_after_damage = state.players()[opponent.index()].life();

    let planeswalker_actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_spell_mode(0)
            .with_targets(vec![TargetChoice::Object(planeswalker)]),
    )
    .ok()?;
    let planeswalker_target_accepted = planeswalker_actions.len() == 1;
    let planeswalker_bound_action_exact = planeswalker_actions.len() == 1
        && matches!(
            planeswalker_actions[0].action(),
            Action::DealDamage {
                source: None,
                target: CombatDamageTarget::Object(object),
                amount: 4,
            } if *object == planeswalker
        );
    let planeswalker_action_applied = matches!(
        apply(&mut state, planeswalker_actions[0].action().clone()),
        Outcome::Applied
    );
    let planeswalker_loyalty_after_damage = state
        .object(planeswalker)
        .and_then(|record| record.loyalty());
    let before_invalid_damage_target = state.deterministic_hash();
    let nonplaneswalker_permanent_rejected_before_mutation = matches!(
        bind_program_actions(
            &state,
            program,
            &ExecutionBindings::new(controller, vec![opponent])
                .with_spell_mode(0)
                .with_targets(vec![TargetChoice::Object(opponent_artifact)]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && state.deterministic_hash()
        == before_invalid_damage_target;

    let indestructible_actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent]).with_spell_mode(1),
    )
    .ok()?;
    let indestructible_bound_action_count = indestructible_actions.len();
    let indestructible_actions_are_restrictions = indestructible_actions
        .iter()
        .all(|bound| matches!(bound.action(), Action::RegisterRestriction { .. }));
    let indestructible_actions_applied = indestructible_actions.iter().all(|bound| {
        matches!(
            apply(&mut state, bound.action().clone()),
            Outcome::RestrictionRegistered(_)
        )
    });
    let controlled_creature_protected = has_indestructible_restriction(&state, controlled_creature);
    let controlled_artifact_protected = has_indestructible_restriction(&state, controlled_artifact);
    let opponent_creature_unprotected = !has_indestructible_restriction(&state, opponent_creature);
    let opponent_artifact_unprotected = !has_indestructible_restriction(&state, opponent_artifact);

    let controlled_creature_target_accepted = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_spell_mode(2)
            .with_targets(vec![TargetChoice::Object(controlled_creature)]),
    )
    .is_ok();
    let double_strike_actions = bind_program_actions(
        &state,
        program,
        &ExecutionBindings::new(controller, vec![opponent])
            .with_spell_mode(2)
            .with_targets(vec![TargetChoice::Object(opponent_creature)]),
    )
    .ok()?;
    let opponent_creature_target_accepted = double_strike_actions.len() == 1;
    let before_noncreature_target = state.deterministic_hash();
    let noncreature_target_rejected_before_mutation = matches!(
        bind_program_actions(
            &state,
            program,
            &ExecutionBindings::new(controller, vec![opponent])
                .with_spell_mode(2)
                .with_targets(vec![TargetChoice::Object(controlled_artifact)]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && state.deterministic_hash()
        == before_noncreature_target;
    let double_strike_bound_action_count = double_strike_actions.len();
    let double_strike_action_is_continuous_effect = double_strike_actions
        .iter()
        .all(|bound| matches!(bound.action(), Action::RegisterContinuousEffect { .. }));
    let double_strike_actions_applied = double_strike_actions.iter().all(|bound| {
        matches!(
            apply(&mut state, bound.action().clone()),
            Outcome::ContinuousEffectRegistered(_)
        )
    });
    let opponent_creature_has_double_strike = state
        .creature_characteristics(opponent_creature)
        .is_ok_and(|characteristics| characteristics.keywords().double_strike());

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
    let opponent_artifact_destroyed = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: opponent_artifact,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(opponent_artifact)
        == Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard));

    let cleanup_reached = advance_to_cleanup(&mut state, controller);
    let expired_until_end_of_turn = state.last_cleanup_report().expired_until_end_of_turn();
    let restrictions_removed = state.restrictions().next().is_none();
    let continuous_effects_removed = state.continuous_effects().next().is_none();
    let double_strike_expired = state
        .creature_characteristics(opponent_creature)
        .is_ok_and(|characteristics| !characteristics.keywords().double_strike());
    let controlled_creature_destroyed_after_cleanup = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: controlled_creature,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(controlled_creature)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));
    let controlled_artifact_destroyed_after_cleanup = matches!(
        apply(
            &mut state,
            Action::DestroyPermanent {
                object: controlled_artifact,
            },
        ),
        Outcome::Applied
    ) && state.object_zone(controlled_artifact)
        == Some(ZoneId::new(Some(controller), ZoneKind::Graveyard));

    Some(json!({
        "setup_succeeded": true,
        "contract": {
            "mode_count": modes.len(),
            "target_counts": target_counts,
            "modes_have_no_object_choices": modes_have_no_object_choices,
            "modes_have_no_optional_choices": modes_have_no_optional_choices,
            "damage_program_exact": damage_program_exact,
            "indestructible_program_exact": indestructible_program_exact,
            "double_strike_program_exact": double_strike_program_exact,
            "no_mode_rejected_before_mutation": no_mode_rejected_before_mutation,
            "out_of_range_mode_rejected_before_mutation":
                out_of_range_mode_rejected_before_mutation,
            "targetless_mode_rejects_extra_target": targetless_mode_rejects_extra_target,
        },
        "damage": {
            "requirement_is_player_or_permanent": damage_requirement_is_player_or_permanent,
            "player_target_accepted": player_target_accepted,
            "player_bound_action_exact": player_bound_action_exact,
            "player_action_applied": player_action_applied,
            "opponent_life_after_damage": opponent_life_after_damage,
            "planeswalker_target_accepted": planeswalker_target_accepted,
            "planeswalker_bound_action_exact": planeswalker_bound_action_exact,
            "planeswalker_action_applied": planeswalker_action_applied,
            "planeswalker_loyalty_after_damage": planeswalker_loyalty_after_damage,
            "nonplaneswalker_permanent_rejected_before_mutation":
                nonplaneswalker_permanent_rejected_before_mutation,
        },
        "indestructible": {
            "bound_action_count": indestructible_bound_action_count,
            "bound_actions_are_restrictions": indestructible_actions_are_restrictions,
            "all_actions_applied": indestructible_actions_applied,
            "controlled_creature_protected": controlled_creature_protected,
            "controlled_artifact_protected": controlled_artifact_protected,
            "opponent_creature_unprotected": opponent_creature_unprotected,
            "opponent_artifact_unprotected": opponent_artifact_unprotected,
            "protected_creature_survived_lethal_damage":
                protected_creature_survived_lethal_damage,
            "protected_artifact_survived_destroy": protected_artifact_survived_destroy,
            "opponent_artifact_destroyed": opponent_artifact_destroyed,
        },
        "double_strike": {
            "controlled_creature_target_accepted": controlled_creature_target_accepted,
            "opponent_creature_target_accepted": opponent_creature_target_accepted,
            "noncreature_target_rejected_before_mutation":
                noncreature_target_rejected_before_mutation,
            "bound_action_count": double_strike_bound_action_count,
            "bound_action_is_continuous_effect": double_strike_action_is_continuous_effect,
            "all_actions_applied": double_strike_actions_applied,
            "opponent_creature_has_double_strike": opponent_creature_has_double_strike,
        },
        "cleanup": {
            "reached": cleanup_reached,
            "expired_until_end_of_turn": expired_until_end_of_turn,
            "restrictions_removed": restrictions_removed,
            "continuous_effects_removed": continuous_effects_removed,
            "double_strike_expired": double_strike_expired,
            "controlled_creature_destroyed_after_cleanup":
                controlled_creature_destroyed_after_cleanup,
            "controlled_artifact_destroyed_after_cleanup":
                controlled_artifact_destroyed_after_cleanup,
        },
    }))
}

fn reconnaissance_mission_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.name() != "Reconnaissance Mission" {
        return None;
    }
    let cycling = program.cycling()?;
    let [trigger] = program.triggered_abilities() else {
        return Some(json!({
            "setup_succeeded": false,
            "observed_trigger_count": program.triggered_abilities().len(),
        }));
    };
    let cycling_cost = cycling.mana_cost();
    let cycling_payment = cycling.exact_payment();
    let cycling_contract_exact = cycling_cost.base_generic() == 2
        && cycling_cost.colored_pool().total() == 0
        && cycling_payment.total() == 2;
    let trigger_event_exact = matches!(
        trigger.event(),
        TriggeredEventProgram::ControllerPermanentDealsCombatDamageToPlayer(predicate)
            if predicate.controller() == TargetControllerPredicate::You
                && predicate.required_types() == ObjectTypes::none().with_creature()
    );
    let trigger_effect_exact = matches!(
        trigger.effects(),
        [EffectProgram::DrawCards {
            players: PlayerBinding::Controller,
            count: AmountProgram::Literal(1),
        }]
    );
    let trigger_choice_contract_exact = trigger.optional_choice_count() == 1
        && trigger.target_requirements().is_empty()
        && trigger.object_choice_requirements().is_empty();

    let mut cycle_state = GameState::new();
    let cycle_controller = match apply(&mut cycle_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let _cycle_opponent = match apply(&mut cycle_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let hand = ZoneId::new(Some(cycle_controller), ZoneKind::Hand);
    let graveyard = ZoneId::new(Some(cycle_controller), ZoneKind::Graveyard);
    let library = ZoneId::new(Some(cycle_controller), ZoneKind::Library);
    let cycling_source = create_probe_object(
        &mut cycle_state,
        9_670_000,
        cycle_controller,
        cycle_controller,
        hand,
        program.base_object(),
        program.base_creature(),
    )?;
    let wrong_zone_copy = create_probe_object(
        &mut cycle_state,
        9_670_001,
        cycle_controller,
        cycle_controller,
        graveyard,
        program.base_object(),
        program.base_creature(),
    )?;
    let draw_card = create_probe_object(
        &mut cycle_state,
        9_670_002,
        cycle_controller,
        cycle_controller,
        library,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let cycling_priority_ready = matches!(
        apply(
            &mut cycle_state,
            Action::StartTurn {
                active_player: cycle_controller,
            },
        ),
        Outcome::Applied
    ) && matches!(
        apply(&mut cycle_state, Action::AdvanceStep),
        Outcome::StepAdvanced(Step::Upkeep)
    ) && cycle_state.priority_player() == Some(cycle_controller);
    let payment = auto_payment_plan(cycling_payment, cycling_cost)
        .ok()
        .flatten()?;
    let before_wrong_zone = cycle_state.deterministic_hash();
    let wrong_zone_rejected_before_mutation = matches!(
        apply(
            &mut cycle_state,
            Action::Cycle {
                player: cycle_controller,
                object: wrong_zone_copy,
                cost: cycling_cost,
                payment,
            },
        ),
        Outcome::Failed(StateError::ObjectNotCastable(object)) if object == wrong_zone_copy
    ) && cycle_state.deterministic_hash()
        == before_wrong_zone;
    let before_unfunded = cycle_state.deterministic_hash();
    let unfunded_cycle_rejected_before_mutation = matches!(
        apply(
            &mut cycle_state,
            Action::Cycle {
                player: cycle_controller,
                object: cycling_source,
                cost: cycling_cost,
                payment,
            },
        ),
        Outcome::Failed(StateError::InsufficientMana)
    ) && cycle_state.deterministic_hash()
        == before_unfunded;
    let cycling_funded = matches!(
        apply(
            &mut cycle_state,
            Action::AddManaToPool {
                player: cycle_controller,
                mana: cycling_payment,
            },
        ),
        Outcome::Applied
    );
    let hand_before_cycle = cycle_state.zone_objects(hand)?.len();
    let library_before_cycle = cycle_state.zone_objects(library)?.len();
    let cycling_action_applied = matches!(
        apply(
            &mut cycle_state,
            Action::Cycle {
                player: cycle_controller,
                object: cycling_source,
                cost: cycling_cost,
                payment,
            },
        ),
        Outcome::Applied
    );
    let cycling_source_discarded = cycle_state.object_zone(cycling_source) == Some(graveyard);
    let cycling_drew_exactly_one = cycle_state.zone_objects(hand)?.len() == hand_before_cycle
        && cycle_state.zone_objects(library)?.len() + 1 == library_before_cycle
        && cycle_state.object_zone(draw_card) == Some(hand);
    let cycling_payment_consumed =
        cycle_state.mana_pool(cycle_controller).ok() == Some(ManaPool::empty());

    let mut trigger_state = GameState::new();
    let controller = match apply(&mut trigger_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut trigger_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let battlefield = ZoneId::new(None, ZoneKind::Battlefield);
    let source = create_probe_object(
        &mut trigger_state,
        9_670_100,
        controller,
        controller,
        battlefield,
        program.base_object(),
        program.base_creature(),
    )?;
    let attacker = create_probe_object(
        &mut trigger_state,
        9_670_101,
        controller,
        controller,
        battlefield,
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none()),
        Some(
            BaseCreatureCharacteristics::new(1, 1)
                .with_keywords(CreatureKeywords::none().with_haste()),
        ),
    )?;
    let definition = trigger.bind(controller, source);
    let trigger_definition_exact = definition.source() == Some(source)
        && matches!(
            definition.condition(),
            TriggerCondition::CombatDamageToPlayer { source: predicate }
                if predicate.controller() == TargetControllerPredicate::You
                    && predicate.required_types() == ObjectTypes::none().with_creature()
        );
    let registered_trigger = match apply(
        &mut trigger_state,
        Action::RegisterTriggeredAbility { definition },
    ) {
        Outcome::TriggerRegistered(trigger) => Some(trigger),
        _ => None,
    };
    let combat_window_ready = advance_to_declare_attackers(&mut trigger_state, controller);
    let trigger_draw_card = create_probe_object(
        &mut trigger_state,
        9_670_102,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Library),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let attacker_declared = matches!(
        apply(
            &mut trigger_state,
            Action::DeclareAttackers {
                player: controller,
                attacks: vec![AttackDeclaration::new(attacker, opponent)],
            },
        ),
        Outcome::Applied
    );
    let blockers_step_reached = matches!(
        apply(&mut trigger_state, Action::AdvanceStep),
        Outcome::StepAdvanced(Step::DeclareBlockers)
    );
    let no_blockers_declared = matches!(
        apply(
            &mut trigger_state,
            Action::DeclareBlockers {
                defending_player: opponent,
                blocks: Vec::new(),
            },
        ),
        Outcome::Applied
    );
    let combat_damage_step_reached = matches!(
        apply(&mut trigger_state, Action::AdvanceStep),
        Outcome::StepAdvanced(Step::CombatDamage)
    );
    let combat_damage_assigned = matches!(
        apply(
            &mut trigger_state,
            Action::AssignCombatDamage {
                assignments: vec![CombatDamageAssignmentRequest::new(
                    attacker,
                    vec![CombatDamageAssignment::new(
                        CombatDamageTarget::Player(opponent),
                        1,
                    )],
                )],
            },
        ),
        Outcome::CombatDamageAssigned(records) if records.len() == 1
    );
    let pending_trigger_exact = registered_trigger.is_some_and(|registered| {
        trigger_state.pending_triggers().len() == 1
            && trigger_state.pending_triggers()[0].trigger() == registered
            && trigger_state.pending_triggers()[0].controller() == controller
            && trigger_state.pending_triggers()[0].source() == Some(source)
    });
    let trigger_entries = match apply(
        &mut trigger_state,
        Action::PutPendingTriggeredAbilitiesOnStack,
    ) {
        Outcome::StackEntriesAdded(entries) => entries,
        _ => Vec::new(),
    };
    let trigger_put_on_stack = registered_trigger.is_some_and(|registered| {
        trigger_entries.len() == 1
            && trigger_state.stack_top().is_some_and(|entry| {
                entry.id() == trigger_entries[0]
                    && entry.controller() == controller
                    && entry.trigger() == Some(registered)
            })
    });
    let choice_bindings = |execute| {
        ExecutionBindings::new(controller, vec![opponent])
            .with_source(source)
            .with_optional_effect_choices(vec![execute])
    };
    let before_missing_choice = trigger_state.deterministic_hash();
    let missing_optional_choice_rejected_before_mutation = matches!(
        bind_triggered_ability_actions(
            &trigger_state,
            trigger,
            &ExecutionBindings::new(controller, vec![opponent]).with_source(source),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::MissingChoice
    ) && trigger_state.deterministic_hash()
        == before_missing_choice;
    let hand_before_choice = trigger_state
        .zone_objects(ZoneId::new(Some(controller), ZoneKind::Hand))?
        .len();
    let before_decline = trigger_state.deterministic_hash();
    let declined_actions =
        bind_triggered_ability_actions(&trigger_state, trigger, &choice_bindings(false)).ok()?;
    let decline_emits_no_actions = declined_actions.is_empty()
        && trigger_state.deterministic_hash() == before_decline
        && trigger_state
            .zone_objects(ZoneId::new(Some(controller), ZoneKind::Hand))?
            .len()
            == hand_before_choice;
    let accepted_actions =
        bind_triggered_ability_actions(&trigger_state, trigger, &choice_bindings(true)).ok()?;
    let accepted_draw_action_exact = accepted_actions.len() == 1
        && matches!(
            accepted_actions[0].action(),
            Action::DrawCards { player, count: 1 } if *player == controller
        );
    let trigger_stack_resolved = trigger_entries
        .first()
        .is_some_and(|entry| resolve_expected_stack_entry(&mut trigger_state, *entry));
    let library_before_draw = trigger_state
        .zone_objects(ZoneId::new(Some(controller), ZoneKind::Library))?
        .len();
    let optional_draw_applied = accepted_actions.iter().all(|bound| {
        matches!(
            apply(&mut trigger_state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let optional_draw_exactly_one = trigger_state
        .zone_objects(ZoneId::new(Some(controller), ZoneKind::Hand))?
        .len()
        == hand_before_choice + 1
        && trigger_state
            .zone_objects(ZoneId::new(Some(controller), ZoneKind::Library))?
            .len()
            + 1
            == library_before_draw
        && trigger_state.object_zone(trigger_draw_card)
            == Some(ZoneId::new(Some(controller), ZoneKind::Hand));

    Some(json!({
        "setup_succeeded": true,
        "contract": {
            "cycling_present": true,
            "cycling_cost_exact": cycling_contract_exact,
            "trigger_count": program.triggered_abilities().len(),
            "trigger_event_exact": trigger_event_exact,
            "trigger_effect_exact": trigger_effect_exact,
            "trigger_choice_contract_exact": trigger_choice_contract_exact,
        },
        "cycling": {
            "priority_ready": cycling_priority_ready,
            "generic_mana_cost": cycling_cost.base_generic(),
            "exact_payment_total": cycling_payment.total(),
            "wrong_zone_rejected_before_mutation": wrong_zone_rejected_before_mutation,
            "unfunded_rejected_before_mutation": unfunded_cycle_rejected_before_mutation,
            "funded": cycling_funded,
            "action_applied": cycling_action_applied,
            "payment_consumed": cycling_payment_consumed,
            "source_discarded_to_owner_graveyard": cycling_source_discarded,
            "drew_exactly_one": cycling_drew_exactly_one,
        },
        "combat_trigger": {
            "definition_exact": trigger_definition_exact,
            "registered": registered_trigger.is_some(),
            "combat_window_ready": combat_window_ready,
            "attacker_declared": attacker_declared,
            "blockers_step_reached": blockers_step_reached,
            "no_blockers_declared": no_blockers_declared,
            "combat_damage_step_reached": combat_damage_step_reached,
            "combat_damage_assigned": combat_damage_assigned,
            "opponent_life_after_damage": trigger_state.players()[opponent.index()].life(),
            "pending_trigger_exact": pending_trigger_exact,
            "put_on_stack": trigger_put_on_stack,
        },
        "optional_draw": {
            "missing_choice_rejected_before_mutation":
                missing_optional_choice_rejected_before_mutation,
            "decline_emits_no_actions_or_draw": decline_emits_no_actions,
            "accept_bound_action_count": accepted_actions.len(),
            "accept_draw_action_exact": accepted_draw_action_exact,
            "trigger_stack_resolved": trigger_stack_resolved,
            "draw_action_applied": optional_draw_applied,
            "drew_exactly_one": optional_draw_exactly_one,
        },
    }))
}

fn matching_artifact_token_count(
    state: &GameState,
    controller: forge_core::PlayerId,
    card: CardId,
) -> usize {
    state
        .zone_objects(ZoneId::new(None, ZoneKind::Battlefield))
        .map_or(0, |objects| {
            objects
                .iter()
                .filter(|object| {
                    state.object(**object).is_some_and(|record| {
                        record.is_token()
                            && record.card() == card
                            && record.owner() == controller
                            && record.controller() == controller
                            && record.base_object().types() == ObjectTypes::none().with_artifact()
                            && record.base_creature().is_none()
                    })
                })
                .count()
        })
}

fn smothering_tithe_branch_probe(
    program: &CardProgram,
    pay: bool,
    salt: u32,
) -> Option<serde_json::Value> {
    let [ability] = program.triggered_abilities() else {
        return Some(json!({"setup_succeeded": false}));
    };
    let [EffectProgram::CreateTokens {
        card: token_card,
        base_object: token_base,
        base_creature: token_creature,
        count: AmountProgram::Literal(1),
        players: PlayerBinding::Controller,
        ..
    }] = ability.effects()
    else {
        return Some(json!({"setup_succeeded": false}));
    };
    let unless_paid = ability.unless_paid()?;

    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source = create_probe_object(
        &mut state,
        salt,
        controller,
        controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        program.base_object(),
        program.base_creature(),
    )?;
    create_probe_object(
        &mut state,
        salt.wrapping_add(1),
        opponent,
        opponent,
        ZoneId::new(Some(opponent), ZoneKind::Library),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let turn_started = matches!(
        apply(
            &mut state,
            Action::StartTurn {
                active_player: controller,
            },
        ),
        Outcome::Applied
    );
    let definition = ability.bind(controller, source);
    let registered = match apply(&mut state, Action::RegisterTriggeredAbility { definition }) {
        Outcome::TriggerRegistered(trigger) => Some(trigger),
        _ => None,
    };
    let draw_applied = matches!(
        apply(
            &mut state,
            Action::DrawCards {
                player: opponent,
                count: 1,
            },
        ),
        Outcome::Applied
    );
    let pending_trigger_exact = registered.is_some_and(|trigger| {
        state.pending_triggers().len() == 1
            && state.pending_triggers()[0].trigger() == trigger
            && state.pending_triggers()[0].controller() == controller
            && state.pending_triggers()[0].source() == Some(source)
    });
    let entries = match apply(&mut state, Action::PutPendingTriggeredAbilitiesOnStack) {
        Outcome::StackEntriesAdded(entries) => entries,
        _ => Vec::new(),
    };
    let trigger_put_on_stack = registered.is_some_and(|trigger| {
        entries.len() == 1
            && state.stack_top().is_some_and(|entry| {
                entry.id() == entries[0]
                    && entry.controller() == controller
                    && entry.trigger() == Some(trigger)
            })
    });

    let base_bindings = ExecutionBindings::new(controller, vec![opponent]).with_source(source);
    let before_missing_player = state.deterministic_hash();
    let missing_triggering_player_rejected_before_mutation = matches!(
        bind_triggered_ability_actions(
            &state,
            ability,
            &base_bindings.clone().with_unless_payment(true),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::MissingBinding
    ) && state.deterministic_hash()
        == before_missing_player;
    let with_triggering_player = base_bindings.with_triggering_player(opponent);
    let before_missing_decision = state.deterministic_hash();
    let missing_payment_decision_rejected_before_mutation = matches!(
        bind_triggered_ability_actions(&state, ability, &with_triggering_player),
        Err(error) if error.code() == ExecutionDiagnosticCode::MissingChoice
    ) && state.deterministic_hash()
        == before_missing_decision;

    let mana_funded = !pay
        || matches!(
            apply(
                &mut state,
                Action::AddManaToPool {
                    player: opponent,
                    mana: unless_paid.exact_payment(),
                },
            ),
            Outcome::Applied
        );
    let payer_mana_before = state.mana_pool(opponent).ok()?.total();
    let controller_mana_before = state.mana_pool(controller).ok()?.total();
    let actions = bind_triggered_ability_actions(
        &state,
        ability,
        &with_triggering_player.with_unless_payment(pay),
    )
    .ok()?;
    let bound_action_count = actions.len();
    let decline_action_exact = !pay
        && actions.len() == 1
        && matches!(
            actions[0].action(),
            Action::CreateToken {
                card,
                owner,
                controller: token_controller,
                base_object,
                base,
            } if *card == *token_card
                && *owner == controller
                && *token_controller == controller
                && *base_object == *token_base
                && *base == *token_creature
        );
    let payment_action_exact = pay
        && actions.len() == 1
        && matches!(
            actions[0].action(),
            Action::PayMana { player, cost, .. }
                if *player == opponent && *cost == unless_paid.mana_cost()
        );
    let tokens_before = matching_artifact_token_count(&state, controller, *token_card);
    let stack_resolved = entries
        .first()
        .is_some_and(|entry| resolve_expected_stack_entry(&mut state, *entry));
    let outcome = actions
        .first()
        .map(|bound| apply(&mut state, bound.action().clone()));
    let action_applied = matches!(
        outcome,
        Some(Outcome::Applied) | Some(Outcome::ObjectCreated(_))
    );
    let created_token = match outcome {
        Some(Outcome::ObjectCreated(object)) => Some(object),
        _ => None,
    };
    let created_token_exact = created_token.is_some_and(|object| {
        state.object(object).is_some_and(|record| {
            record.is_token()
                && record.card() == *token_card
                && record.owner() == controller
                && record.controller() == controller
                && record.base_object() == *token_base
                && record.base_creature() == *token_creature
                && state.object_zone(object) == Some(ZoneId::new(None, ZoneKind::Battlefield))
        })
    });
    let tokens_after = matching_artifact_token_count(&state, controller, *token_card);
    let payer_mana_after = state.mana_pool(opponent).ok()?.total();
    let controller_mana_after = state.mana_pool(controller).ok()?.total();

    Some(json!({
        "setup_succeeded": true,
        "pay_selected": pay,
        "turn_started": turn_started,
        "registered": registered.is_some(),
        "draw_applied": draw_applied,
        "pending_trigger_exact": pending_trigger_exact,
        "put_on_stack": trigger_put_on_stack,
        "missing_triggering_player_rejected_before_mutation":
            missing_triggering_player_rejected_before_mutation,
        "missing_payment_decision_rejected_before_mutation":
            missing_payment_decision_rejected_before_mutation,
        "mana_funded": mana_funded,
        "bound_action_count": bound_action_count,
        "decline_create_action_exact": decline_action_exact,
        "payment_action_targets_triggering_opponent": payment_action_exact,
        "stack_resolved": stack_resolved,
        "action_applied": action_applied,
        "token_count_before": tokens_before,
        "token_count_after": tokens_after,
        "created_token_exact": created_token_exact,
        "exactly_one_treasure_created": !pay
            && created_token_exact
            && tokens_after == tokens_before + 1,
        "treasure_suppressed": pay && tokens_after == tokens_before,
        "payer_mana_before": payer_mana_before,
        "payer_mana_after": payer_mana_after,
        "payer_mana_consumed": pay
            && payer_mana_before == unless_paid.exact_payment().total()
            && payer_mana_after == 0,
        "controller_mana_unchanged": controller_mana_before == controller_mana_after,
    }))
}

fn smothering_tithe_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.name() != "Smothering Tithe" {
        return None;
    }
    let [ability] = program.triggered_abilities() else {
        return Some(json!({
            "setup_succeeded": false,
            "observed_trigger_count": program.triggered_abilities().len(),
        }));
    };
    let unless_paid = ability.unless_paid()?;
    let (
        effect_exact,
        artifact_token_template_exact,
        treasure_subtype_present,
        treasure_mana_ability_exact,
        treasure_mana_outputs,
    ) = match ability.effects() {
        [EffectProgram::CreateTokens {
            base_object,
            base_creature,
            mana_ability: Some(mana_ability),
            count: AmountProgram::Literal(1),
            players: PlayerBinding::Controller,
            ..
        }] => {
            let outputs = mana_ability
                .output_choices()
                .options()
                .iter()
                .copied()
                .map(mana_label)
                .collect::<Vec<_>>();
            (
                true,
                base_object.types() == ObjectTypes::none().with_artifact()
                    && base_object.colors() == ObjectColors::none()
                    && base_creature.is_none(),
                base_object.subtypes().as_slice().iter().any(|subtype| {
                    String::from_utf8_lossy(subtype.as_bytes()).eq_ignore_ascii_case("Treasure")
                }),
                mana_ability.cost().mana() == ManaCost::new(0, 0, 0, 0, 0, 0)
                    && mana_ability.cost().tap_source()
                    && mana_ability.cost().sacrifice_source()
                    && outputs == ["{W}", "{U}", "{B}", "{R}", "{G}"],
                outputs,
            )
        }
        _ => (false, false, false, false, Vec::new()),
    };

    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let first_opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let second_opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source = create_probe_object(
        &mut state,
        9_680_000,
        controller,
        controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        program.base_object(),
        program.base_creature(),
    )?;
    let definition = ability.bind(controller, source);
    let definition_exact = definition.source() == Some(source)
        && matches!(
            definition.condition(),
            TriggerCondition::PlayerDrewCard {
                player: TriggerPlayerFilter::OpponentOfController,
            }
        );
    let card_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none());
    create_probe_object(
        &mut state,
        9_680_001,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Library),
        card_base,
        None,
    )?;
    for card in 9_680_002..9_680_004 {
        create_probe_object(
            &mut state,
            card,
            first_opponent,
            first_opponent,
            ZoneId::new(Some(first_opponent), ZoneKind::Library),
            card_base,
            None,
        )?;
    }
    let turn_started = matches!(
        apply(
            &mut state,
            Action::StartTurn {
                active_player: controller,
            },
        ),
        Outcome::Applied
    );
    let registered = match apply(&mut state, Action::RegisterTriggeredAbility { definition }) {
        Outcome::TriggerRegistered(trigger) => Some(trigger),
        _ => None,
    };

    let controller_cursor = state.event_cursor();
    let controller_draw_applied = matches!(
        apply(
            &mut state,
            Action::DrawCards {
                player: controller,
                count: 1,
            },
        ),
        Outcome::Applied
    );
    let controller_card_draw_events = state
        .events_since(controller_cursor)
        .ok()?
        .iter()
        .filter(|record| {
            matches!(
                record.event(),
                GameEvent::CardDrawn { player, .. } if player == controller
            )
        })
        .count();
    let controller_draw_queued_no_trigger = state.pending_triggers().is_empty();

    let empty_cursor = state.event_cursor();
    let empty_library_draw_applied = matches!(
        apply(
            &mut state,
            Action::DrawCards {
                player: second_opponent,
                count: 1,
            },
        ),
        Outcome::Applied
    );
    let empty_events = state.events_since(empty_cursor).ok()?;
    let empty_library_events = empty_events
        .iter()
        .filter(|record| {
            matches!(
                record.event(),
                GameEvent::EmptyLibraryDraw { player } if player == second_opponent
            )
        })
        .count();
    let failed_draw_card_events = empty_events
        .iter()
        .filter(|record| {
            matches!(
                record.event(),
                GameEvent::CardDrawn { player, .. } if player == second_opponent
            )
        })
        .count();
    let empty_library_queued_no_trigger = state.pending_triggers().is_empty();

    let opponent_cursor = state.event_cursor();
    let opponent_draw_applied = matches!(
        apply(
            &mut state,
            Action::DrawCards {
                player: first_opponent,
                count: 2,
            },
        ),
        Outcome::Applied
    );
    let opponent_card_draw_events = state
        .events_since(opponent_cursor)
        .ok()?
        .iter()
        .filter(|record| {
            matches!(
                record.event(),
                GameEvent::CardDrawn { player, .. } if player == first_opponent
            )
        })
        .count();
    let pending_count = state.pending_triggers().len();
    let pending_triggers_exact = registered.is_some_and(|trigger| {
        state.pending_triggers().iter().all(|pending| {
            pending.trigger() == trigger
                && pending.controller() == controller
                && pending.source() == Some(source)
        })
    });
    let pending_event_sequences_distinct = state.pending_triggers().len() == 2
        && state.pending_triggers()[0].event_sequence()
            != state.pending_triggers()[1].event_sequence();
    let stack_entries = match apply(&mut state, Action::PutPendingTriggeredAbilitiesOnStack) {
        Outcome::StackEntriesAdded(entries) => entries,
        _ => Vec::new(),
    };
    let stack_entries_exact = registered.is_some_and(|trigger| {
        stack_entries.len() == 2
            && stack_entries.iter().all(|entry| {
                state.stack_entries().iter().any(|candidate| {
                    candidate.id() == *entry
                        && candidate.controller() == controller
                        && candidate.trigger() == Some(trigger)
                })
            })
    });

    Some(json!({
        "setup_succeeded": true,
        "contract": {
            "ability_count": program.triggered_abilities().len(),
            "event_exact": ability.event() == TriggeredEventProgram::OpponentDrawsCard,
            "definition_exact": definition_exact,
            "payer_is_triggering_opponent":
                unless_paid.payer() == PlayerBinding::TriggeringPlayer,
            "generic_mana_cost": unless_paid.mana_cost().base_generic(),
            "colored_mana_cost": unless_paid.mana_cost().colored_pool().total(),
            "exact_payment_total": unless_paid.exact_payment().total(),
            "effect_exact": effect_exact,
            "artifact_token_template_exact": artifact_token_template_exact,
            "treasure_subtype_present": treasure_subtype_present,
            "treasure_mana_ability_exact": treasure_mana_ability_exact,
            "treasure_mana_outputs": treasure_mana_outputs,
            "no_targets_or_choices": ability.target_requirements().is_empty()
                && ability.object_choice_requirements().is_empty()
                && ability.optional_choice_count() == 0,
        },
        "event_boundary": {
            "turn_started": turn_started,
            "registered": registered.is_some(),
            "controller_draw_applied": controller_draw_applied,
            "controller_card_draw_event_count": controller_card_draw_events,
            "controller_draw_queued_no_trigger": controller_draw_queued_no_trigger,
            "empty_library_draw_applied": empty_library_draw_applied,
            "empty_library_event_count": empty_library_events,
            "failed_draw_card_event_count": failed_draw_card_events,
            "empty_library_queued_no_trigger": empty_library_queued_no_trigger,
            "opponent_draw_applied": opponent_draw_applied,
            "opponent_card_draw_event_count": opponent_card_draw_events,
            "pending_trigger_count": pending_count,
            "one_trigger_per_opponent_card_drawn": pending_count == opponent_card_draw_events,
            "pending_triggers_exact": pending_triggers_exact,
            "pending_event_sequences_distinct": pending_event_sequences_distinct,
            "put_on_stack_count": stack_entries.len(),
            "stack_entries_exact": stack_entries_exact,
        },
        "decline": smothering_tithe_branch_probe(program, false, 9_681_000),
        "pay": smothering_tithe_branch_probe(program, true, 9_682_000),
    }))
}

fn purphoros_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.name() != "Purphoros, God of the Forge" {
        return None;
    }
    let [static_ability] = program.static_abilities() else {
        return Some(json!({
            "setup_succeeded": false,
            "observed_static_count": program.static_abilities().len(),
        }));
    };
    let [trigger] = program.triggered_abilities() else {
        return Some(json!({
            "setup_succeeded": false,
            "observed_trigger_count": program.triggered_abilities().len(),
        }));
    };
    let [activated] = program.activated_effects() else {
        return Some(json!({
            "setup_succeeded": false,
            "observed_activated_count": program.activated_effects().len(),
        }));
    };

    let static_program_exact = matches!(
        static_ability,
        StaticAbilityProgram::DevotionSourceTypeRemoval {
            color: ManaKind::Red,
            threshold: 5,
            types,
        } if *types == ObjectTypes::none().with_creature()
    );
    let trigger_program_exact = matches!(
        trigger.event(),
        TriggeredEventProgram::ControllerPermanentEnters {
            predicate,
            exclude_source: true,
        } if predicate.controller() == TargetControllerPredicate::You
            && predicate.required_types() == ObjectTypes::none().with_creature()
    ) && matches!(
        trigger.effects(),
        [EffectProgram::DealDamageToPlayers {
            players: PlayerBinding::Opponents,
            amount: AmountProgram::Literal(2),
        }]
    ) && trigger.target_requirements().is_empty()
        && trigger.object_choice_requirements().is_empty()
        && trigger.optional_choice_count() == 0;
    let activated_program_exact = activated.mana_cost().base_generic() == 2
        && activated.mana_cost().colored_pool() == ManaPool::of(ManaKind::Red, 1)
        && activated.exact_payment().total() == 3
        && activated.timing() == ActivationTiming::Instant
        && !activated.tap_source()
        && !activated.sacrifice_source()
        && activated.pay_life() == 0
        && activated.sacrifice_cost().is_none()
        && activated.target_requirements().is_empty()
        && activated.object_choice_requirements().is_empty()
        && activated.optional_choice_count() == 0
        && matches!(
            activated.effects(),
            [EffectProgram::ModifyPowerToughness {
                objects: ObjectSetProgram::Battlefield(predicate),
                power: AmountProgram::Literal(1),
                toughness: AmountProgram::Literal(0),
                duration: ContinuousEffectDuration::UntilEndOfTurn,
            }] if predicate.controller() == TargetControllerPredicate::You
                && predicate.required_types() == ObjectTypes::none().with_creature()
        );

    let mut devotion_state = GameState::new();
    let devotion_controller = match apply(&mut devotion_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let devotion_opponent = match apply(&mut devotion_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let devotion_source = create_probe_object(
        &mut devotion_state,
        9_690_000,
        devotion_controller,
        devotion_controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        program.base_object(),
        program.base_creature(),
    )?;
    let static_actions = static_ability.bind_actions(devotion_controller, devotion_source);
    let static_definition_exact = matches!(
        static_actions.as_slice(),
        [Action::RegisterContinuousEffect { definition }]
            if definition.source() == Some(devotion_source)
                && definition.target() == ContinuousEffectTarget::Object(devotion_source)
                && definition.operation()
                    == ContinuousEffectOperation::RemoveTypes {
                        types: ObjectTypes::none().with_creature(),
                    }
                && definition.duration()
                    == ContinuousEffectDuration::WhileSourceOnBattlefield
                && definition.condition()
                    == ContinuousEffectCondition::ControllerDevotionLessThan {
                        color: ManaKind::Red,
                        threshold: 5,
                    }
    );
    let static_registered = static_actions.iter().all(|action| {
        matches!(
            apply(&mut devotion_state, action.clone()),
            Outcome::ContinuousEffectRegistered(_)
        )
    });
    let devotion_low = devotion_state
        .controller_devotion(devotion_controller, ManaKind::Red)
        .ok()?;
    let source_noncreature_at_one = devotion_state
        .object_characteristics(devotion_source)
        .is_ok_and(|characteristics| !characteristics.types().creature())
        && matches!(
            devotion_state.creature_characteristics(devotion_source),
            Err(StateError::NotACreature(object)) if object == devotion_source
        );
    create_probe_object(
        &mut devotion_state,
        9_690_001,
        devotion_opponent,
        devotion_opponent,
        ZoneId::new(None, ZoneKind::Battlefield),
        BaseObjectCharacteristics::new(
            ObjectTypes::none().with_enchantment(),
            ObjectColors::none(),
        )
        .with_printed_mana_symbols(ManaPool::of(ManaKind::Red, 10)),
        None,
    )?;
    let opponent_symbols_ignored = devotion_state
        .controller_devotion(devotion_controller, ManaKind::Red)
        .ok()
        == Some(1)
        && devotion_state
            .object_characteristics(devotion_source)
            .is_ok_and(|characteristics| !characteristics.types().creature());
    let devotion_anchor = create_probe_object(
        &mut devotion_state,
        9_690_002,
        devotion_controller,
        devotion_controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        BaseObjectCharacteristics::new(
            ObjectTypes::none().with_enchantment(),
            ObjectColors::none(),
        )
        .with_printed_mana_symbols(ManaPool::of(ManaKind::Red, 4)),
        None,
    )?;
    let devotion_high = devotion_state
        .controller_devotion(devotion_controller, ManaKind::Red)
        .ok()?;
    let high_characteristics = devotion_state
        .creature_characteristics(devotion_source)
        .ok()?;
    let source_creature_at_five = devotion_state
        .object_characteristics(devotion_source)
        .is_ok_and(|characteristics| characteristics.types().creature())
        && high_characteristics.power() == 6
        && high_characteristics.toughness() == 5
        && high_characteristics.keywords().indestructible();
    let anchor_removed = matches!(
        apply(
            &mut devotion_state,
            Action::MoveObject {
                object: devotion_anchor,
                to: ZoneId::new(Some(devotion_controller), ZoneKind::Graveyard),
            },
        ),
        Outcome::Applied
    );
    let devotion_low_again = devotion_state
        .controller_devotion(devotion_controller, ManaKind::Red)
        .ok()?;
    let source_noncreature_after_drop = devotion_state
        .object_characteristics(devotion_source)
        .is_ok_and(|characteristics| !characteristics.types().creature())
        && matches!(
            devotion_state.creature_characteristics(devotion_source),
            Err(StateError::NotACreature(object)) if object == devotion_source
        );

    let mut trigger_state = GameState::new();
    let controller = match apply(&mut trigger_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let first_opponent = match apply(&mut trigger_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let second_opponent = match apply(&mut trigger_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source = create_probe_object(
        &mut trigger_state,
        9_691_000,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Graveyard),
        program.base_object(),
        program.base_creature(),
    )?;
    let creature_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_creature(), ObjectColors::none());
    let opponent_creature = create_probe_object(
        &mut trigger_state,
        9_691_001,
        first_opponent,
        first_opponent,
        ZoneId::new(Some(first_opponent), ZoneKind::Hand),
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let controlled_noncreature = create_probe_object(
        &mut trigger_state,
        9_691_002,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Hand),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let controlled_creature = create_probe_object(
        &mut trigger_state,
        9_691_003,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Hand),
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let trigger_turn_started = matches!(
        apply(
            &mut trigger_state,
            Action::StartTurn {
                active_player: controller,
            },
        ),
        Outcome::Applied
    );
    let trigger_definition = trigger.bind(controller, source);
    let trigger_definition_exact = trigger_definition.source() == Some(source)
        && matches!(
            trigger_definition.condition(),
            TriggerCondition::PermanentEnteredBattlefield {
                predicate,
                exclude_source: true,
            } if predicate.controller() == TargetControllerPredicate::You
                && predicate.required_types() == ObjectTypes::none().with_creature()
        );
    let registered_trigger = match apply(
        &mut trigger_state,
        Action::RegisterTriggeredAbility {
            definition: trigger_definition,
        },
    ) {
        Outcome::TriggerRegistered(trigger) => Some(trigger),
        _ => None,
    };
    let source_entered = matches!(
        apply(
            &mut trigger_state,
            Action::MoveObject {
                object: source,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ),
        Outcome::Applied
    );
    let self_entry_excluded = trigger_state.pending_triggers().is_empty();
    let opponent_creature_entered = matches!(
        apply(
            &mut trigger_state,
            Action::MoveObject {
                object: opponent_creature,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ),
        Outcome::Applied
    );
    let opponent_creature_excluded = trigger_state.pending_triggers().is_empty();
    let controlled_noncreature_entered = matches!(
        apply(
            &mut trigger_state,
            Action::MoveObject {
                object: controlled_noncreature,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ),
        Outcome::Applied
    );
    let controlled_noncreature_excluded = trigger_state.pending_triggers().is_empty();
    let controlled_creature_entered = matches!(
        apply(
            &mut trigger_state,
            Action::MoveObject {
                object: controlled_creature,
                to: ZoneId::new(None, ZoneKind::Battlefield),
            },
        ),
        Outcome::Applied
    );
    let pending_trigger_exact = registered_trigger.is_some_and(|registered| {
        trigger_state.pending_triggers().len() == 1
            && trigger_state.pending_triggers()[0].trigger() == registered
            && trigger_state.pending_triggers()[0].controller() == controller
            && trigger_state.pending_triggers()[0].source() == Some(source)
    });
    let trigger_entries = match apply(
        &mut trigger_state,
        Action::PutPendingTriggeredAbilitiesOnStack,
    ) {
        Outcome::StackEntriesAdded(entries) => entries,
        _ => Vec::new(),
    };
    let trigger_put_on_stack = registered_trigger.is_some_and(|registered| {
        trigger_entries.len() == 1
            && trigger_state.stack_top().is_some_and(|entry| {
                entry.id() == trigger_entries[0]
                    && entry.controller() == controller
                    && entry.trigger() == Some(registered)
            })
    });
    let damage_actions = bind_triggered_ability_actions(
        &trigger_state,
        trigger,
        &ExecutionBindings::new(controller, vec![first_opponent, second_opponent])
            .with_source(source),
    )
    .ok()?;
    let untargeted_contract =
        trigger.target_requirements().is_empty() && trigger.object_choice_requirements().is_empty();
    let exact_damage_actions = damage_actions.len() == 2
        && [first_opponent, second_opponent]
            .iter()
            .zip(damage_actions.iter())
            .all(|(opponent, bound)| {
                matches!(
                    bound.action(),
                    Action::DealDamage {
                        source: Some(bound_source),
                        target: CombatDamageTarget::Player(player),
                        amount: 2,
                    } if *bound_source == source && *player == *opponent
                )
            });
    let trigger_stack_resolved = trigger_entries
        .first()
        .is_some_and(|entry| resolve_expected_stack_entry(&mut trigger_state, *entry));
    let damage_cursor = trigger_state.event_cursor();
    let damage_actions_applied = damage_actions.iter().all(|bound| {
        matches!(
            apply(&mut trigger_state, bound.action().clone()),
            Outcome::Applied
        )
    });
    let damage_events = trigger_state
        .events_since(damage_cursor)
        .ok()?
        .iter()
        .filter(|record| {
            matches!(
                record.event(),
                GameEvent::NoncombatDamageDealt {
                    source: Some(event_source),
                    target: CombatDamageTarget::Player(player),
                    amount: 2,
                } if event_source == source
                    && (player == first_opponent || player == second_opponent)
            )
        })
        .count();

    let mut pump_state = GameState::new();
    let pump_controller = match apply(&mut pump_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let pump_opponent = match apply(&mut pump_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let pump_source = create_probe_object(
        &mut pump_state,
        9_692_000,
        pump_controller,
        pump_controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        program.base_object(),
        program.base_creature(),
    )?;
    let pumped_creature = create_probe_object(
        &mut pump_state,
        9_692_001,
        pump_controller,
        pump_controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        creature_base,
        Some(BaseCreatureCharacteristics::new(2, 2)),
    )?;
    let controlled_artifact = create_probe_object(
        &mut pump_state,
        9_692_002,
        pump_controller,
        pump_controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none()),
        None,
    )?;
    let opponent_pump_creature = create_probe_object(
        &mut pump_state,
        9_692_003,
        pump_opponent,
        pump_opponent,
        ZoneId::new(None, ZoneKind::Battlefield),
        creature_base,
        Some(BaseCreatureCharacteristics::new(3, 3)),
    )?;
    let pump_static_registered = static_ability
        .bind_actions(pump_controller, pump_source)
        .iter()
        .all(|action| {
            matches!(
                apply(&mut pump_state, action.clone()),
                Outcome::ContinuousEffectRegistered(_)
            )
        });
    let source_noncreature_before_pump = pump_state
        .object_characteristics(pump_source)
        .is_ok_and(|characteristics| !characteristics.types().creature());
    let pump_actions = bind_activated_effect_actions(
        &pump_state,
        activated,
        &ExecutionBindings::new(pump_controller, vec![pump_opponent]).with_source(pump_source),
    )
    .ok()?;
    let pump_action_exact = matches!(
        pump_actions.as_slice(),
        [bound] if matches!(
            bound.action(),
            Action::RegisterContinuousEffect { definition }
                if definition.target() == ContinuousEffectTarget::Object(pumped_creature)
                    && definition.operation()
                        == ContinuousEffectOperation::ModifyPowerToughness {
                            power: 1,
                            toughness: 0,
                        }
                    && definition.duration() == ContinuousEffectDuration::UntilEndOfTurn
        )
    );
    let pump_funded = matches!(
        apply(
            &mut pump_state,
            Action::AddManaToPool {
                player: pump_controller,
                mana: activated.exact_payment(),
            },
        ),
        Outcome::Applied
    );
    let pump_payment = auto_payment_plan(activated.exact_payment(), activated.mana_cost())
        .ok()
        .flatten()?;
    let pump_paid = matches!(
        apply(
            &mut pump_state,
            Action::PayMana {
                player: pump_controller,
                cost: activated.mana_cost(),
                plan: pump_payment,
            },
        ),
        Outcome::Applied
    );
    let pump_payment_consumed =
        pump_state.mana_pool(pump_controller).ok() == Some(ManaPool::empty());
    let pump_actions_applied = pump_actions.iter().all(|bound| {
        matches!(
            apply(&mut pump_state, bound.action().clone()),
            Outcome::ContinuousEffectRegistered(_)
        )
    });
    let pumped_characteristics = pump_state.creature_characteristics(pumped_creature).ok()?;
    let opponent_characteristics = pump_state
        .creature_characteristics(opponent_pump_creature)
        .ok()?;
    let controlled_creature_got_plus_one_zero =
        pumped_characteristics.power() == 3 && pumped_characteristics.toughness() == 2;
    let opponent_creature_unchanged =
        opponent_characteristics.power() == 3 && opponent_characteristics.toughness() == 3;
    let noncreature_unchanged = pump_state
        .object_characteristics(controlled_artifact)
        .is_ok_and(|characteristics| !characteristics.types().creature());
    let cleanup_reached = advance_to_cleanup(&mut pump_state, pump_controller);
    let expired_until_end_of_turn = pump_state.last_cleanup_report().expired_until_end_of_turn();
    let post_cleanup = pump_state.creature_characteristics(pumped_creature).ok()?;
    let pump_expired_at_cleanup = post_cleanup.power() == 2 && post_cleanup.toughness() == 2;

    Some(json!({
        "setup_succeeded": true,
        "contract": {
            "printed_red_symbols": program
                .base_object()
                .printed_mana_symbols()
                .get(ManaKind::Red),
            "printed_types_include_enchantment_and_creature":
                program.base_object().types().enchantment()
                    && program.base_object().types().creature(),
            "printed_subtype_is_god": program
                .base_object()
                .subtypes()
                .as_slice()
                .iter()
                .any(|subtype| String::from_utf8_lossy(subtype.as_bytes())
                    .eq_ignore_ascii_case("God")),
            "printed_indestructible": program
                .base_creature()
                .is_some_and(|base| base.keywords().indestructible()),
            "static_program_exact": static_program_exact,
            "trigger_program_exact": trigger_program_exact,
            "activated_program_exact": activated_program_exact,
        },
        "devotion": {
            "static_definition_exact": static_definition_exact,
            "static_registered": static_registered,
            "low": devotion_low,
            "source_noncreature_at_one": source_noncreature_at_one,
            "opponent_symbols_ignored": opponent_symbols_ignored,
            "high": devotion_high,
            "source_creature_at_five": source_creature_at_five,
            "anchor_removed": anchor_removed,
            "low_again": devotion_low_again,
            "source_noncreature_after_drop": source_noncreature_after_drop,
        },
        "creature_enter_trigger": {
            "turn_started": trigger_turn_started,
            "definition_exact": trigger_definition_exact,
            "registered": registered_trigger.is_some(),
            "source_entered": source_entered,
            "self_entry_excluded": self_entry_excluded,
            "opponent_creature_entered": opponent_creature_entered,
            "opponent_creature_excluded": opponent_creature_excluded,
            "controlled_noncreature_entered": controlled_noncreature_entered,
            "controlled_noncreature_excluded": controlled_noncreature_excluded,
            "controlled_creature_entered": controlled_creature_entered,
            "pending_trigger_exact": pending_trigger_exact,
            "put_on_stack": trigger_put_on_stack,
        },
        "opponent_damage": {
            "untargeted_contract": untargeted_contract,
            "bound_action_count": damage_actions.len(),
            "exact_actions": exact_damage_actions,
            "trigger_stack_resolved": trigger_stack_resolved,
            "all_actions_applied": damage_actions_applied,
            "exact_damage_event_count": damage_events,
            "controller_life": trigger_state.players()[controller.index()].life(),
            "first_opponent_life": trigger_state.players()[first_opponent.index()].life(),
            "second_opponent_life": trigger_state.players()[second_opponent.index()].life(),
        },
        "team_pump": {
            "static_registered": pump_static_registered,
            "source_noncreature_before_pump": source_noncreature_before_pump,
            "generic_mana_cost": activated.mana_cost().base_generic(),
            "red_mana_cost": activated.mana_cost().colored_pool().get(ManaKind::Red),
            "funded": pump_funded,
            "paid": pump_paid,
            "payment_consumed": pump_payment_consumed,
            "bound_action_count": pump_actions.len(),
            "bound_action_exact": pump_action_exact,
            "all_actions_applied": pump_actions_applied,
            "controlled_creature_got_plus_one_zero": controlled_creature_got_plus_one_zero,
            "opponent_creature_unchanged": opponent_creature_unchanged,
            "controlled_noncreature_unchanged": noncreature_unchanged,
            "cleanup_reached": cleanup_reached,
            "expired_until_end_of_turn": expired_until_end_of_turn,
            "pump_expired_at_cleanup": pump_expired_at_cleanup,
        },
    }))
}

fn bala_ged_modal_dfc_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if program.name() != "Bala Ged Recovery // Bala Ged Sanctuary" {
        return None;
    }
    let back = program.modal_dfc_back()?;
    let combined_capabilities = program
        .capabilities()
        .iter()
        .map(|capability| capability.as_str())
        .collect::<Vec<_>>();
    let front_contract_exact = program.kind() == ProgramKind::Sorcery
        && program.mana_cost().base_generic() == 2
        && program.mana_cost().colored_pool() == ManaPool::of(ManaKind::Green, 1)
        && program.activated_abilities().is_empty()
        && program.activated_effects().is_empty()
        && program.triggered_abilities().is_empty()
        && program.static_abilities().is_empty()
        && matches!(
            program.effects(),
            [EffectProgram::MoveTargetObject {
                target: 0,
                from: ZoneKind::Graveyard,
                to: ZoneKind::Hand,
                overload_predicate: None,
            }]
        )
        && program.target_requirements().len() == 1
        && program.target_requirements()[0].kind()
            == TargetKind::ObjectInZoneKind(ZoneKind::Graveyard);
    let [back_mana] = back.activated_abilities() else {
        return Some(json!({
            "setup_succeeded": false,
            "observed_back_mana_ability_count": back.activated_abilities().len(),
        }));
    };
    let back_contract_exact = back.kind() == ProgramKind::Land
        && back.base_object().types() == ObjectTypes::none().with_land()
        && back.base_object().enters_tapped()
        && back.target_requirements().is_empty()
        && back.effects().is_empty()
        && back.activated_effects().is_empty()
        && back.triggered_abilities().is_empty()
        && back.static_abilities().is_empty()
        && back.modal_dfc_back().is_none()
        && back_mana.cost().mana() == ManaCost::new(0, 0, 0, 0, 0, 0)
        && back_mana.cost().tap_source()
        && !back_mana.cost().sacrifice_source()
        && back_mana.output_choices().options() == [ManaPool::of(ManaKind::Green, 1)];

    let mut front_state = GameState::new();
    let controller = match apply(&mut front_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut front_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let card_base =
        BaseObjectCharacteristics::new(ObjectTypes::none().with_artifact(), ObjectColors::none());
    let recovered = create_probe_object(
        &mut front_state,
        9_700_000,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Graveyard),
        card_base,
        None,
    )?;
    let wrong_zone = create_probe_object(
        &mut front_state,
        9_700_001,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Hand),
        card_base,
        None,
    )?;
    let opponent_card = create_probe_object(
        &mut front_state,
        9_700_002,
        opponent,
        opponent,
        ZoneId::new(Some(opponent), ZoneKind::Graveyard),
        card_base,
        None,
    )?;
    let before_missing_target = front_state.deterministic_hash();
    let missing_target_rejected_before_mutation = execute_program(
        &mut front_state,
        program,
        &ExecutionBindings::new(controller, vec![opponent]),
    )
    .is_err()
        && front_state.deterministic_hash() == before_missing_target;
    let before_wrong_zone = front_state.deterministic_hash();
    let wrong_zone_rejected_before_mutation = matches!(
        execute_program(
            &mut front_state,
            program,
            &ExecutionBindings::new(controller, vec![opponent])
                .with_targets(vec![TargetChoice::Object(wrong_zone)]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && front_state.deterministic_hash()
        == before_wrong_zone;
    let before_opponent_card = front_state.deterministic_hash();
    let opponent_card_rejected_before_mutation = matches!(
        execute_program(
            &mut front_state,
            program,
            &ExecutionBindings::new(controller, vec![opponent])
                .with_targets(vec![TargetChoice::Object(opponent_card)]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && front_state.deterministic_hash()
        == before_opponent_card;
    let front_bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_targets(vec![TargetChoice::Object(recovered)]);
    let front_actions = bind_program_actions(&front_state, program, &front_bindings).ok()?;
    let front_action_exact = matches!(
        front_actions.as_slice(),
        [bound] if matches!(
            bound.action(),
            Action::MoveObject { object, to }
                if *object == recovered
                    && *to == ZoneId::new(Some(controller), ZoneKind::Hand)
        )
    );
    let front_trace = execute_program(&mut front_state, program, &front_bindings).ok()?;
    let recovered_to_hand = front_trace.records().len() == 1
        && front_state.object_zone(recovered)
            == Some(ZoneId::new(Some(controller), ZoneKind::Hand));
    let unrelated_cards_unchanged = front_state.object_zone(wrong_zone)
        == Some(ZoneId::new(Some(controller), ZoneKind::Hand))
        && front_state.object_zone(opponent_card)
            == Some(ZoneId::new(Some(opponent), ZoneKind::Graveyard));
    let before_back_target = front_state.deterministic_hash();
    let back_rejects_front_target_before_mutation = matches!(
        bind_program_actions(
            &front_state,
            back,
            &ExecutionBindings::new(controller, vec![opponent])
                .with_targets(vec![TargetChoice::Object(recovered)]),
        ),
        Err(error) if error.code() == ExecutionDiagnosticCode::InvalidChoice
    ) && front_state.deterministic_hash()
        == before_back_target;

    let mut back_state = GameState::new();
    let land_controller = match apply(&mut back_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let land_opponent = match apply(&mut back_state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let land = create_probe_object(
        &mut back_state,
        9_701_000,
        land_controller,
        land_controller,
        ZoneId::new(Some(land_controller), ZoneKind::Hand),
        back.base_object(),
        back.base_creature(),
    )?;
    let front_copy = create_probe_object(
        &mut back_state,
        9_701_001,
        land_controller,
        land_controller,
        ZoneId::new(Some(land_controller), ZoneKind::Hand),
        program.base_object(),
        program.base_creature(),
    )?;
    create_probe_object(
        &mut back_state,
        9_701_002,
        land_controller,
        land_controller,
        ZoneId::new(Some(land_controller), ZoneKind::Library),
        card_base,
        None,
    )?;
    let land_window_ready = advance_to_precombat_main(&mut back_state, land_controller);
    let before_front_land_play = back_state.deterministic_hash();
    let front_not_playable_as_land = matches!(
        apply(
            &mut back_state,
            Action::PlayLand {
                player: land_controller,
                object: front_copy,
            },
        ),
        Outcome::Failed(StateError::ObjectNotPlayableAsLand(object)) if object == front_copy
    ) && back_state.deterministic_hash() == before_front_land_play;
    let land_played = matches!(
        apply(
            &mut back_state,
            Action::PlayLand {
                player: land_controller,
                object: land,
            },
        ),
        Outcome::Applied
    );
    let entered_battlefield_tapped = back_state.object_zone(land)
        == Some(ZoneId::new(None, ZoneKind::Battlefield))
        && back_state
            .object(land)
            .is_some_and(|record| record.tapped());
    let green_output = ManaPool::of(ManaKind::Green, 1);
    let mana_definition = back_mana.bind_selected(land_controller, land, green_output)?;
    let registered_mana = match apply(
        &mut back_state,
        Action::RegisterActivatedAbility {
            definition: mana_definition,
        },
    ) {
        Outcome::ActivatedAbilityRegistered(ability) => Some(ability),
        _ => None,
    };
    let zero_payment = auto_payment_plan(ManaPool::empty(), back_mana.cost().mana())
        .ok()
        .flatten()?;
    let before_tapped_activation = back_state.deterministic_hash();
    let tapped_land_activation_rejected_before_mutation = registered_mana.is_some_and(|ability| {
        matches!(
            apply(
                &mut back_state,
                Action::ActivateAbility {
                    player: land_controller,
                    ability,
                    payment: zero_payment,
                },
            ),
            Outcome::Failed(StateError::SourceAlreadyTapped(object)) if object == land
        )
    }) && back_state.deterministic_hash()
        == before_tapped_activation;
    let manually_untapped = matches!(
        apply(
            &mut back_state,
            Action::SetObjectTapped {
                object: land,
                tapped: false,
            },
        ),
        Outcome::Applied
    );
    let mana_activated = registered_mana.is_some_and(|ability| {
        matches!(
            apply(
                &mut back_state,
                Action::ActivateAbility {
                    player: land_controller,
                    ability,
                    payment: zero_payment,
                },
            ),
            Outcome::Applied
        )
    });
    let added_exactly_green = back_state.mana_pool(land_controller).ok() == Some(green_output);
    let land_tapped_for_mana = back_state
        .object(land)
        .is_some_and(|record| record.tapped());

    Some(json!({
        "setup_succeeded": true,
        "contract": {
            "combined_capabilities": combined_capabilities,
            "front_contract_exact": front_contract_exact,
            "back_contract_exact": back_contract_exact,
        },
        "front_face": {
            "missing_target_rejected_before_mutation":
                missing_target_rejected_before_mutation,
            "wrong_zone_rejected_before_mutation": wrong_zone_rejected_before_mutation,
            "opponent_card_rejected_before_mutation":
                opponent_card_rejected_before_mutation,
            "bound_action_count": front_actions.len(),
            "bound_action_exact": front_action_exact,
            "trace_record_count": front_trace.records().len(),
            "recovered_to_controller_hand": recovered_to_hand,
            "unrelated_cards_unchanged": unrelated_cards_unchanged,
        },
        "back_face": {
            "land_window_ready": land_window_ready,
            "land_played": land_played,
            "entered_battlefield_tapped": entered_battlefield_tapped,
            "mana_ability_registered": registered_mana.is_some(),
            "tapped_activation_rejected_before_mutation":
                tapped_land_activation_rejected_before_mutation,
            "manually_untapped": manually_untapped,
            "mana_activated": mana_activated,
            "added_exactly_green": added_exactly_green,
            "land_tapped_for_mana": land_tapped_for_mana,
        },
        "face_isolation": {
            "front_has_no_land_type_or_mana_ability":
                !program.base_object().types().land()
                    && program.activated_abilities().is_empty(),
            "back_has_no_spell_target_or_effect": back.target_requirements().is_empty()
                && back.effects().is_empty(),
            "back_rejects_front_target_before_mutation":
                back_rejects_front_target_before_mutation,
            "front_not_playable_as_land": front_not_playable_as_land,
            "back_has_no_nested_face": back.modal_dfc_back().is_none(),
            "opponent_player_unused_by_back_face":
                back_state.players()[land_opponent.index()].life() == 20,
        },
    }))
}

fn reveal_event_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let direct_reveal = program
        .effects()
        .iter()
        .any(|effect| matches!(effect, EffectProgram::RevealChosenObjects { .. }));
    let triggered = program.triggered_abilities().iter().find(|ability| {
        ability
            .effects()
            .iter()
            .any(|effect| matches!(effect, EffectProgram::RevealChosenObjects { .. }))
    });
    if !direct_reveal && triggered.is_none() {
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
    let source = create_probe_object(
        &mut state,
        9_990_000,
        controller,
        controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        program.base_object(),
        program.base_creature(),
    )?;
    let requirement = if direct_reveal {
        *program.object_choice_requirements().first()?
    } else {
        *triggered?.object_choice_requirements().first()?
    };
    if requirement.zone() != ZoneKind::Library {
        return Some(json!({"setup_succeeded": false, "phase": "choice_zone"}));
    }
    let mut types = requirement.required_types();
    if requirement.required_any_types() != ObjectTypes::none() {
        types = types.union(one_required_type(requirement.required_any_types()));
    }
    if types == ObjectTypes::none() && !requirement.required_subtypes().as_slice().is_empty() {
        types = ObjectTypes::none().with_creature();
    }
    if types == ObjectTypes::none() || types.intersects(requirement.forbidden_types()) {
        return Some(json!({"setup_succeeded": false, "phase": "choice_types"}));
    }
    let candidate_base = BaseObjectCharacteristics::new(types, ObjectColors::none())
        .with_supertypes(requirement.required_supertypes())
        .with_subtypes(requirement.required_subtypes());
    let selected = create_probe_object(
        &mut state,
        9_990_001,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Library),
        candidate_base,
        if types.creature() {
            Some(BaseCreatureCharacteristics::new(2, 2))
        } else {
            None
        },
    )?;
    let bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_source(source)
        .with_object_choices(vec![vec![selected]]);
    let actions = if direct_reveal {
        bind_program_actions(&state, program, &bindings).ok()?
    } else {
        let ability = triggered?;
        let bindings =
            bindings.with_optional_effect_choices(vec![true; ability.optional_choice_count()]);
        bind_triggered_ability_actions(&state, ability, &bindings).ok()?
    };
    let reveal_index = actions.iter().position(|bound| {
        matches!(
            bound.action(),
            Action::RevealObjects { objects } if objects.as_slice() == [selected]
        )
    });
    let destination_index = actions.iter().position(|bound| {
        matches!(
            bound.action(),
            Action::MoveObject { object, .. }
                | Action::PutObjectOnTopOfLibrary { object, .. }
                if *object == selected
        )
    });
    let cursor = state.event_cursor();
    let all_actions_applied = actions
        .iter()
        .all(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let revealed = state
        .events_since(cursor)
        .ok()?
        .iter()
        .filter_map(|record| match record.event() {
            GameEvent::ObjectRevealed { object } => Some(object),
            _ => None,
        })
        .collect::<Vec<_>>();

    Some(json!({
        "setup_succeeded": true,
        "reveal_action_present": reveal_index.is_some(),
        "destination_action_present": destination_index.is_some(),
        "reveal_precedes_destination": matches!((reveal_index, destination_index), (Some(reveal), Some(destination)) if reveal < destination),
        "public_reveal_event_emitted": revealed == vec![selected],
        "all_actions_applied": all_actions_applied,
    }))
}

fn regeneration_prohibition_probe(program: &CardProgram) -> Option<serde_json::Value> {
    if !program.effects().iter().any(|effect| {
        matches!(
            effect,
            EffectProgram::DestroyPermanentWithoutRegeneration { .. }
        )
    }) {
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
    let creature_stats = Some(BaseCreatureCharacteristics::new(3, 3));
    let baseline = create_probe_object(
        &mut state,
        9_991_000,
        opponent,
        opponent,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    let target = create_probe_object(
        &mut state,
        9_991_001,
        opponent,
        opponent,
        battlefield,
        creature_base,
        creature_stats,
    )?;
    for object in [baseline, target] {
        if !matches!(
            apply(
                &mut state,
                Action::RegisterRestriction {
                    definition: RestrictionDefinition::new(
                        opponent,
                        RestrictionEffect::RegenerationShield { object },
                    )
                    .with_duration(ContinuousEffectDuration::UntilEndOfTurn),
                },
            ),
            Outcome::RestrictionRegistered(_)
        ) {
            return Some(json!({"setup_succeeded": false, "phase": "shield"}));
        }
    }
    let _ = apply(
        &mut state,
        Action::MarkDamageOnObject {
            object: baseline,
            amount: 2,
        },
    );
    let normal_destroy_applied = matches!(
        apply(&mut state, Action::DestroyPermanent { object: baseline },),
        Outcome::Applied
    );
    let baseline_record = state.object(baseline)?;
    let bindings = ExecutionBindings::new(controller, vec![opponent])
        .with_targets(vec![TargetChoice::Object(target)]);
    let actions = bind_program_actions(&state, program, &bindings).ok()?;
    let no_regeneration_action_present = actions.iter().any(|bound| {
        matches!(
            bound.action(),
            Action::DestroyPermanentWithoutRegeneration { object } if *object == target
        )
    });
    let destruction_action_applied = actions
        .iter()
        .find(|bound| {
            matches!(
                bound.action(),
                Action::DestroyPermanentWithoutRegeneration { object } if *object == target
            )
        })
        .is_some_and(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied));
    let target_graveyard = ZoneId::new(Some(opponent), ZoneKind::Graveyard);

    Some(json!({
        "setup_succeeded": true,
        "normal_destroy_applied": normal_destroy_applied,
        "normal_destroy_replaced": state.object_zone(baseline) == Some(battlefield),
        "normal_destroy_tapped": baseline_record.tapped(),
        "normal_destroy_cleared_damage": baseline_record.damage_marked() == 0,
        "no_regeneration_action_present": no_regeneration_action_present,
        "destruction_action_applied": destruction_action_applied,
        "shielded_target_destroyed": state.object_zone(target) == Some(target_graveyard),
        "all_shields_consumed_or_expired": !state.restrictions().any(|(_, definition)| matches!(definition.effect(), RestrictionEffect::RegenerationShield { .. })),
    }))
}

fn cast_or_copy_probe(program: &CardProgram) -> Option<serde_json::Value> {
    let ability = program.triggered_abilities().iter().find(|ability| {
        matches!(
            ability.event(),
            TriggeredEventProgram::ControllerCastsOrCopies(_)
        )
    })?;
    let mut state = GameState::new();
    let controller = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let opponent = match apply(&mut state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        _ => return Some(json!({"setup_succeeded": false})),
    };
    let source = create_probe_object(
        &mut state,
        9_992_000,
        controller,
        controller,
        ZoneId::new(None, ZoneKind::Battlefield),
        program.base_object(),
        program.base_creature(),
    )?;
    let definition = ability.bind(controller, source);
    let condition_exact = matches!(
        definition.condition(),
        TriggerCondition::StackEntryAddedOrCopied { .. }
    );
    let trigger = match apply(&mut state, Action::RegisterTriggeredAbility { definition }) {
        Outcome::TriggerRegistered(trigger) => trigger,
        _ => return Some(json!({"setup_succeeded": false, "phase": "register"})),
    };
    for salt in 0..2 {
        let _ = apply(
            &mut state,
            Action::CreateObject {
                card: CardId::new(9_992_010 + salt),
                owner: controller,
                controller,
                zone: ZoneId::new(Some(controller), ZoneKind::Library),
            },
        );
    }
    let spell = create_probe_object(
        &mut state,
        9_992_020,
        controller,
        controller,
        ZoneId::new(Some(controller), ZoneKind::Hand),
        BaseObjectCharacteristics::new(ObjectTypes::none().with_instant(), ObjectColors::none()),
        None,
    )?;
    let priority_ready = prepare_stack_priority(&mut state, controller);
    let original = match apply(
        &mut state,
        Action::PutSpellOnStack {
            player: controller,
            object: spell,
            kind: StackObjectKind::InstantSpell,
            hold_priority: true,
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        _ => return Some(json!({"setup_succeeded": false, "phase": "cast"})),
    };
    let cast_trigger_queued =
        state.pending_triggers().len() == 1 && state.pending_triggers()[0].trigger() == trigger;
    let copy = match apply(
        &mut state,
        Action::CopyStackEntry {
            player: controller,
            entry: original,
        },
    ) {
        Outcome::StackEntryAdded(entry) => entry,
        _ => return Some(json!({"setup_succeeded": false, "phase": "copy"})),
    };
    let cast_and_copy_queued = state.pending_triggers().len() == 2
        && state
            .pending_triggers()
            .iter()
            .all(|pending| pending.trigger() == trigger);
    let copy_provenance_exact = state
        .stack_entries()
        .iter()
        .find(|entry| entry.id() == copy)
        .and_then(|entry| entry.copy_info())
        .is_some_and(|info| info.source_entry() == original && info.source_object() == Some(spell));
    let bindings = ExecutionBindings::new(controller, vec![opponent]).with_source(source);
    let actions = bind_triggered_ability_actions(&state, ability, &bindings).ok()?;
    let draw_action_exact = matches!(
        actions.as_slice(),
        [bound]
            if matches!(bound.action(), Action::DrawCards { player, count } if *player == controller && *count == 1)
    );
    let hand = ZoneId::new(Some(controller), ZoneKind::Hand);
    let before = state
        .zone_objects(hand)
        .map_or(0, <[forge_core::ObjectId]>::len);
    let two_draw_resolutions = (0..2).all(|_| {
        actions
            .iter()
            .all(|bound| matches!(apply(&mut state, bound.action().clone()), Outcome::Applied))
    });
    let after = state
        .zone_objects(hand)
        .map_or(0, <[forge_core::ObjectId]>::len);

    Some(json!({
        "setup_succeeded": priority_ready,
        "condition_exact": condition_exact,
        "cast_trigger_queued": cast_trigger_queued,
        "cast_and_copy_queued": cast_and_copy_queued,
        "copy_provenance_exact": copy_provenance_exact,
        "draw_action_exact": draw_action_exact,
        "two_draw_resolutions": two_draw_resolutions && after == before.saturating_add(2),
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
        "reveal_event": reveal_event_probe(program),
        "regeneration_prohibition": regeneration_prohibition_probe(program),
        "cast_or_copy": cast_or_copy_probe(program),
        "no_maximum_hand_size": no_maximum_hand_size_probe(program),
        "equipment": equipment_probe(program),
        "sacrifice_counter": sacrifice_counter_probe(program),
        "temporary_protection": temporary_protection_probe(program),
        "commander_alternate_cost": commander_alternate_cost_probe(program),
        "flashback_looting": flashback_looting_probe(program),
        "split_second": split_second_probe(program),
        "overload": overload_probe(program),
        "evoke": evoke_probe(program),
        "boros_charm": boros_charm_probe(program),
        "reconnaissance_mission": reconnaissance_mission_probe(program),
        "smothering_tithe": smothering_tithe_probe(program),
        "purphoros": purphoros_probe(program),
        "bala_ged_modal_dfc": bala_ged_modal_dfc_probe(program),
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

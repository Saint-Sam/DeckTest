#![forbid(unsafe_code)]

use forge_cards::runtime::{
    compile_card_program, ActivatedAbilityProgram, CardProgram, EffectProgram,
};
use forge_core::{
    apply, auto_payment_plan, Action, CardId, GameState, ManaKind, ManaPool, Outcome, ZoneId,
    ZoneKind,
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

#![forbid(unsafe_code)]

//! Emits the deterministic T4 runtime-isomorphism evidence fixture.

use forge_core::{
    apply, AbilityPlayer, Action, ActivatedAbilityDefinition, ActivatedAbilityEffect,
    ActivationCost, ActivationTiming, BenchmarkRuntimeSemantics, CardId, DecisionContext,
    DecisionDescriptor, DecisionKind, DecisionOption, GameState, ManaCost, ManaPool, ObjectId,
    Outcome, PlayerId, StackObjectKind, ZoneId, ZoneKind,
};
use serde_json::json;
use std::{env, fs, path::PathBuf};

struct FixtureContext {
    context: DecisionContext,
    exact_without_semantics: DecisionContext,
    player: PlayerId,
    view: forge_core::PlayerView,
    option: DecisionOption,
    runtime: BenchmarkRuntimeSemantics,
}

fn apply_ok(state: &mut GameState, action: Action) -> Result<Outcome, String> {
    match apply(state, action) {
        Outcome::Failed(error) => Err(format!("fixture action failed: {error:?}")),
        outcome => Ok(outcome),
    }
}

fn add_player(state: &mut GameState) -> Result<PlayerId, String> {
    match apply_ok(state, Action::AddPlayer)? {
        Outcome::PlayerAdded(player) => Ok(player),
        outcome => Err(format!("unexpected AddPlayer outcome: {outcome:?}")),
    }
}

fn create_permanent(
    state: &mut GameState,
    player: PlayerId,
    card: u32,
) -> Result<ObjectId, String> {
    match apply_ok(
        state,
        Action::CreateObject {
            card: CardId::new(card),
            owner: player,
            controller: player,
            zone: ZoneId::new(None, ZoneKind::Battlefield),
        },
    )? {
        Outcome::ObjectCreated(object) => Ok(object),
        outcome => Err(format!("unexpected CreateObject outcome: {outcome:?}")),
    }
}

fn register_mana_ability(
    state: &mut GameState,
    player: PlayerId,
    source: ObjectId,
    mana: ManaPool,
) -> Result<forge_core::ActivatedAbilityId, String> {
    let definition = ActivatedAbilityDefinition::new(
        player,
        Some(source),
        ActivationTiming::Instant,
        ActivationCost::new(ManaCost::new(0, 0, 0, 0, 0, 0)),
        ActivatedAbilityEffect::AddMana {
            player: AbilityPlayer::Controller,
            mana,
        },
    );
    match apply_ok(
        state,
        Action::RegisterActivatedAbility {
            definition: Box::new(definition),
        },
    )? {
        Outcome::ActivatedAbilityRegistered(ability) => Ok(ability),
        outcome => Err(format!(
            "unexpected RegisterActivatedAbility outcome: {outcome:?}"
        )),
    }
}

fn build_fixture(reverse: bool) -> Result<FixtureContext, String> {
    let mut state = GameState::new();
    let player = add_player(&mut state)?;
    let (source, decoy) = if reverse {
        let decoy = create_permanent(&mut state, player, 202)?;
        let source = create_permanent(&mut state, player, 101)?;
        (source, decoy)
    } else {
        let source = create_permanent(&mut state, player, 101)?;
        let decoy = create_permanent(&mut state, player, 202)?;
        (source, decoy)
    };
    let (ability, _decoy_ability) = if reverse {
        let decoy_ability =
            register_mana_ability(&mut state, player, decoy, ManaPool::new(0, 1, 0, 0, 0, 0))?;
        let ability =
            register_mana_ability(&mut state, player, source, ManaPool::new(0, 0, 0, 0, 1, 0))?;
        (ability, decoy_ability)
    } else {
        let ability =
            register_mana_ability(&mut state, player, source, ManaPool::new(0, 0, 0, 0, 1, 0))?;
        let decoy_ability =
            register_mana_ability(&mut state, player, decoy, ManaPool::new(0, 1, 0, 0, 0, 0))?;
        (ability, decoy_ability)
    };
    let payment = state
        .payment_plans_for_player(player, ManaCost::new(0, 0, 0, 0, 0, 0))
        .map_err(|error| format!("zero-payment enumeration failed: {error:?}"))?
        .best()
        .ok_or_else(|| "zero-payment plan is missing".to_owned())?;
    let view = state
        .player_view(player)
        .map_err(|error| format!("fixture PlayerView failed: {error:?}"))?;
    let option = DecisionOption::new(
        DecisionDescriptor::ActivateAbility {
            source,
            ability,
            payment,
        },
        Vec::new(),
    );
    let exact_without_semantics = DecisionContext::new(
        DecisionKind::MainPhase,
        player,
        &view,
        vec![option.clone()],
        Vec::new(),
    )
    .map_err(|error| format!("exact fixture context failed: {error}"))?;
    let mut runtime = BenchmarkRuntimeSemantics::default();
    runtime.bind_ability(ability, source, b"fixture-oracle-101/mana/0");
    let context = DecisionContext::new_with_benchmark_semantics(
        DecisionKind::MainPhase,
        player,
        &view,
        vec![option.clone()],
        Vec::new(),
        &runtime,
    )
    .map_err(|error| format!("normalized fixture context failed: {error}"))?;
    Ok(FixtureContext {
        context,
        exact_without_semantics,
        player,
        view,
        option,
        runtime,
    })
}

fn stack_context(kind: StackObjectKind) -> Result<DecisionContext, String> {
    let mut state = GameState::new();
    let player = add_player(&mut state)?;
    apply_ok(
        &mut state,
        Action::StartTurn {
            active_player: player,
        },
    )?;
    apply_ok(&mut state, Action::AdvanceStep)?;
    let entry = match apply_ok(
        &mut state,
        Action::PutAbilityOnStack {
            player,
            kind,
            hold_priority: true,
        },
    )? {
        Outcome::StackEntryAdded(entry) => entry,
        outcome => return Err(format!("unexpected stack fixture outcome: {outcome:?}")),
    };
    let view = state
        .player_view(player)
        .map_err(|error| format!("stack fixture PlayerView failed: {error:?}"))?;
    let stack_entry = state
        .stack_entries()
        .iter()
        .find(|candidate| candidate.id() == entry)
        .cloned()
        .ok_or_else(|| "stack fixture entry is missing".to_owned())?;
    let mut runtime = BenchmarkRuntimeSemantics::default();
    runtime.bind_stack_entry(stack_entry, 0);
    DecisionContext::new_with_benchmark_semantics(
        DecisionKind::Priority,
        player,
        &view,
        vec![DecisionOption::new(
            DecisionDescriptor::PassPriority,
            vec![Action::PassPriority { player }],
        )],
        Vec::new(),
        &runtime,
    )
    .map_err(|error| format!("stack fixture context failed: {error}"))
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "usage: forge-t4-runtime-isomorphism OUTPUT COMMIT TREE".to_owned())?;
    let product_commit = args
        .next()
        .ok_or_else(|| "missing exact product commit".to_owned())?;
    let product_tree = args
        .next()
        .ok_or_else(|| "missing exact product tree".to_owned())?;
    if args.next().is_some() {
        return Err("unexpected runtime-isomorphism argument".to_owned());
    }

    let first = build_fixture(false)?;
    let reordered = build_fixture(true)?;
    let exact_ids_unchanged = first.context.id() == first.exact_without_semantics.id()
        && first.context.state_key() == first.exact_without_semantics.state_key()
        && reordered.context.id() == reordered.exact_without_semantics.id()
        && reordered.context.state_key() == reordered.exact_without_semantics.state_key();
    let allocation_and_registration_isomorphic =
        first.context.normalized_benchmark_key() == reordered.context.normalized_benchmark_key();
    let exact_runtime_handles_remain_distinct = first.context.id() != reordered.context.id()
        && first.context.state_key() != reordered.context.state_key();

    let first_path = DecisionContext::new_scoped_with_benchmark_semantics(
        DecisionKind::MainPhase,
        first.player,
        &first.view,
        vec![first.option.clone()],
        Vec::new(),
        17,
        &first.runtime,
    )
    .map_err(|error| format!("first path fixture failed: {error}"))?;
    let other_path = DecisionContext::new_scoped_with_benchmark_semantics(
        DecisionKind::MainPhase,
        first.player,
        &first.view,
        vec![first.option.clone()],
        Vec::new(),
        18,
        &first.runtime,
    )
    .map_err(|error| format!("other path fixture failed: {error}"))?;
    let hierarchical_paths_remain_distinct =
        first_path.normalized_benchmark_key() != other_path.normalized_benchmark_key();

    let mut unequal_runtime = BenchmarkRuntimeSemantics::default();
    if let DecisionDescriptor::ActivateAbility {
        source, ability, ..
    } = first.option.descriptor()
    {
        unequal_runtime.bind_ability(*ability, *source, b"fixture-oracle-101/draw/0");
    } else {
        return Err("fixture option lost its activated-ability descriptor".to_owned());
    }
    let unequal = DecisionContext::new_with_benchmark_semantics(
        DecisionKind::MainPhase,
        first.player,
        &first.view,
        vec![first.option.clone()],
        Vec::new(),
        &unequal_runtime,
    )
    .map_err(|error| format!("unequal semantics fixture failed: {error}"))?;
    let unequal_semantics_remain_distinct =
        first.context.normalized_benchmark_key() != unequal.normalized_benchmark_key();
    let activated_stack = stack_context(StackObjectKind::ActivatedAbility)?;
    let triggered_stack = stack_context(StackObjectKind::TriggeredAbility)?;
    let visible_stack_semantics_remain_distinct =
        activated_stack.normalized_benchmark_key() != triggered_stack.normalized_benchmark_key();
    let normalization_complete = first.context.benchmark_normalization_complete()
        && reordered.context.benchmark_normalization_complete()
        && first_path.benchmark_normalization_complete()
        && other_path.benchmark_normalization_complete()
        && unequal.benchmark_normalization_complete()
        && activated_stack.benchmark_normalization_complete()
        && triggered_stack.benchmark_normalization_complete();
    let passed = exact_ids_unchanged
        && allocation_and_registration_isomorphic
        && exact_runtime_handles_remain_distinct
        && hierarchical_paths_remain_distinct
        && unequal_semantics_remain_distinct
        && visible_stack_semantics_remain_distinct
        && normalization_complete;
    let report = json!({
        "schema_version": 1,
        "status": if passed { "passed" } else { "failed" },
        "artifact_classification": "diagnostic_not_promotion_eligible",
        "product_commit": product_commit,
        "product_tree": product_tree,
        "checks": {
            "exact_replay_ids_unchanged": exact_ids_unchanged,
            "object_allocation_order_isomorphic": allocation_and_registration_isomorphic,
            "equivalent_mana_source_creation_order_isomorphic": allocation_and_registration_isomorphic,
            "ability_registration_order_isomorphic": allocation_and_registration_isomorphic,
            "equivalent_zone_membership_runtime_handles_isomorphic": allocation_and_registration_isomorphic,
            "exact_runtime_handles_remain_distinct": exact_runtime_handles_remain_distinct,
            "hierarchical_paths_remain_distinct": hierarchical_paths_remain_distinct,
            "unequal_semantics_remain_distinct": unequal_semantics_remain_distinct,
            "visible_stack_semantics_remain_distinct": visible_stack_semantics_remain_distinct,
            "normalization_complete": normalization_complete
        },
        "fixtures": {
            "first_exact_context_id": first.context.id().to_string(),
            "reordered_exact_context_id": reordered.context.id().to_string(),
            "shared_normalized_benchmark_key": first.context.normalized_benchmark_key().to_string(),
            "first_path_key": first_path.normalized_benchmark_key().to_string(),
            "other_path_key": other_path.normalized_benchmark_key().to_string(),
            "unequal_semantics_key": unequal.normalized_benchmark_key().to_string()
        },
        "claim_limits": [
            "exact replay identifiers remain authoritative for replay execution",
            "normalized keys are evidence-only and require complete semantic bindings",
            "sealed benchmark split evidence remains a separate promotion gate"
        ]
    });
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(
        &output,
        serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("cannot serialize fixture report: {error}"))?,
    )
    .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    if passed {
        Ok(())
    } else {
        Err("runtime-isomorphism fixture failed".to_owned())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

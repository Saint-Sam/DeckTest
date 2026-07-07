#![allow(missing_docs)]

//! Kernel performance benchmarks for T1.13.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use forge_core::{
    apply, legal_actions, Action, CardId, GameOutcome, GameState, Outcome, PlayerId, ZoneId,
    ZoneKind,
};
use std::time::Duration;

fn add_player(state: &mut GameState) -> PlayerId {
    match apply(state, Action::AddPlayer) {
        Outcome::PlayerAdded(player) => player,
        other => panic!("unexpected AddPlayer outcome: {other:?}"),
    }
}

fn apply_ok(state: &mut GameState, action: Action) {
    match apply(state, action) {
        Outcome::Applied
        | Outcome::TurnOrderDecided(_)
        | Outcome::StepAdvanced(_)
        | Outcome::Priority(_) => {}
        other => panic!("unexpected action outcome: {other:?}"),
    }
}

fn create_object(state: &mut GameState, player: PlayerId, zone: ZoneId, card: u32) {
    match apply(
        state,
        Action::CreateObject {
            card: CardId::new(card),
            owner: player,
            controller: player,
            zone,
        },
    ) {
        Outcome::ObjectCreated(_) => {}
        other => panic!("unexpected CreateObject outcome: {other:?}"),
    }
}

fn seed_library(state: &mut GameState, player: PlayerId, first_card: u32, count: u32) {
    let library = ZoneId::new(Some(player), ZoneKind::Library);
    for offset in 0..count {
        create_object(state, player, library, first_card + offset);
    }
}

fn setup_state(cards_per_player: u32) -> GameState {
    let mut state = GameState::new();
    apply_ok(&mut state, Action::SetSeed { seed: 0xF0_26_E2 });
    let first = add_player(&mut state);
    let second = add_player(&mut state);
    seed_library(&mut state, first, 1_000, cards_per_player);
    seed_library(&mut state, second, 2_000, cards_per_player);
    apply_ok(&mut state, Action::DecideTurnOrder);
    apply_ok(&mut state, Action::DrawOpeningHands);
    let players: Vec<PlayerId> = state.players().iter().map(|player| player.id()).collect();
    for player in players {
        apply_ok(
            &mut state,
            Action::KeepOpeningHand {
                player,
                bottom: Vec::new(),
            },
        );
    }
    state
}

fn priority_state() -> (GameState, PlayerId) {
    let mut state = GameState::new();
    let active = add_player(&mut state);
    let _nonactive = add_player(&mut state);
    apply_ok(
        &mut state,
        Action::StartTurn {
            active_player: active,
        },
    );
    apply_ok(&mut state, Action::AdvanceStep);
    (state, active)
}

fn play_four_turns(mut state: GameState) -> u64 {
    let active = match state.starting_player() {
        Some(player) => player,
        None => panic!("setup state has no starting player"),
    };
    apply_ok(
        &mut state,
        Action::StartTurn {
            active_player: active,
        },
    );

    let mut steps = 0_u32;
    while state.game_outcome() == GameOutcome::InProgress && state.turn_number() <= 4 {
        steps = steps.saturating_add(1);
        if steps > 512 {
            panic!("playout step limit exceeded");
        }
        if let Some(player) = state.priority_player() {
            apply_ok(&mut state, Action::PassPriority { player });
        } else {
            apply_ok(&mut state, Action::AdvanceStep);
        }
    }
    state.deterministic_hash_streaming().get()
}

fn bench_clone(c: &mut Criterion) {
    let state = setup_state(100);
    c.bench_function("kernel/clone_200_card_state_x64", |b| {
        b.iter(|| {
            for _ in 0..64 {
                black_box(black_box(&state).clone());
            }
        });
    });
}

fn bench_legal_actions(c: &mut Criterion) {
    let (state, _) = priority_state();
    c.bench_function("kernel/legal_actions_priority_x128", |b| {
        b.iter(|| {
            let mut total = 0_usize;
            for _ in 0..128 {
                total = total.saturating_add(legal_actions(black_box(&state)).len());
            }
            black_box(total);
        });
    });
}

fn bench_apply(c: &mut Criterion) {
    let (state, player) = priority_state();
    c.bench_function("kernel/apply_pass_priority_x64", |b| {
        b.iter_batched(
            || vec![state.clone(); 64],
            |mut states| {
                for state in &mut states {
                    black_box(apply(state, Action::PassPriority { player }));
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_playout(c: &mut Criterion) {
    let state = setup_state(100);
    c.bench_function("kernel/full_playout_four_turns", |b| {
        b.iter_batched(
            || state.clone(),
            |state| black_box(play_four_turns(state)),
            BatchSize::SmallInput,
        );
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(300));
    targets = bench_clone, bench_legal_actions, bench_apply, bench_playout
}
criterion_main!(benches);

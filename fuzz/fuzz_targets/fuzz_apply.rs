#![no_main]

use forge_core::{
    apply, legal_actions, Action, CardId, GameOutcome, GameState, Outcome, PlayerId, ZoneId,
    ZoneKind,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    fuzz_apply(data);
});

fn fuzz_apply(data: &[u8]) {
    let mut reader = ByteReader::new(data);
    let seed = reader.read_u64();
    let mut state = GameState::new();
    let mut players = Vec::new();

    let _ = apply(&mut state, Action::SetSeed { seed });
    for _ in 0..2 {
        if let Outcome::PlayerAdded(player) = apply(&mut state, Action::AddPlayer) {
            players.push(player);
        }
    }
    for (index, player) in players.iter().copied().enumerate() {
        seed_library(&mut state, player, 10_000 + (index as u32 * 1_000), 8);
    }

    let mut steps = 0_u16;
    while let Some(byte) = reader.next() {
        if steps >= 512 || state.game_outcome() != GameOutcome::InProgress {
            break;
        }
        steps = steps.saturating_add(1);
        if byte % 5 == 0 {
            apply_one_legal_action(&mut state, byte);
        } else {
            apply_scripted_action(&mut state, &players, byte, &mut reader);
        }
        assert_invariants(&state);
    }
}

fn seed_library(state: &mut GameState, player: PlayerId, first_card: u32, count: u32) {
    let zone = ZoneId::new(Some(player), ZoneKind::Library);
    for offset in 0..count {
        let _ = apply(
            state,
            Action::CreateObject {
                card: CardId::new(first_card.saturating_add(offset)),
                owner: player,
                controller: player,
                zone,
            },
        );
    }
}

fn apply_one_legal_action(state: &mut GameState, selector: u8) {
    let actions = legal_actions(state);
    if actions.is_empty() {
        return;
    }
    let index = usize::from(selector) % actions.len();
    let action = actions.actions()[index].clone();
    let _ = apply(state, action);
}

fn apply_scripted_action(
    state: &mut GameState,
    players: &[PlayerId],
    selector: u8,
    reader: &mut ByteReader<'_>,
) {
    let Some(player) = players.get(usize::from(selector) % players.len()).copied() else {
        return;
    };
    match selector % 12 {
        0 => {
            let _ = apply(state, Action::DecideTurnOrder);
        }
        1 => {
            let _ = apply(state, Action::DrawOpeningHands);
        }
        2 => {
            let _ = apply(
                state,
                Action::KeepOpeningHand {
                    player,
                    bottom: Vec::new(),
                },
            );
        }
        3 => {
            let active_player = state.starting_player().unwrap_or(player);
            let _ = apply(state, Action::StartTurn { active_player });
        }
        4 => {
            let _ = apply(state, Action::AdvanceStep);
        }
        5 => {
            let amount = u32::from(reader.next().unwrap_or(0) % 8);
            let _ = apply(state, Action::LoseLife { player, amount });
        }
        6 => {
            let amount = u32::from(reader.next().unwrap_or(0) % 8);
            let _ = apply(state, Action::GainLife { player, amount });
        }
        7 => {
            let amount = u32::from(reader.next().unwrap_or(0) % 4);
            let _ = apply(state, Action::AddPoisonCounters { player, amount });
        }
        8 => {
            let _ = apply(state, Action::CheckStateBasedActions);
        }
        9 => {
            if state.priority_player() == Some(player) {
                let _ = apply(state, Action::PassPriority { player });
            }
        }
        10 => {
            let _ = state.player_view(player);
        }
        _ => {
            let _ = state.deterministic_hash_streaming();
        }
    }
}

fn assert_invariants(state: &GameState) {
    if let Err(error) = state.validate_zone_conservation() {
        panic!("zone conservation failed: {error:?}");
    }
    if state.deterministic_hash() != state.deterministic_hash_streaming() {
        panic!("allocated and streaming hashes diverged");
    }
}

struct ByteReader<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, index: 0 }
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.data.get(self.index).copied();
        self.index = self.index.saturating_add(1);
        value
    }

    fn read_u64(&mut self) -> u64 {
        let mut value = 0_u64;
        for shift in 0..8 {
            value |= u64::from(self.next().unwrap_or(0)) << (shift * 8);
        }
        value
    }
}

#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! AI policy, search, and difficulty ladder crate for Forge 2.0.

use forge_core::PlayerView;

/// Returns true when the bootstrap crate is linked correctly.
#[must_use]
pub const fn crate_ready() -> bool {
    true
}

/// Returns true when a redacted player view is structurally readable by AI code.
#[must_use]
pub fn can_read_player_view(view: &PlayerView) -> bool {
    view.players().get(view.observer().index()).is_some()
}

#[cfg(test)]
mod tests {
    use super::{can_read_player_view, crate_ready};
    use forge_core::{apply, Action, GameState, Outcome};

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }

    #[test]
    fn ai_entrypoint_accepts_player_view() {
        let mut state = GameState::new();
        let player = match apply(&mut state, Action::AddPlayer) {
            Outcome::PlayerAdded(player) => player,
            other => panic!("unexpected player outcome: {other:?}"),
        };
        let view = state
            .player_view(player)
            .unwrap_or_else(|error| panic!("unexpected view error: {error:?}"));

        assert!(can_read_player_view(&view));
    }
}

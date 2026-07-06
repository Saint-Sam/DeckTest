#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Legacy import, translation, and coverage tooling crate for Forge 2.0.

/// Returns true when the bootstrap crate is linked correctly.
#[must_use]
pub const fn crate_ready() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::crate_ready;

    #[test]
    fn bootstrap_crate_is_ready() {
        assert!(crate_ready());
    }
}

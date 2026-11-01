//! punch-core: store, domain rules, and GitHub/Basecamp integrations.
//! Has no terminal or UI dependencies. See docs/features/v1/SPEC.md.

/// Crate version, shared with the binary.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}

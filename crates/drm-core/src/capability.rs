//! The frozen O/D/C root vocabulary and the fixed capability -> root mapping.
//!
//! Every capability, and every symbol later *derived* from capabilities by
//! [`crate::vocabulary::Vocabulary`], must reduce -- recursively, with cycle
//! detection -- to nothing but these three root tokens. This invariant is
//! checked by [`Vocabulary::audit`](crate::vocabulary::Vocabulary::audit) and
//! is exercised in `drm-core`'s test suite; it must never change without a
//! major version bump, since external tooling may depend on the root
//! vocabulary staying exactly `["OBSERVE", "DERIVE", "COMMIT"]`.

/// The frozen root vocabulary. Do not add, remove, or reorder entries.
pub const ROOT: [&str; 3] = ["OBSERVE", "DERIVE", "COMMIT"];

/// A capability is a primitive, directly-executable unit of work. Each one
/// maps to a short, fixed sequence of root tokens describing its shape in
/// terms of observation / derivation / commitment.
pub fn root_expansion(capability: &str) -> &'static [&'static str] {
    match capability {
        "fs.read" | "state.read" | "proc.observe" | "timer.observe" => &["OBSERVE"],
        "http.request" | "web.selenium" | "ipc.request" | "process.run" => &["DERIVE", "COMMIT", "OBSERVE"],
        "transform.extract" | "transform.summarize" => &["DERIVE"],
        "fs.write" | "state.write" | "notify.send" => &["DERIVE", "COMMIT"],
        _ => &[],
    }
}

/// All capabilities known to the runtime, in a stable order.
pub const CAPABILITIES: [&str; 13] = [
    "fs.read",
    "state.read",
    "proc.observe",
    "timer.observe",
    "http.request",
    "web.selenium",
    "ipc.request",
    "process.run",
    "transform.extract",
    "transform.summarize",
    "fs.write",
    "state.write",
    "notify.send",
];

pub fn is_root(token: &str) -> bool {
    ROOT.contains(&token)
}

pub fn is_known_capability(capability: &str) -> bool {
    !root_expansion(capability).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_vocabulary_is_frozen() {
        assert_eq!(ROOT, ["OBSERVE", "DERIVE", "COMMIT"]);
    }

    #[test]
    fn every_capability_reduces_to_roots_only() {
        for cap in CAPABILITIES {
            let roots = root_expansion(cap);
            assert!(!roots.is_empty(), "capability {cap} has no root expansion");
            assert!(roots.iter().all(|r| is_root(r)), "capability {cap} expands to a non-root token");
        }
    }

    #[test]
    fn unknown_capability_has_empty_expansion() {
        assert!(root_expansion("nonexistent.capability").is_empty());
        assert!(!is_known_capability("nonexistent.capability"));
    }
}

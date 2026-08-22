//! Resolves the parts of a [`drm_core::identity::ExecutionContext`] that
//! come from the surrounding OS/session rather than from the calling
//! application: `host_id` and `user_scope`. `application_id`,
//! `workload_id`, and `task_id` are always supplied by the caller (drmd's
//! wire protocol, a benchmark harness, ...) -- this module exists so those
//! callers don't each have to reinvent "what machine/user am I" resolution,
//! not to guess at identity a caller should be stating explicitly.

/// Resolve a stable host identifier for this machine.
///
/// Order: `$DRM_HOST_ID` override (tests, containers that want a fixed,
/// reproducible identity) -> `/proc/sys/kernel/hostname` (no subprocess,
/// no `libc` dependency, works identically in a container) ->
/// `"unknown-host"`.
pub fn resolve_host_id() -> String {
    if let Ok(v) = std::env::var("DRM_HOST_ID") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    "unknown-host".to_string()
}

/// Resolve the identity of the user/session that owns this process.
///
/// Order: `$DRM_USER_SCOPE` override -> `$USER` -> `$LOGNAME` ->
/// `"unknown-user"`.
pub fn resolve_user_scope() -> String {
    for var in ["DRM_USER_SCOPE", "USER", "LOGNAME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    "unknown-user".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // resolve_host_id/resolve_user_scope read process-global environment
    // variables; serialize the tests that mutate them so they can't race
    // against each other under a parallel test runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn host_id_prefers_explicit_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DRM_HOST_ID", "test-host-42");
        assert_eq!(resolve_host_id(), "test-host-42");
        std::env::remove_var("DRM_HOST_ID");
    }

    #[test]
    fn user_scope_prefers_explicit_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DRM_USER_SCOPE", "test-user-42");
        assert_eq!(resolve_user_scope(), "test-user-42");
        std::env::remove_var("DRM_USER_SCOPE");
    }

    #[test]
    fn host_id_falls_back_to_a_non_empty_value_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DRM_HOST_ID");
        assert!(!resolve_host_id().is_empty());
    }

    #[test]
    fn user_scope_falls_back_to_a_non_empty_value_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DRM_USER_SCOPE");
        std::env::remove_var("USER");
        std::env::remove_var("LOGNAME");
        assert_eq!(resolve_user_scope(), "unknown-user");
    }
}

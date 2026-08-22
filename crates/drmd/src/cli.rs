//! A small, dependency-free argument parser. `drmd` has five subcommands
//! and a handful of `--flag value` options each -- not enough surface to
//! justify a full argument-parsing crate, and keeping this hand-rolled
//! means the whole workspace builds with zero external crates: `cargo
//! build` never touches the network, matching the historical projects'
//! own zero-dependency convention.

use std::collections::HashMap;
use std::path::PathBuf;

pub const DEFAULT_SOCKET: &str = "/run/drmd/drmd.sock";
pub const DEFAULT_WORK_DIR: &str = "/var/lib/drmd";
pub const DEFAULT_STATE_DIR: &str = "/var/lib/drmd/state";
pub const DEFAULT_BENCH_OUT: &str = "results";

/// The first argument that isn't part of a `--flag [value]` pair -- used
/// by subcommands with one required positional argument (`application
/// <id>`, `workload <id>`, `explain <id>`, `reset <scope>`). Must be
/// `args[0]`: these subcommands always take their identifier first,
/// flags after, matching every example in the spec's CLI section.
pub fn positional(args: &[String]) -> Option<&str> {
    args.first().filter(|a| !a.starts_with("--")).map(|s| s.as_str())
}

pub struct ParsedArgs {
    pub flags: HashMap<String, String>,
    pub switches: std::collections::HashSet<String>,
}

impl ParsedArgs {
    pub fn parse(args: &[String]) -> Self {
        let mut flags = HashMap::new();
        let mut switches = std::collections::HashSet::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if let Some(name) = arg.strip_prefix("--") {
                if matches!(name, "ancestral") {
                    switches.insert(name.to_string());
                    i += 1;
                    continue;
                }
                if let Some(value) = args.get(i + 1) {
                    flags.insert(name.to_string(), value.clone());
                    i += 2;
                } else {
                    switches.insert(name.to_string());
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        Self { flags, switches }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(|s| s.as_str())
    }

    pub fn get_or(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or(default).to_string()
    }

    pub fn path_or(&self, name: &str, default: &str) -> PathBuf {
        PathBuf::from(self.get_or(name, default))
    }

    pub fn has(&self, name: &str) -> bool {
        self.switches.contains(name) || self.flags.contains_key(name)
    }
}

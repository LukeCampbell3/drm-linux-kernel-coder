//! Snapshot format for [`crate::registry::Registry`]. Pure
//! serialize/deserialize -- no filesystem access here (this crate stays
//! I/O-free); atomic file writing lives in `drmd`, which owns the daemon's
//! state directory.
//!
//! The snapshot is a dedicated DTO, not a serialization of the live
//! `Registry`/`HybridPlanner` types directly: the live types carry
//! transient, mid-flight bookkeeping (deferred-consolidation queues,
//! frequency tables mid-count) that has no business surviving a restart
//! and would only risk resuming in a half-applied state. The snapshot
//! captures exactly the stable, meaningful state the spec asks for
//! (permanent + provisional vocabulary, history needed to resume scoring,
//! word metadata, application/workload ownership, birth context, usage
//! and transfer evidence, admission/expiry evidence) and every load goes
//! through a real reconstruction path, not a raw memory copy.
//!
//! Schema is versioned and checked strictly: [`Snapshot::validate_version`]
//! is the single gate every loader must call before trusting a snapshot's
//! contents. Never silently load incompatible state (spec §4).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::identity::ExecutionContext;
use crate::lifecycle::LifecycleStage;
use crate::registry::{AppState, GlobalVocabulary, Registry, WordMeta};
use crate::vocabulary::Seq;

/// Bump this whenever the snapshot shape changes in a way that would make
/// an old snapshot mean something different if loaded naively (renamed
/// field, changed semantics, removed stage, ...). Purely additive changes
/// that old loaders would still interpret correctly may keep the version,
/// but when in doubt, bump it -- the cost of a false-positive "start
/// fresh" is far lower than silently misinterpreting old state.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub struct SchemaMismatch {
    pub found: u32,
    pub expected: u32,
}

impl std::fmt::Display for SchemaMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "snapshot schema_version {} is incompatible with this build's {} -- refusing to load",
            self.found, self.expected
        )
    }
}

impl std::error::Error for SchemaMismatch {}

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub step: u64,
    pub applications: BTreeMap<String, AppSnapshot>,
    pub global: GlobalSnapshot,
}

impl Snapshot {
    /// The one gate every loader must pass through. An unknown or
    /// mismatched version is always an error, never a best-effort
    /// partial load.
    pub fn validate_version(&self) -> Result<(), SchemaMismatch> {
        if self.schema_version == SCHEMA_VERSION {
            Ok(())
        } else {
            Err(SchemaMismatch {
                found: self.schema_version,
                expected: SCHEMA_VERSION,
            })
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AppSnapshot {
    pub created_step: u64,
    pub permanent: BTreeMap<String, Seq>,
    pub permanent_counter: usize,
    pub provisional: BTreeMap<String, Seq>,
    pub history: BTreeMap<String, Seq>,
    pub word_meta: HashMap<String, WordMetaSnapshot>,
}

#[derive(Serialize, Deserialize)]
pub struct GlobalSnapshot {
    pub permanent: BTreeMap<String, Seq>,
    pub counter: usize,
    pub promotion_threshold_apps: usize,
    pub word_meta: HashMap<String, WordMetaSnapshot>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ExecutionContextSnapshot {
    pub host_id: String,
    pub user_scope: String,
    pub application_id: String,
    pub workload_id: String,
    pub task_id: String,
}

impl From<&ExecutionContext> for ExecutionContextSnapshot {
    fn from(c: &ExecutionContext) -> Self {
        Self {
            host_id: c.host_id.clone(),
            user_scope: c.user_scope.clone(),
            application_id: c.application_id.clone(),
            workload_id: c.workload_id.clone(),
            task_id: c.task_id.clone(),
        }
    }
}

impl From<ExecutionContextSnapshot> for ExecutionContext {
    fn from(s: ExecutionContextSnapshot) -> Self {
        ExecutionContext {
            host_id: s.host_id,
            user_scope: s.user_scope,
            application_id: s.application_id,
            workload_id: s.workload_id,
            task_id: s.task_id,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct WordMetaSnapshot {
    pub birth: ExecutionContextSnapshot,
    pub stage: String,
    pub created_step: u64,
    pub last_use_step: u64,
    pub usage_count: u64,
    pub used_by_tasks: Vec<String>,
    pub used_by_workloads: Vec<String>,
    pub admission_evidence: String,
    pub expiry_evidence: Option<String>,
}

fn stage_to_str(s: LifecycleStage) -> &'static str {
    use LifecycleStage::*;
    match s {
        Observed => "observed",
        Candidate => "candidate",
        Provisional => "provisional",
        Validating => "validating",
        Verified => "verified",
        Permanent => "permanent",
        Rejected => "rejected",
        Expired => "expired",
        RolledBack => "rolled_back",
    }
}

fn stage_from_str(s: &str) -> LifecycleStage {
    use LifecycleStage::*;
    match s {
        "observed" => Observed,
        "candidate" => Candidate,
        "provisional" => Provisional,
        "validating" => Validating,
        "verified" => Verified,
        "permanent" => Permanent,
        "rejected" => Rejected,
        "expired" => Expired,
        "rolled_back" => RolledBack,
        // An unrecognized stage string means an old snapshot predates a
        // stage that was added later. Treat conservatively as Observed
        // rather than guessing something more trusted -- never upgrade
        // trust on a version we don't fully understand.
        _ => Observed,
    }
}

fn word_meta_to_snapshot(m: &WordMeta) -> WordMetaSnapshot {
    WordMetaSnapshot {
        birth: (&m.birth).into(),
        stage: stage_to_str(m.stage).to_string(),
        created_step: m.created_step,
        last_use_step: m.last_use_step,
        usage_count: m.usage_count,
        used_by_tasks: m.used_by_tasks.iter().cloned().collect(),
        used_by_workloads: m.used_by_workloads.iter().cloned().collect(),
        admission_evidence: m.admission_evidence.clone(),
        expiry_evidence: m.expiry_evidence.clone(),
    }
}

fn word_meta_from_snapshot(s: WordMetaSnapshot) -> WordMeta {
    WordMeta {
        birth: s.birth.into(),
        stage: stage_from_str(&s.stage),
        created_step: s.created_step,
        last_use_step: s.last_use_step,
        usage_count: s.usage_count,
        used_by_tasks: s.used_by_tasks.into_iter().collect(),
        used_by_workloads: s.used_by_workloads.into_iter().collect(),
        admission_evidence: s.admission_evidence,
        expiry_evidence: s.expiry_evidence,
    }
}

pub fn to_snapshot(reg: &Registry) -> Snapshot {
    let applications = reg
        .applications
        .iter()
        .map(|(app_id, app)| {
            let permanent = app.planner.base.vocab.derived.clone().into_iter().collect();
            let provisional = app.planner.provisional_raw().into_iter().collect();
            let history = app.planner.base.history.clone().into_iter().collect();
            let word_meta = app.word_meta.iter().map(|(k, v)| (k.clone(), word_meta_to_snapshot(v))).collect();
            (
                app_id.clone(),
                AppSnapshot {
                    created_step: app.created_step,
                    permanent,
                    permanent_counter: app.planner.base.vocab.counter,
                    provisional,
                    history,
                    word_meta,
                },
            )
        })
        .collect();

    let global = GlobalSnapshot {
        permanent: reg.global.vocab.derived.clone().into_iter().collect(),
        counter: reg.global.counter(),
        promotion_threshold_apps: reg.global.promotion_threshold_apps,
        word_meta: reg
            .global
            .word_meta
            .iter()
            .map(|(k, v)| (k.clone(), word_meta_to_snapshot(v)))
            .collect(),
    };

    Snapshot {
        schema_version: SCHEMA_VERSION,
        step: reg.step,
        applications,
        global,
    }
}

/// Reconstruct a `Registry` from a snapshot already known to have passed
/// [`Snapshot::validate_version`]. Every application's `HybridPlanner` is
/// rebuilt from scratch and repopulated -- no live state is ever
/// deserialized directly.
pub fn from_snapshot(snap: Snapshot) -> Registry {
    let mut reg = Registry::new();
    reg.step = snap.step;

    reg.global = GlobalVocabulary::restore(
        snap.global.permanent.into_iter().collect(),
        snap.global.counter,
        snap.global.promotion_threshold_apps,
    );
    for (k, v) in snap.global.word_meta {
        reg.global.word_meta.insert(k, word_meta_from_snapshot(v));
    }

    for (app_id, app_snap) in snap.applications {
        let mut app = AppState::restored(app_snap.created_step);
        app.planner.restore_permanent(app_snap.permanent, app_snap.permanent_counter);
        app.planner.restore_provisional(app_snap.provisional);
        app.planner.restore_history(app_snap.history);
        for (k, v) in app_snap.word_meta {
            app.word_meta.insert(k, word_meta_from_snapshot(v));
        }
        reg.applications.insert(app_id, app);
    }

    reg.rebuild_cross_app_index();
    reg
}

pub fn to_json(snap: &Snapshot) -> serde_json::Result<String> {
    serde_json::to_string_pretty(snap)
}

pub fn from_json(s: &str) -> serde_json::Result<Snapshot> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::ExecutionContext;
    use crate::vocabulary::Seq;

    fn motif() -> Seq {
        vec![
            "fs.read".into(),
            "transform.extract".into(),
            "transform.summarize".into(),
            "fs.write".into(),
            "notify.send".into(),
        ]
    }

    #[test]
    fn round_trip_preserves_permanent_vocabulary_and_word_meta() {
        let mut reg = Registry::new();
        for i in 0..6 {
            reg.plan(&ExecutionContext::new("h", "u", "app", "wl", format!("t{i}")), motif(), "x", false);
        }
        reg.consolidate();
        let derived_before = reg.applications["app"].planner.base.vocab.derived.len();
        assert!(derived_before > 0, "test setup should have grown at least one permanent word");

        let snap = to_snapshot(&reg);
        let json = to_json(&snap).unwrap();
        let loaded_snap = from_json(&json).unwrap();
        loaded_snap.validate_version().unwrap();
        let restored = from_snapshot(loaded_snap);

        assert_eq!(restored.applications["app"].planner.base.vocab.derived.len(), derived_before);
        assert_eq!(
            restored.applications["app"].word_meta.len(),
            reg.applications["app"].word_meta.len()
        );
    }

    #[test]
    fn mismatched_schema_version_is_rejected_not_silently_loaded() {
        let mut snap = to_snapshot(&Registry::new());
        snap.schema_version = SCHEMA_VERSION + 1;
        let err = snap.validate_version().unwrap_err();
        assert_eq!(err.found, SCHEMA_VERSION + 1);
    }
}

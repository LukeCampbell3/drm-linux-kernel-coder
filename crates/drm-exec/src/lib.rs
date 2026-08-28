//! `drm-exec`: real-Linux execution of DRM O/D/C capabilities.
//!
//! [`executor::LiveExecutor`] gives every capability a real, observable
//! side effect against the host -- filesystem, `/proc`, a timer, loopback
//! sockets, and a spawned child process -- so that planning decisions made
//! by `drm-core` are backed by genuine execution, not a simulation.
//!
//! [`specialize::SpecializationSet`] is the opt-in bridge to `drm-opt`:
//! attached to a `LiveExecutor`, it lets `fs.read` and pure `transform.*`
//! chains actually take a verified specialized path (a cached read, a
//! memoized transform) instead of merely naming one. Unattached (the
//! default), `LiveExecutor` behaves exactly as it always has.

pub mod code;
pub mod executor;
pub mod servers;
pub mod specialize;
pub mod web;

pub use code::{evolve_task, MutationReport};
pub use executor::{make_fixtures, ExecError, LiveExecutor};
pub use servers::{TcpFixtureServer, UnixFixtureServer};
pub use specialize::SpecializationSet;
pub use web::WebConfig;

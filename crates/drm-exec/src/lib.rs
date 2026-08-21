//! `drm-exec`: real-Linux execution of DRM O/D/C capabilities.
//!
//! [`executor::LiveExecutor`] gives every capability a real, observable
//! side effect against the host -- filesystem, `/proc`, a timer, loopback
//! sockets, and a spawned child process -- so that planning decisions made
//! by `drm-core` are backed by genuine execution, not a simulation.

pub mod executor;
pub mod servers;

pub use executor::{make_fixtures, ExecError, LiveExecutor};
pub use servers::{TcpFixtureServer, UnixFixtureServer};

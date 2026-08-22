//! Thin clients for talking to a running `drmd serve` daemon over its
//! Unix socket -- one function per wire-protocol request kind (see
//! `protocol` module docs for the request grammar).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub struct SubmitArgs {
    pub task: String,
    pub ops: Vec<String>,
    pub source: String,
    pub output: String,
    pub url: String,
    pub ancestral: bool,
    pub application: Option<String>,
    pub workload: Option<String>,
    pub host: Option<String>,
    pub user: Option<String>,
}

fn round_trip(socket: &Path, request: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to connect to {} ({e}) -- is `drmd serve` running?", socket.display()),
        )
    })?;
    writeln!(stream, "{request}")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim_end().to_string())
}

pub fn submit(socket: &Path, args: &SubmitArgs) -> std::io::Result<String> {
    let mut req = format!("SUBMIT\ttask={}\tops={}", args.task, args.ops.join(","));
    if !args.source.is_empty() {
        req.push_str(&format!("\tsource={}", args.source));
    }
    if !args.output.is_empty() {
        req.push_str(&format!("\toutput={}", args.output));
    }
    if !args.url.is_empty() {
        req.push_str(&format!("\turl={}", args.url));
    }
    if args.ancestral {
        req.push_str("\tancestral=1");
    }
    if let Some(app) = &args.application {
        req.push_str(&format!("\tapp={app}"));
    }
    if let Some(wl) = &args.workload {
        req.push_str(&format!("\tworkload={wl}"));
    }
    if let Some(h) = &args.host {
        req.push_str(&format!("\thost={h}"));
    }
    if let Some(u) = &args.user {
        req.push_str(&format!("\tuser={u}"));
    }
    round_trip(socket, &req)
}

pub fn status(socket: &Path) -> std::io::Result<String> {
    round_trip(socket, "STATUS")
}

pub fn applications(socket: &Path) -> std::io::Result<String> {
    round_trip(socket, "APPLICATIONS")
}

pub fn application(socket: &Path, id: &str) -> std::io::Result<String> {
    round_trip(socket, &format!("APPLICATION\tid={id}"))
}

pub fn workload(socket: &Path, id: &str) -> std::io::Result<String> {
    round_trip(socket, &format!("WORKLOAD\tid={id}"))
}

pub fn learned(socket: &Path, app_filter: Option<&str>) -> std::io::Result<String> {
    match app_filter {
        Some(app) => round_trip(socket, &format!("LEARNED\tapp={app}")),
        None => round_trip(socket, "LEARNED"),
    }
}

pub fn optimizations(socket: &Path) -> std::io::Result<String> {
    round_trip(socket, "OPTIMIZATIONS")
}

pub fn metrics(socket: &Path) -> std::io::Result<String> {
    round_trip(socket, "METRICS")
}

pub fn explain(socket: &Path, id: &str) -> std::io::Result<String> {
    round_trip(socket, &format!("EXPLAIN\tid={id}"))
}

pub fn reset(socket: &Path, scope: &str) -> std::io::Result<String> {
    round_trip(socket, &format!("RESET\tscope={scope}"))
}

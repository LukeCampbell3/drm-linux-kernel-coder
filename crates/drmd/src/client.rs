//! `drmd submit` / `drmd status`: a thin client for talking to a running
//! `drmd serve` daemon over its Unix socket.

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
    round_trip(socket, &req)
}

pub fn status(socket: &Path) -> std::io::Result<String> {
    round_trip(socket, "STATUS")
}

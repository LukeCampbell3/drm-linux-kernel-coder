//! `drmd serve`'s wire protocol: one request per connection, one line in,
//! one line out.
//!
//! This is deliberately not JSON-in/JSON-out: requests only ever carry
//! plain identifiers and filesystem-relative paths (no embedded tabs or
//! newlines expected), so a trivial `key=value` line -- tab-separated,
//! first field is the command -- avoids pulling in a JSON parser for
//! input we fully control the shape of. Responses, which do need to carry
//! arbitrary text (error messages), are emitted as JSON via
//! [`crate::fmt`].
//!
//! ```text
//! SUBMIT\ttask=<id>\tops=<cap1,cap2,...>\t[app=<id>]\t[workload=<id>]\t[host=<id>]\t[user=<id>]\t[source=<path>]\t[output=<path>]\t[url=<path>]\t[ancestral=1]
//! STATUS
//! APPLICATIONS
//! APPLICATION\tid=<application_id>
//! WORKLOAD\tid=<workload_id>
//! LEARNED\t[app=<application_id>]
//! OPTIMIZATIONS
//! METRICS
//! EXPLAIN\tid=<optimization_id>
//! RESET\tscope=<application:<id>|provisional:<id>|global|all>
//! ```
//!
//! `app`/`workload`/`host`/`user` default to `"default"` when omitted --
//! Phase 1 callers that only ever specified a bare task still work
//! unchanged, just as a single-application, single-workload registry.

use drm_core::{Episode, ExecutionContext};

#[derive(Debug)]
pub enum Request {
    Submit(Box<Episode>),
    Status,
    Applications,
    Application(String),
    Workload(String),
    Learned(Option<String>),
    Optimizations,
    Metrics,
    Explain(String),
    Reset(String),
}

#[derive(Debug)]
pub enum ProtocolError {
    Empty,
    UnknownCommand(String),
    MissingField(&'static str),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::Empty => write!(f, "empty request"),
            ProtocolError::UnknownCommand(c) => write!(f, "unknown command `{c}`"),
            ProtocolError::MissingField(name) => write!(f, "missing required field `{name}`"),
        }
    }
}

impl std::error::Error for ProtocolError {}

fn fields(parts: std::str::Split<'_, char>) -> std::collections::HashMap<&str, &str> {
    parts.filter_map(|f| f.split_once('=')).collect()
}

pub fn parse_request(line: &str, next_idx: usize) -> Result<Request, ProtocolError> {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut parts = line.split('\t');
    let command = parts.next().ok_or(ProtocolError::Empty)?;
    match command {
        "STATUS" => Ok(Request::Status),
        "APPLICATIONS" => Ok(Request::Applications),
        "OPTIMIZATIONS" => Ok(Request::Optimizations),
        "METRICS" => Ok(Request::Metrics),
        "APPLICATION" => {
            let f = fields(parts);
            let id = f.get("id").ok_or(ProtocolError::MissingField("id"))?;
            Ok(Request::Application(id.to_string()))
        }
        "WORKLOAD" => {
            let f = fields(parts);
            let id = f.get("id").ok_or(ProtocolError::MissingField("id"))?;
            Ok(Request::Workload(id.to_string()))
        }
        "LEARNED" => {
            let f = fields(parts);
            Ok(Request::Learned(f.get("app").map(|s| s.to_string())))
        }
        "EXPLAIN" => {
            let f = fields(parts);
            let id = f.get("id").ok_or(ProtocolError::MissingField("id"))?;
            Ok(Request::Explain(id.to_string()))
        }
        "RESET" => {
            let f = fields(parts);
            let scope = f.get("scope").ok_or(ProtocolError::MissingField("scope"))?;
            Ok(Request::Reset(scope.to_string()))
        }
        "SUBMIT" => {
            let f = fields(parts);
            let task = f.get("task").ok_or(ProtocolError::MissingField("task"))?.to_string();
            let ops: Vec<String> = f
                .get("ops")
                .map(|v| v.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
                .unwrap_or_default();
            if ops.is_empty() {
                return Err(ProtocolError::MissingField("ops"));
            }
            let source = f.get("source").unwrap_or(&"").to_string();
            let mut output = f.get("output").unwrap_or(&"").to_string();
            let mut url = f.get("url").unwrap_or(&"").to_string();
            let ancestral = f
                .get("ancestral")
                .map(|v| *v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if output.is_empty() {
                output = format!("outputs/{task}.txt");
            }
            if url.is_empty() {
                url = "/news_0.html".to_string();
            }
            let workload = f.get("workload").map(|s| s.to_string()).unwrap_or_else(|| task.clone());
            let ctx = ExecutionContext::new(
                *f.get("host").unwrap_or(&"default"),
                *f.get("user").unwrap_or(&"default"),
                *f.get("app").unwrap_or(&"default"),
                workload,
                task,
            );
            let mut episode = Episode::with_ctx(next_idx, ctx, "serve", ops);
            episode.source = source;
            episode.output = output;
            episode.url_path = url;
            episode.ancestral = ancestral;
            Ok(Request::Submit(Box::new(episode)))
        }
        other => Err(ProtocolError::UnknownCommand(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_submit_line() {
        let req = parse_request("SUBMIT\ttask=t\tops=fs.read,fs.write\tsource=in.csv", 1).unwrap();
        match req {
            Request::Submit(ep) => {
                assert_eq!(ep.task(), "t");
                assert_eq!(ep.ctx.application_id, "default");
                assert_eq!(ep.ops, vec!["fs.read", "fs.write"]);
                assert_eq!(ep.source, "in.csv");
                assert_eq!(ep.output, "outputs/t.txt");
            }
            _ => panic!("expected Submit"),
        }
    }

    #[test]
    fn parses_full_identity_on_submit() {
        let req = parse_request(
            "SUBMIT\ttask=t1\tops=fs.read,fs.write\tapp=nginx\tworkload=api_get\thost=srv1\tuser=svc",
            1,
        )
        .unwrap();
        match req {
            Request::Submit(ep) => {
                assert_eq!(ep.ctx.application_id, "nginx");
                assert_eq!(ep.ctx.workload_id, "api_get");
                assert_eq!(ep.ctx.host_id, "srv1");
                assert_eq!(ep.ctx.user_scope, "svc");
            }
            _ => panic!("expected Submit"),
        }
    }

    #[test]
    fn rejects_submit_without_ops() {
        assert!(matches!(
            parse_request("SUBMIT\ttask=t", 1),
            Err(ProtocolError::MissingField("ops"))
        ));
    }

    #[test]
    fn status_has_no_fields() {
        assert!(matches!(parse_request("STATUS", 1), Ok(Request::Status)));
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(matches!(parse_request("FOO", 1), Err(ProtocolError::UnknownCommand(_))));
    }

    #[test]
    fn application_requires_id() {
        assert!(matches!(parse_request("APPLICATION", 1), Err(ProtocolError::MissingField("id"))));
    }

    #[test]
    fn learned_app_filter_is_optional() {
        assert!(matches!(parse_request("LEARNED", 1), Ok(Request::Learned(None))));
        assert!(matches!(parse_request("LEARNED\tapp=nginx", 1), Ok(Request::Learned(Some(_)))));
    }
}

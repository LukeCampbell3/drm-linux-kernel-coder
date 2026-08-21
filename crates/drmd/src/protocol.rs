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
//! SUBMIT\ttask=<id>\tops=<cap1,cap2,...>\t[source=<path>]\t[output=<path>]\t[url=<path>]\t[ancestral=1]
//! STATUS
//! ```

use drm_core::Episode;

#[derive(Debug)]
pub enum Request {
    Submit(Episode),
    Status,
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

pub fn parse_request(line: &str, next_idx: usize) -> Result<Request, ProtocolError> {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut parts = line.split('\t');
    let command = parts.next().ok_or(ProtocolError::Empty)?;
    match command {
        "STATUS" => Ok(Request::Status),
        "SUBMIT" => {
            let mut task = None;
            let mut ops: Vec<String> = Vec::new();
            let mut source = String::new();
            let mut output = String::new();
            let mut url = String::new();
            let mut ancestral = false;
            for field in parts {
                let Some((key, value)) = field.split_once('=') else { continue };
                match key {
                    "task" => task = Some(value.to_string()),
                    "ops" => ops = value.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect(),
                    "source" => source = value.to_string(),
                    "output" => output = value.to_string(),
                    "url" => url = value.to_string(),
                    "ancestral" => ancestral = value == "1" || value.eq_ignore_ascii_case("true"),
                    _ => {}
                }
            }
            let task = task.ok_or(ProtocolError::MissingField("task"))?;
            if ops.is_empty() {
                return Err(ProtocolError::MissingField("ops"));
            }
            if output.is_empty() {
                output = format!("outputs/{task}.txt");
            }
            if url.is_empty() {
                url = "/news_0.html".to_string();
            }
            Ok(Request::Submit(Episode {
                idx: next_idx,
                task,
                phase: "serve".to_string(),
                ops,
                source,
                output,
                url_path: url,
                ancestral,
            }))
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
                assert_eq!(ep.task, "t");
                assert_eq!(ep.ops, vec!["fs.read", "fs.write"]);
                assert_eq!(ep.source, "in.csv");
                assert_eq!(ep.output, "outputs/t.txt");
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
}

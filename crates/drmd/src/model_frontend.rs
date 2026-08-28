use std::process::Command;
use std::time::{Duration, Instant};

const CAPABILITIES: &[&str] = &["task.watch", "app.execute", "web.selenium", "code.evolve"];
const DECISIONS: &[&str] = &["watch", "execute", "clarify", "deny"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    pub decision: String,
    pub family: String,
    pub capability: String,
    pub confidence_milli: u16,
}

#[derive(Clone, Debug)]
pub struct ModelResult {
    pub plan: Plan,
    pub elapsed: Duration,
    pub provider: String,
}

pub fn assist(goal: &str, provider: &str) -> Result<ModelResult, String> {
    if goal.trim().is_empty() || goal.len() > 4096 {
        return Err("goal must contain 1..4096 bytes".into());
    }
    if !matches!(provider, "glm" | "qwen") {
        return Err("provider must be glm or qwen".into());
    }
    let adapter =
        std::env::var("DRMD_MODEL_ADAPTER").map_err(|_| "DRMD_MODEL_ADAPTER must name an operator-installed adapter".to_string())?;
    let started = Instant::now();
    let output = Command::new(adapter)
        .args(["--provider", provider, "--goal", goal])
        .output()
        .map_err(|error| format!("model adapter failed to start: {error}"))?;
    let elapsed = started.elapsed();
    if !output.status.success() {
        return Err(format!("model adapter exited with {}", output.status));
    }
    if output.stdout.len() > 8192 {
        return Err("model adapter response exceeds 8192 bytes".into());
    }
    let text = String::from_utf8(output.stdout).map_err(|_| "model adapter response is not UTF-8")?;
    Ok(ModelResult {
        plan: parse_and_guard(&text)?,
        elapsed,
        provider: provider.into(),
    })
}

pub fn parse_and_guard(text: &str) -> Result<Plan, String> {
    let field = |name: &str| -> Result<&str, String> {
        let prefix = format!("{name}=");
        let values: Vec<&str> = text.lines().filter_map(|line| line.strip_prefix(&prefix)).collect();
        if values.len() == 1 {
            Ok(values[0].trim())
        } else {
            Err(format!("expected exactly one {name} field"))
        }
    };
    let decision = field("decision")?;
    let family = field("family")?;
    let capability = field("capability")?;
    let confidence = field("confidence_milli")?.parse::<u16>().map_err(|_| "invalid confidence_milli")?;
    if !DECISIONS.contains(&decision) {
        return Err("unknown decision".into());
    }
    if !CAPABILITIES.contains(&capability) {
        return Err("unknown capability".into());
    }
    if family.is_empty() || family.len() > 96 || !family.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err("family must be a safe 1..96 byte identifier".into());
    }
    if confidence > 1000 {
        return Err("confidence_milli must be <= 1000".into());
    }
    if decision == "execute" && capability != "app.execute" {
        return Err("execute may only select app.execute".into());
    }
    if decision == "watch" && capability != "task.watch" {
        return Err("watch may only select task.watch".into());
    }
    if matches!(decision, "clarify" | "deny") && capability == "app.execute" {
        return Err("non-execution decisions cannot select app.execute".into());
    }
    Ok(Plan {
        decision: decision.into(),
        family: family.into(),
        capability: capability.into(),
        confidence_milli: confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_plan() {
        let plan = parse_and_guard("decision=watch\nfamily=calendar_to_notes\ncapability=task.watch\nconfidence_milli=830\n").unwrap();
        assert_eq!(plan.decision, "watch");
    }

    #[test]
    fn rejects_model_requested_direct_execution() {
        assert!(parse_and_guard("decision=execute\nfamily=x\ncapability=web.selenium\nconfidence_milli=999\n").is_err());
    }
}

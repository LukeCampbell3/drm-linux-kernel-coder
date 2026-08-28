//! Policy and process boundary for Selenium-backed public-web access.

use std::path::PathBuf;
use std::process::Command;

use crate::executor::ExecError;

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub python: PathBuf,
    pub bridge: PathBuf,
    pub webdriver_url: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub allow_private: bool,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
}

impl WebConfig {
    pub fn from_env() -> Option<Self> {
        let hosts = std::env::var("DRMD_WEB_ALLOWED_HOSTS").ok()?;
        let allowed_hosts = hosts
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        Some(Self {
            python: std::env::var_os("DRMD_WEB_PYTHON")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("python3")),
            bridge: std::env::var_os("DRMD_SELENIUM_BRIDGE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/usr/local/lib/drmd/selenium_bridge.py")),
            webdriver_url: std::env::var("DRMD_WEBDRIVER_URL").ok(),
            allowed_hosts,
            allow_private: std::env::var("DRMD_WEB_ALLOW_PRIVATE").as_deref() == Ok("1"),
            timeout_secs: env_number("DRMD_WEB_TIMEOUT_SECS", 20),
            max_output_bytes: env_number("DRMD_WEB_MAX_OUTPUT_BYTES", 1_000_000),
        })
    }

    pub fn fetch(&self, url: &str, application_id: &str) -> Result<String, ExecError> {
        let host = validated_host(url)?;
        if !self.allow_private && is_private_host(&host) {
            return Err(ExecError::WebDenied(format!("private or local host `{host}` is blocked")));
        }
        if !self.allowed_hosts.iter().any(|rule| host_matches(&host, rule)) {
            return Err(ExecError::WebDenied(format!("host `{host}` is not in DRMD_WEB_ALLOWED_HOSTS")));
        }

        let mut command = Command::new(&self.python);
        command
            .arg(&self.bridge)
            .arg("--url")
            .arg(url)
            .arg("--application-id")
            .arg(application_id)
            .arg("--timeout")
            .arg(self.timeout_secs.to_string())
            .arg("--max-output-bytes")
            .arg(self.max_output_bytes.to_string());
        if let Some(endpoint) = &self.webdriver_url {
            command.arg("--webdriver-url").arg(endpoint);
        }
        let output = command.output()?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().chars().take(500).collect::<String>();
            return Err(ExecError::WebBridge(if detail.is_empty() {
                "Selenium bridge failed".into()
            } else {
                detail
            }));
        }
        if output.stdout.len() > self.max_output_bytes {
            return Err(ExecError::WebBridge("Selenium bridge exceeded its output limit".into()));
        }
        String::from_utf8(output.stdout).map_err(|_| ExecError::WebBridge("Selenium bridge returned non-UTF-8 output".into()))
    }
}

fn env_number<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn validated_host(url: &str) -> Result<String, ExecError> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| ExecError::WebDenied("only http:// and https:// URLs are allowed".into()))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(ExecError::WebDenied("URL must contain a host and no credentials".into()));
    }
    let host = if authority.starts_with('[') {
        authority.strip_prefix('[').and_then(|v| v.split(']').next()).unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    }
    .trim_end_matches('.')
    .to_ascii_lowercase();
    if host.is_empty() {
        Err(ExecError::WebDenied("URL host is empty".into()))
    } else {
        Ok(host)
    }
}

fn host_matches(host: &str, rule: &str) -> bool {
    let rule = rule.trim().trim_end_matches('.').to_ascii_lowercase();
    rule == "*" || host == rule || rule.strip_prefix("*.").is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
}

fn is_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_policy_is_exact_or_explicit_wildcard() {
        assert!(host_matches("docs.example.com", "*.example.com"));
        assert!(!host_matches("example.com", "*.example.com"));
        assert!(!host_matches("badexample.com", "*.example.com"));
    }

    #[test]
    fn blocks_unsafe_url_shapes_and_private_ips() {
        assert!(validated_host("file:///etc/passwd").is_err());
        assert!(validated_host("https://user:pass@example.com").is_err());
        assert!(is_private_host("127.0.0.1"));
        assert!(is_private_host("::1"));
    }
}

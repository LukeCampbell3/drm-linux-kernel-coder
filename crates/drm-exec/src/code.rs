//! Guarded, transactional source-code patching for explicitly authorized DRM instances.

use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::executor::ExecError;

#[derive(Clone, Debug)]
pub struct CodeConfig {
    pub root: PathBuf,
    pub allowed_paths: Vec<PathBuf>,
    pub max_patch_bytes: usize,
    pub allow_delete: bool,
    pub verify_program: PathBuf,
    pub verify_args: Vec<String>,
}

impl CodeConfig {
    pub fn from_env() -> Option<Self> {
        let root = PathBuf::from(std::env::var_os("DRMD_CODE_ROOT")?);
        let allowed_paths = std::env::var("DRMD_CODE_ALLOWED_PATHS")
            .ok()?
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect();
        Some(Self {
            root,
            allowed_paths,
            max_patch_bytes: env_number("DRMD_CODE_MAX_PATCH_BYTES", 262_144),
            allow_delete: std::env::var("DRMD_CODE_ALLOW_DELETE").as_deref() == Ok("1"),
            verify_program: std::env::var_os("DRMD_CODE_VERIFY_PROGRAM")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("cargo")),
            verify_args: std::env::var("DRMD_CODE_VERIFY_ARGS")
                .unwrap_or_else(|_| "test --workspace".into())
                .split_whitespace()
                .map(str::to_string)
                .collect(),
        })
    }

    pub fn apply(&self, patch: &[u8]) -> Result<(), ExecError> {
        if patch.is_empty() || patch.len() > self.max_patch_bytes {
            return Err(ExecError::CodeDenied(format!(
                "patch size must be between 1 and {} bytes",
                self.max_patch_bytes
            )));
        }
        let patch_text = std::str::from_utf8(patch).map_err(|_| ExecError::CodeDenied("patch must be UTF-8 text".into()))?;
        if patch_text.contains("GIT binary patch") || patch_text.contains("Binary files ") {
            return Err(ExecError::CodeDenied("binary patches are not permitted".into()));
        }

        let root = self.root.canonicalize()?;
        ensure_git_root(&root)?;
        let targets = patch_targets(patch_text, self.allow_delete)?;
        for target in &targets {
            validate_target(&root, target, &self.allowed_paths)?;
            ensure_target_clean(&root, target)?;
        }

        run_git_apply(&root, patch, true, false)?;
        run_git_apply(&root, patch, false, false)?;

        let verified = Command::new(&self.verify_program)
            .args(&self.verify_args)
            .current_dir(&root)
            .status();
        match verified {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => rollback(&root, patch, format!("verification exited with {status}")),
            Err(error) => rollback(&root, patch, format!("verification could not start: {error}")),
        }
    }
}

fn env_number<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

fn ensure_git_root(root: &Path) -> Result<(), ExecError> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(ExecError::CodeDenied("DRMD_CODE_ROOT is not a Git worktree".into()));
    }
    let discovered = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if discovered.canonicalize()? != root {
        return Err(ExecError::CodeDenied("DRMD_CODE_ROOT must be the Git worktree root".into()));
    }
    Ok(())
}

fn patch_targets(patch: &str, allow_delete: bool) -> Result<Vec<PathBuf>, ExecError> {
    let mut targets = Vec::new();
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            let mut parts = line.split_whitespace();
            let old = parts.nth(2).unwrap_or_default();
            let new = parts.next().unwrap_or_default();
            for raw in [old, new] {
                if raw != "/dev/null" {
                    let path = raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/")).unwrap_or(raw);
                    targets.push(PathBuf::from(path));
                }
            }
        }
        if line == "+++ /dev/null" && !allow_delete {
            return Err(ExecError::CodeDenied("file deletion requires DRMD_CODE_ALLOW_DELETE=1".into()));
        }
    }
    if targets.is_empty() {
        return Err(ExecError::CodeDenied("patch has no unified-diff targets".into()));
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}

fn validate_target(root: &Path, target: &Path, allowed: &[PathBuf]) -> Result<(), ExecError> {
    if target.is_absolute()
        || target
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
        || target.starts_with(".git")
    {
        return Err(ExecError::CodeDenied(format!("unsafe target `{}`", target.display())));
    }
    if !allowed.iter().any(|prefix| target.starts_with(prefix)) {
        return Err(ExecError::CodeDenied(format!(
            "target `{}` is outside DRMD_CODE_ALLOWED_PATHS",
            target.display()
        )));
    }
    let mut cursor = root.to_path_buf();
    for component in target.components() {
        cursor.push(component);
        if cursor.symlink_metadata().is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(ExecError::CodeDenied(format!(
                "symlink target component `{}` is blocked",
                cursor.display()
            )));
        }
    }
    Ok(())
}

fn ensure_target_clean(root: &Path, target: &Path) -> Result<(), ExecError> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--"])
        .arg(target)
        .current_dir(root)
        .output()?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(ExecError::CodeDenied(format!(
            "target `{}` has pre-existing changes",
            target.display()
        )));
    }
    Ok(())
}

fn run_git_apply(root: &Path, patch: &[u8], check: bool, reverse: bool) -> Result<(), ExecError> {
    let mut command = Command::new("git");
    command.args(["apply", "--whitespace=error-all"]);
    if check {
        command.arg("--check");
    }
    if reverse {
        command.arg("--reverse");
    }
    let mut child = command.current_dir(root).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    child.stdin.as_mut().expect("piped stdin").write_all(patch)?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ExecError::CodePatch(
            String::from_utf8_lossy(&output.stderr).trim().chars().take(1000).collect(),
        ))
    }
}

fn rollback(root: &Path, patch: &[u8], reason: String) -> Result<(), ExecError> {
    match run_git_apply(root, patch, false, true) {
        Ok(()) => Err(ExecError::CodeVerification(format!("{reason}; patch rolled back"))),
        Err(rollback_error) => Err(ExecError::CodeVerification(format!("{reason}; ROLLBACK FAILED: {rollback_error}"))),
    }
}

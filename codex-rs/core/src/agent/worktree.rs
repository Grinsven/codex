use crate::config::AgentWorktreeCleanup;
use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use tokio::process::Command;

pub(crate) const DEFAULT_WORKTREE_BASE_REF: &str = "HEAD";
pub(crate) const DEFAULT_WORKTREE_ROOT_DIR_NAME: &str = "agent-worktrees";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentWorktreeCleanupStatus {
    Pending,
    Removed,
    Kept,
    RemoveFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AgentWorktreeInfo {
    pub(crate) repo_root: PathBuf,
    pub(crate) worktree_path: PathBuf,
    pub(crate) branch: String,
    pub(crate) base_ref: String,
    pub(crate) cleanup: AgentWorktreeCleanup,
    pub(crate) cleanup_status: AgentWorktreeCleanupStatus,
    pub(crate) cleanup_error: Option<String>,
    pub(crate) tip_commit: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProvisionAgentWorktreeRequest {
    pub(crate) cwd: PathBuf,
    pub(crate) worktree_root_dir: PathBuf,
    pub(crate) owner_thread_id: ThreadId,
    pub(crate) call_id: String,
    pub(crate) base_ref: Option<String>,
    pub(crate) branch_name: Option<String>,
    pub(crate) cleanup: AgentWorktreeCleanup,
}

pub(crate) async fn provision_agent_worktree(
    request: ProvisionAgentWorktreeRequest,
) -> Result<AgentWorktreeInfo, String> {
    let repo_root = resolve_repo_root(&request.cwd).await?;
    tokio::fs::create_dir_all(&request.worktree_root_dir)
        .await
        .map_err(|err| format!("failed to create worktree root dir: {err}"))?;

    let base_ref = request
        .base_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| DEFAULT_WORKTREE_BASE_REF.to_string());
    let explicit_branch = request.branch_name.as_deref().map(sanitize_branch_name);

    let max_attempts = 128;
    for attempt in 0..max_attempts {
        let branch = match explicit_branch.as_deref() {
            Some(branch) => branch.to_string(),
            None => default_branch_name(request.owner_thread_id, &request.call_id, attempt),
        };

        if branch_exists(&repo_root, &branch).await? {
            if explicit_branch.is_some() {
                return Err(format!("git branch already exists: {branch}"));
            }
            continue;
        }

        let worktree_path =
            match worktree_path_for_branch(&request.worktree_root_dir, &branch, attempt).await {
                Ok(path) => path,
                Err(err) => {
                    if explicit_branch.is_some() {
                        return Err(err);
                    }
                    continue;
                }
            };

        match add_worktree(&repo_root, &worktree_path, &branch, &base_ref).await {
            Ok(()) => {
                let tip_commit = current_head_commit(&worktree_path).await.ok();
                return Ok(AgentWorktreeInfo {
                    repo_root,
                    worktree_path,
                    branch,
                    base_ref,
                    cleanup: request.cleanup,
                    cleanup_status: AgentWorktreeCleanupStatus::Pending,
                    cleanup_error: None,
                    tip_commit,
                });
            }
            Err(err) => {
                if explicit_branch.is_some() {
                    return Err(err);
                }
                let lower_err = err.to_lowercase();
                if lower_err.contains("already exists")
                    || lower_err.contains("is already checked out")
                {
                    continue;
                }
                return Err(err);
            }
        }
    }

    Err("failed to provision an isolated worktree after multiple attempts".to_string())
}

pub(crate) async fn finalize_agent_worktree(
    worktree: &mut AgentWorktreeInfo,
    force_remove: bool,
) -> Result<(), String> {
    if !matches!(worktree.cleanup_status, AgentWorktreeCleanupStatus::Pending) {
        return Ok(());
    }

    if let Ok(tip_commit) = current_head_commit(&worktree.worktree_path).await {
        worktree.tip_commit = Some(tip_commit);
    }

    let should_remove =
        force_remove || matches!(worktree.cleanup, AgentWorktreeCleanup::AutoRemoveWorktree);
    if !should_remove {
        worktree.cleanup_status = AgentWorktreeCleanupStatus::Kept;
        worktree.cleanup_error = None;
        return Ok(());
    }

    if !path_exists(&worktree.worktree_path).await {
        worktree.cleanup_status = AgentWorktreeCleanupStatus::Removed;
        worktree.cleanup_error = None;
        return Ok(());
    }

    match remove_worktree(&worktree.repo_root, &worktree.worktree_path).await {
        Ok(()) => {
            worktree.cleanup_status = AgentWorktreeCleanupStatus::Removed;
            worktree.cleanup_error = None;
            Ok(())
        }
        Err(err) => {
            worktree.cleanup_status = AgentWorktreeCleanupStatus::RemoveFailed;
            worktree.cleanup_error = Some(err.clone());
            Err(err)
        }
    }
}

fn sanitize_branch_name(branch: &str) -> String {
    let mut out = String::with_capacity(branch.len());
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }

    let trimmed = out
        .trim_matches(|ch| matches!(ch, '-' | '/' | '.'))
        .to_string();
    if trimmed.is_empty() {
        return "codex/agent/generated".to_string();
    }

    let normalized = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");

    if normalized.is_empty() {
        "codex/agent/generated".to_string()
    } else {
        normalized
    }
}

fn default_branch_name(owner_thread_id: ThreadId, call_id: &str, attempt: usize) -> String {
    let owner = owner_thread_id.to_string();
    let call = sanitize_branch_name(call_id);
    let call = call.replace('/', "-");
    if attempt == 0 {
        format!("codex/agent/{owner}/{call}")
    } else {
        format!("codex/agent/{owner}/{call}-{attempt}")
    }
}

async fn worktree_path_for_branch(
    worktree_root_dir: &Path,
    branch: &str,
    attempt: usize,
) -> Result<PathBuf, String> {
    let mut stem = branch.replace('/', "__");
    if attempt > 0 {
        stem = format!("{stem}--{attempt}");
    }
    let candidate = worktree_root_dir.join(stem);
    if path_exists(&candidate).await {
        return Err(format!(
            "worktree path already exists: {}",
            candidate.display()
        ));
    }
    Ok(candidate)
}

async fn resolve_repo_root(cwd: &Path) -> Result<PathBuf, String> {
    let output = run_git(cwd, ["rev-parse", "--show-toplevel"]).await?;
    if !output.status.success() {
        return Err("worktree isolation requires cwd to be inside a git repository".to_string());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| format!("invalid UTF-8 from git repo root query: {err}"))?;
    let root = stdout.trim();
    if root.is_empty() {
        return Err("git repo root query returned an empty path".to_string());
    }
    Ok(PathBuf::from(root))
}

async fn branch_exists(repo_root: &Path, branch: &str) -> Result<bool, String> {
    let output = run_git(
        repo_root,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await?;
    Ok(output.status.success())
}

async fn add_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    base_ref: &str,
) -> Result<(), String> {
    let path_str = path_to_utf8(worktree_path)?;
    let output = run_git(
        repo_root,
        ["worktree", "add", "-b", branch, &path_str, base_ref],
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    Err(command_stderr(
        "git worktree add failed",
        output.stderr,
        Some(output.stdout),
    ))
}

async fn remove_worktree(repo_root: &Path, worktree_path: &Path) -> Result<(), String> {
    let path_str = path_to_utf8(worktree_path)?;
    let output = run_git(repo_root, ["worktree", "remove", "--force", &path_str]).await?;
    if output.status.success() {
        return Ok(());
    }
    Err(command_stderr(
        "git worktree remove failed",
        output.stderr,
        Some(output.stdout),
    ))
}

async fn current_head_commit(cwd: &Path) -> Result<String, String> {
    let output = run_git(cwd, ["rev-parse", "HEAD"]).await?;
    if !output.status.success() {
        return Err(command_stderr(
            "git rev-parse HEAD failed",
            output.stderr,
            Some(output.stdout),
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| format!("invalid UTF-8 from git rev-parse HEAD: {err}"))?;
    let commit = stdout.trim();
    if commit.is_empty() {
        return Err("git rev-parse HEAD returned an empty commit".to_string());
    }
    Ok(commit.to_string())
}

async fn run_git<I, S>(cwd: &Path, args: I) -> Result<std::process::Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|err| format!("failed to run git command: {err}"))
}

fn command_stderr(prefix: &str, stderr: Vec<u8>, stdout: Option<Vec<u8>>) -> String {
    let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
    if !stderr.is_empty() {
        return format!("{prefix}: {stderr}");
    }
    if let Some(stdout) = stdout {
        let stdout = String::from_utf8_lossy(&stdout).trim().to_string();
        if !stdout.is_empty() {
            return format!("{prefix}: {stdout}");
        }
    }
    prefix.to_string()
}

fn path_to_utf8(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

async fn path_exists(path: &Path) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn run_git(repo: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn setup_repo() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        run_git(&repo_root, &["init", "--initial-branch=main"]);
        run_git(&repo_root, &["config", "user.name", "Test User"]);
        run_git(&repo_root, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        run_git(&repo_root, &["add", "README.md"]);
        run_git(&repo_root, &["commit", "-m", "init"]);
        (temp_dir, repo_root)
    }

    #[tokio::test]
    async fn provision_creates_worktree_and_branch() {
        let (_temp_dir, repo_root) = setup_repo();
        let worktree_root_dir = repo_root.join(".codex-worktrees");
        let request = ProvisionAgentWorktreeRequest {
            cwd: repo_root.clone(),
            worktree_root_dir: worktree_root_dir.clone(),
            owner_thread_id: ThreadId::new(),
            call_id: "call-1".to_string(),
            base_ref: Some("HEAD".to_string()),
            branch_name: None,
            cleanup: AgentWorktreeCleanup::AutoRemoveWorktree,
        };

        let worktree = provision_agent_worktree(request)
            .await
            .expect("provision worktree");
        assert!(worktree.worktree_path.starts_with(&worktree_root_dir));
        assert!(path_exists(&worktree.worktree_path).await);
        assert!(worktree.branch.starts_with("codex/agent/"));
        assert!(worktree.tip_commit.is_some());
    }

    #[tokio::test]
    async fn finalize_keeps_worktree_when_policy_is_keep() {
        let (_temp_dir, repo_root) = setup_repo();
        let worktree_root_dir = repo_root.join(".codex-worktrees");
        let request = ProvisionAgentWorktreeRequest {
            cwd: repo_root.clone(),
            worktree_root_dir,
            owner_thread_id: ThreadId::new(),
            call_id: "call-2".to_string(),
            base_ref: None,
            branch_name: Some("codex/agent/custom".to_string()),
            cleanup: AgentWorktreeCleanup::KeepWorktree,
        };

        let mut worktree = provision_agent_worktree(request)
            .await
            .expect("provision worktree");
        finalize_agent_worktree(&mut worktree, false)
            .await
            .expect("finalize worktree");

        assert_eq!(worktree.cleanup_status, AgentWorktreeCleanupStatus::Kept);
        assert!(path_exists(&worktree.worktree_path).await);
    }

    #[tokio::test]
    async fn finalize_removes_worktree_when_policy_is_auto_remove() {
        let (_temp_dir, repo_root) = setup_repo();
        let worktree_root_dir = repo_root.join(".codex-worktrees");
        let request = ProvisionAgentWorktreeRequest {
            cwd: repo_root,
            worktree_root_dir,
            owner_thread_id: ThreadId::new(),
            call_id: "call-3".to_string(),
            base_ref: None,
            branch_name: None,
            cleanup: AgentWorktreeCleanup::AutoRemoveWorktree,
        };

        let mut worktree = provision_agent_worktree(request)
            .await
            .expect("provision worktree");
        let worktree_path = worktree.worktree_path.clone();
        finalize_agent_worktree(&mut worktree, false)
            .await
            .expect("finalize worktree");

        assert_eq!(worktree.cleanup_status, AgentWorktreeCleanupStatus::Removed);
        assert!(!path_exists(&worktree_path).await);
    }
}

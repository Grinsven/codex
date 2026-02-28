use codex_protocol::protocol::AgentStatus;

/// Helpers for model-visible session state markers that are stored in user-role
/// messages but are not user intent.
use crate::agent::worktree::AgentWorktreeInfo;
use crate::contextual_user_message::SUBAGENT_NOTIFICATION_FRAGMENT;

pub(crate) fn format_subagent_notification_message(
    agent_id: &str,
    status: &AgentStatus,
    worktree: Option<&AgentWorktreeInfo>,
) -> String {
    let mut payload = serde_json::json!({
        "agent_id": agent_id,
        "status": status,
    });
    if let Some(worktree) = worktree {
        payload["worktree"] = serde_json::json!({
            "repo_root": worktree.repo_root,
            "worktree_path": worktree.worktree_path,
            "branch": worktree.branch,
            "base_ref": worktree.base_ref,
            "tip_commit": worktree.tip_commit,
            "cleanup": worktree.cleanup,
            "cleanup_status": worktree.cleanup_status,
            "cleanup_error": worktree.cleanup_error,
        });
    }
    let payload_json = payload.to_string();
    SUBAGENT_NOTIFICATION_FRAGMENT.wrap(payload_json)
}

pub(crate) fn format_subagent_context_line(agent_id: &str, agent_nickname: Option<&str>) -> String {
    match agent_nickname.filter(|nickname| !nickname.is_empty()) {
        Some(agent_nickname) => format!("- {agent_id}: {agent_nickname}"),
        None => format!("- {agent_id}"),
    }
}

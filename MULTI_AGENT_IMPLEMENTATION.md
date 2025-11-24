# Multi-Agent Feature Implementation Notes

## Completed Tasks (as of 2025-11-24)

### Core Implementation
- [x] **Agent Registry**: Implemented in `codex-rs/core/src/agent.rs` to load agent configurations from `~/.codex/agents.toml`.
  - Added security validation to prevent loading prompts from outside the allowed directory (`validate_prompt_path`).
  - Added symlink protection for the configuration file itself.
  - Implemented nested prompt loading refactor for readability.
- [x] **Agent Handler**: Implemented `AgentHandler` in `codex-rs/core/src/tools/handlers/agent.rs` to execute agent tasks.
  - Uses the `agent` tool to invoke sub-agents.
  - **Tool Inheritance**: Sub-agents inherit tools from the parent session, but the `agent` tool itself is excluded to prevent recursion.
  - **ID Randomization**: Uses `rand::thread_rng().r#gen()` to generate unique call IDs, fixing potential collision issues and deprecated usage.
  - **Response Handling**: Streams agent output and reasoning deltas back to the session.
- [x] **Protocol Updates**: Added `AgentBegin`, `AgentProgress`, `AgentEnd`, and `ListAgentsResponse` events to `codex-rs/protocol/src/protocol.rs`.
  - Updated `codex-rs/core/src/rollout/policy.rs` to persist `AgentBegin` and `AgentEnd` events for history playback.
- [x] **Tools Spec**: Registered the `agent` tool in `codex-rs/core/src/tools/spec.rs`.
  - Enabled parallel execution support for the agent tool.

### TUI Integration
- [x] **Slash Commands**: Added `/agents` command to list available agents (`codex-rs/tui/src/chatwidget.rs`, `slash_command.rs`).
- [x] **Mentions**: Implemented parsing for `@agent: task` syntax in `codex-rs/tui/src/agent_mention.rs`.
  - Regex updated to support hyphens in agent names (e.g., `@code-reviewer`).
  - Mentions are converted into `agent` tool calls before submission.
- [x] **Visual Feedback**: Added history cells for agent execution status (running, progress, done) in `codex-rs/tui/src/history_cell.rs`.

### Documentation
- [x] **Guide**: Created `docs/subagents.md` detailing configuration, usage, and best practices.
- [x] **Examples**: Created `example-agents.toml` with sample agent configurations.
- [x] **Getting Started**: Updated `README.md` and `docs/getting-started.md` with quickstart instructions.
- [x] **AGENTS.md**: Added agent usage rules and ensured trailing newline compliance.

### Testing
- [x] **Unit Tests**: Added tests for agent registry validation and mention parsing.
- [x] **Integration Tests**: Added `sub_agent_inherits_tools` test in `codex-rs/core/tests/suite/agent_tool.rs` to verify tool inheritance logic.
- [x] **Verification**: All tests passed in `codex-core` (459 tests) and `codex-tui` (490 tests).

## Pending / Future Work
- [ ] **Runtime Error Investigation**: The error `{"detail":"Instructions are not valid"}` was observed when running agents against specific endpoints (likely GitHub Models/Azure). This is identified as a configuration mismatch (using `Responses` protocol with an endpoint that rejects `instructions`). Future work could auto-detect this or warn the user.
- [ ] **Test Coverage**: Add more robust integration tests for the TUI interactions if possible (currently manual verification).
- [ ] **Release**: Build and publish the updated `codex-cli` package.

## Branch State
- **Branch**: `agents-multi-final`
- **Latest Commit**: `3d52a35cc` (fix: persist agent events, refactor prompt loading, harden security)
- **Sync Status**: Local workspace is synced with remote.

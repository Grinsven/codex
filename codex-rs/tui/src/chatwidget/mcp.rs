use std::collections::HashMap;
use std::path::PathBuf;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::app_event::McpServerConfigScope;
use crate::bottom_pane::ColumnWidthMode;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell;
use crate::render::renderable::ColumnRenderable;
use codex_app_server_protocol::ConfigLayerSource;
use codex_app_server_protocol::McpAuthStatus as AppServerMcpAuthStatus;
use codex_app_server_protocol::McpServerStatus;
use codex_app_server_protocol::McpServerStatusDetail;
use codex_config::types::McpServerConfig;
use codex_config::types::McpServerTransportConfig;
use codex_protocol::protocol::McpAuthStatus;
use ratatui::style::Stylize;
use ratatui::text::Line;

const MCP_MANAGER_SELECTION_VIEW_ID: &str = "mcp-manager-selection";
const MCP_SERVER_ACTIONS_VIEW_ID: &str = "mcp-server-actions";
const MCP_SERVER_DISABLE_SCOPE_VIEW_ID: &str = "mcp-server-disable-scope";

impl ChatWidget {
    pub(crate) fn add_mcp_output(&mut self, detail: McpServerStatusDetail) {
        if matches!(detail, McpServerStatusDetail::Full) {
            self.add_mcp_inventory_output(detail);
            return;
        }

        if self.managed_mcp_servers().is_empty() {
            self.add_to_history(history_cell::empty_mcp_output());
            return;
        }

        self.active_mcp_action_server = None;
        self.pending_mcp_tools_view = None;
        self.pending_mcp_manager_open = self.mcp_status_snapshot.is_empty();
        if !self.pending_mcp_manager_open {
            self.bottom_pane
                .show_selection_view(self.mcp_manager_popup_params());
            self.request_redraw();
        }
        self.app_event_tx.send(AppEvent::FetchMcpInventory {
            detail: McpServerStatusDetail::ToolsAndAuthOnly,
        });
    }

    fn add_mcp_inventory_output(&mut self, detail: McpServerStatusDetail) {
        self.flush_answer_stream_with_separator();
        self.flush_active_cell();
        self.active_cell = Some(Box::new(history_cell::new_mcp_inventory_loading(
            self.config.animations,
        )));
        self.bump_active_cell_revision();
        self.request_redraw();
        self.app_event_tx
            .send(AppEvent::FetchMcpInventory { detail });
    }

    pub(crate) fn open_mcp_server_actions(&mut self, name: String) {
        if !self.managed_mcp_servers().contains_key(name.as_str()) {
            self.add_error_message(format!("No MCP server named `{name}`."));
            return;
        }

        self.active_mcp_action_server = Some(name.clone());
        self.bottom_pane
            .show_selection_view(self.mcp_server_actions_popup_params(&name));
    }

    pub(crate) fn open_mcp_server_disable_scope_picker(&mut self, name: String) {
        if !self.managed_mcp_servers().contains_key(name.as_str()) {
            self.add_error_message(format!("No MCP server named `{name}`."));
            return;
        }

        self.active_mcp_action_server = Some(name.clone());
        self.bottom_pane
            .show_selection_view(self.mcp_server_disable_scope_popup_params(&name));
    }

    pub(crate) fn view_mcp_server_tools(&mut self, name: String) {
        if let Some(status) = self.mcp_status_snapshot.get(name.as_str()).cloned() {
            self.add_mcp_server_history(status);
            return;
        }

        self.pending_mcp_tools_view = Some(name);
        self.app_event_tx.send(AppEvent::FetchMcpInventory {
            detail: McpServerStatusDetail::ToolsAndAuthOnly,
        });
    }

    pub(crate) fn sync_mcp_config(&mut self, config: &crate::legacy_core::config::Config) {
        self.config.mcp_servers = config.mcp_servers.clone();
        self.config.mcp_oauth_credentials_store_mode = config.mcp_oauth_credentials_store_mode;
    }

    pub(crate) fn refresh_mcp_manager_views(&mut self) {
        let _ = self.bottom_pane.replace_selection_view_if_active(
            MCP_MANAGER_SELECTION_VIEW_ID,
            self.mcp_manager_popup_params(),
        );

        let Some(server_name) = self.active_mcp_action_server.clone() else {
            return;
        };

        if self
            .managed_mcp_servers()
            .contains_key(server_name.as_str())
        {
            let _ = self.bottom_pane.replace_selection_view_if_active(
                MCP_SERVER_ACTIONS_VIEW_ID,
                self.mcp_server_actions_popup_params(&server_name),
            );
        } else {
            self.active_mcp_action_server = None;
        }
    }

    pub(crate) fn on_mcp_inventory_loaded(&mut self, statuses: Vec<McpServerStatus>) {
        self.mcp_status_snapshot = statuses
            .into_iter()
            .map(|status| (status.name.clone(), status))
            .collect();

        if let Some(server_name) = self.pending_mcp_tools_view.take() {
            if let Some(status) = self.mcp_status_snapshot.get(server_name.as_str()).cloned() {
                self.add_mcp_server_history(status);
            } else {
                self.add_error_message(format!(
                    "Failed to load MCP inventory for `{server_name}`."
                ));
            }
            return;
        }

        if self.pending_mcp_manager_open {
            self.pending_mcp_manager_open = false;
            self.bottom_pane
                .show_selection_view(self.mcp_manager_popup_params());
            self.request_redraw();
            return;
        }

        self.refresh_mcp_manager_views();
    }

    pub(crate) fn clear_pending_mcp_inventory_request(&mut self) {
        self.pending_mcp_manager_open = false;
        self.pending_mcp_tools_view = None;
    }

    fn add_mcp_server_history(&mut self, status: McpServerStatus) {
        self.active_mcp_action_server = None;
        self.add_to_history(history_cell::new_mcp_tools_output_from_statuses(
            &self.config,
            &[status],
            McpServerStatusDetail::ToolsAndAuthOnly,
        ));
    }

    fn mcp_manager_popup_params(&self) -> SelectionViewParams {
        let servers = self.managed_mcp_servers();
        let mut names: Vec<String> = servers.keys().cloned().collect();
        names.sort();

        let mut header = ColumnRenderable::new();
        header.push(Line::from("MCP Servers".bold()));
        header.push(Line::from(
            "Manage MCP servers from /mcp. Repo overrides apply to the current cwd.".dim(),
        ));

        let items = names
            .into_iter()
            .filter_map(|name| {
                let config = servers.get(name.as_str())?;
                let search_value = format!("{name} {}", mcp_transport_summary(&config.transport));
                let description = self.manager_row_description(name.as_str(), config);
                Some(SelectionItem {
                    name: name.clone(),
                    description: Some(description),
                    search_value: Some(search_value),
                    selected_description: Some(
                        "Press Enter to manage this MCP server.".to_string(),
                    ),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenMcpServerActions { name: name.clone() });
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                })
            })
            .collect();

        SelectionViewParams {
            view_id: Some(MCP_MANAGER_SELECTION_VIEW_ID),
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: true,
            search_placeholder: Some("Type to search MCP servers".to_string()),
            col_width_mode: ColumnWidthMode::AutoAllRows,
            ..Default::default()
        }
    }

    fn mcp_server_actions_popup_params(&self, server_name: &str) -> SelectionViewParams {
        let managed_servers = self.managed_mcp_servers();
        let Some(config) = managed_servers.get(server_name).cloned() else {
            return self.mcp_manager_popup_params();
        };
        let global_servers = self.global_mcp_servers();
        let global_config = global_servers.get(server_name).cloned();
        let status = self.mcp_status_snapshot.get(server_name);
        let repo_override_active = global_config
            .as_ref()
            .is_some_and(|global| global.enabled != config.enabled);

        let auth_status = status
            .map(|current| app_server_auth_status_display(current.auth_status))
            .unwrap_or_else(|| McpAuthStatus::Unsupported.to_string());
        let tool_count = status.map_or(0, |current| current.tools.len());
        let resource_count = status.map_or(0, |current| current.resources.len());
        let resource_template_count = status.map_or(0, |current| current.resource_templates.len());

        let mut header = ColumnRenderable::new();
        header.push(Line::from(server_name.to_string().bold()));
        header.push(Line::from(mcp_status_line(&config).dim()));
        header.push(Line::from(
            format!("Transport: {}", mcp_transport_summary(&config.transport)).dim(),
        ));
        if let Some(path) = self.global_mcp_config_path() {
            header.push(Line::from(format!("Config: {}", path.display()).dim()));
        }
        if repo_override_active {
            header.push(Line::from("Repo override active for current cwd.".dim()));
        } else if global_config.is_none() {
            header.push(Line::from("Repo-local server for current cwd.".dim()));
        }
        header.push(Line::from(format!("Auth: {auth_status}").dim()));
        header.push(Line::from(
            format!(
                "Tools: {tool_count} · Resources: {resource_count} · Resource templates: {resource_template_count}"
            )
            .dim(),
        ));

        let mut items = vec![SelectionItem {
            name: "View tools".to_string(),
            description: Some("Show this server's tools and resources in the transcript.".into()),
            actions: vec![Box::new({
                let name = server_name.to_string();
                move |tx| tx.send(AppEvent::ViewMcpServerTools { name: name.clone() })
            })],
            dismiss_on_select: true,
            ..Default::default()
        }];

        if config.enabled {
            items.push(SelectionItem {
                name: "Reconnect".to_string(),
                description: Some("Reconnect configured MCP servers immediately.".into()),
                actions: vec![Box::new({
                    let name = server_name.to_string();
                    move |tx| tx.send(AppEvent::ReconnectMcpServer { name: name.clone() })
                })],
                ..Default::default()
            });
        }

        let enable_scope = if repo_override_active
            || global_config
                .as_ref()
                .map_or(true, |global| !global.enabled)
        {
            McpServerConfigScope::Repo
        } else {
            McpServerConfigScope::Global
        };
        let toggle_name = if config.enabled {
            "Disable".to_string()
        } else if enable_scope == McpServerConfigScope::Repo {
            "Enable in this repo".to_string()
        } else {
            "Enable globally".to_string()
        };
        let toggle_description = if config.enabled {
            "Choose whether to disable this MCP server globally or only in this repo."
        } else if enable_scope == McpServerConfigScope::Repo {
            "Remove the repo override and enable this MCP server in the current repo."
        } else {
            "Enable this MCP server everywhere your user config applies."
        };
        items.push(SelectionItem {
            name: toggle_name,
            description: Some(toggle_description.to_string()),
            actions: vec![Box::new({
                let name = server_name.to_string();
                let currently_enabled = config.enabled;
                move |tx| {
                    if currently_enabled {
                        tx.send(AppEvent::OpenMcpServerDisableScopePicker { name: name.clone() })
                    } else {
                        tx.send(AppEvent::SetMcpServerEnabled {
                            name: name.clone(),
                            enabled: true,
                            scope: enable_scope,
                        })
                    }
                }
            })],
            ..Default::default()
        });

        SelectionViewParams {
            view_id: Some(MCP_SERVER_ACTIONS_VIEW_ID),
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new(move |tx| tx.send(AppEvent::OpenMcpManager))),
            col_width_mode: ColumnWidthMode::AutoAllRows,
            ..Default::default()
        }
    }

    fn mcp_server_disable_scope_popup_params(&self, server_name: &str) -> SelectionViewParams {
        let mut header = ColumnRenderable::new();
        header.push(Line::from("Disable MCP server".bold()));
        header.push(Line::from(server_name.to_string().bold()));
        header.push(Line::from(
            "Choose whether to disable this server globally or only in this repo.".dim(),
        ));

        let mut items = Vec::new();
        if self.global_mcp_servers().contains_key(server_name) {
            items.push(SelectionItem {
                name: "Disable globally".to_string(),
                description: Some(
                    "Turn this MCP server off everywhere your user config applies.".to_string(),
                ),
                actions: vec![Box::new({
                    let name = server_name.to_string();
                    move |tx| {
                        tx.send(AppEvent::SetMcpServerEnabled {
                            name: name.clone(),
                            enabled: false,
                            scope: McpServerConfigScope::Global,
                        })
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }
        items.push(SelectionItem {
            name: "Disable in this repo".to_string(),
            description: Some(
                "Only turn this MCP server off for the current repo/worktree.".to_string(),
            ),
            actions: vec![Box::new({
                let name = server_name.to_string();
                move |tx| {
                    tx.send(AppEvent::SetMcpServerEnabled {
                        name: name.clone(),
                        enabled: false,
                        scope: McpServerConfigScope::Repo,
                    })
                }
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        SelectionViewParams {
            view_id: Some(MCP_SERVER_DISABLE_SCOPE_VIEW_ID),
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            on_cancel: Some(Box::new({
                let name = server_name.to_string();
                move |tx| tx.send(AppEvent::OpenMcpServerActions { name: name.clone() })
            })),
            col_width_mode: ColumnWidthMode::AutoAllRows,
            ..Default::default()
        }
    }

    fn manager_row_description(&self, server_name: &str, config: &McpServerConfig) -> String {
        let auth_status = self
            .mcp_status_snapshot
            .get(server_name)
            .map(|current| app_server_auth_status_display(current.auth_status))
            .unwrap_or_else(|| McpAuthStatus::Unsupported.to_string());
        let tool_count = self
            .mcp_status_snapshot
            .get(server_name)
            .map_or(0, |current| current.tools.len());
        let status = mcp_status_line(config);
        format!(
            "{status} · Auth: {auth_status} · Tools: {tool_count} · {}",
            mcp_transport_summary(&config.transport)
        )
    }

    fn managed_mcp_servers(&self) -> HashMap<String, McpServerConfig> {
        let mut servers = self.config.mcp_servers.get().clone();
        for (name, config) in self.global_mcp_servers() {
            servers.entry(name).or_insert(config);
        }
        servers
    }

    pub(crate) fn global_mcp_servers(&self) -> HashMap<String, McpServerConfig> {
        let Some(user_layer) = self.config.config_layer_stack.get_user_layer() else {
            return HashMap::new();
        };
        let Some(servers_value) = user_layer.config.get("mcp_servers") else {
            return HashMap::new();
        };
        servers_value.clone().try_into().unwrap_or_default()
    }

    fn global_mcp_config_path(&self) -> Option<PathBuf> {
        let user_layer = self.config.config_layer_stack.get_user_layer()?;
        match &user_layer.name {
            ConfigLayerSource::User { file } => Some(file.as_path().to_path_buf()),
            _ => None,
        }
    }
}

fn app_server_auth_status_display(status: AppServerMcpAuthStatus) -> String {
    match status {
        AppServerMcpAuthStatus::Unsupported => McpAuthStatus::Unsupported.to_string(),
        AppServerMcpAuthStatus::NotLoggedIn => McpAuthStatus::NotLoggedIn.to_string(),
        AppServerMcpAuthStatus::BearerToken => McpAuthStatus::BearerToken.to_string(),
        AppServerMcpAuthStatus::OAuth => McpAuthStatus::OAuth.to_string(),
    }
}

fn mcp_status_line(config: &McpServerConfig) -> &'static str {
    if config.enabled {
        "Enabled"
    } else {
        "Disabled"
    }
}

fn mcp_transport_summary(transport: &McpServerTransportConfig) -> String {
    match transport {
        McpServerTransportConfig::Stdio { command, args, .. } => {
            if args.is_empty() {
                format!("stdio · {command}")
            } else {
                format!("stdio · {} {}", command, args.join(" "))
            }
        }
        McpServerTransportConfig::StreamableHttp { url, .. } => {
            format!("http · {url}")
        }
    }
}

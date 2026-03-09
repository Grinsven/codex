use std::collections::HashMap;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::bottom_pane::ColumnWidthMode;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::history_cell;
use crate::render::renderable::ColumnRenderable;
use codex_app_server_protocol::ConfigLayerSource;
use codex_core::config::types::McpServerConfig;
use codex_core::config::types::McpServerTransportConfig;
use codex_protocol::protocol::McpAuthStatus;
use codex_protocol::protocol::McpListToolsResponseEvent;
use codex_protocol::protocol::Op;
use ratatui::style::Stylize;
use ratatui::text::Line;

const MCP_MANAGER_SELECTION_VIEW_ID: &str = "mcp-manager-selection";
const MCP_SERVER_ACTIONS_VIEW_ID: &str = "mcp-server-actions";

impl ChatWidget {
    pub(crate) fn add_mcp_output(&mut self) {
        if self.global_mcp_servers().is_empty() {
            self.add_to_history(history_cell::empty_mcp_output());
            return;
        }

        self.active_mcp_action_server = None;
        self.bottom_pane
            .show_selection_view(self.mcp_manager_popup_params());
        self.submit_op(Op::ListMcpTools);
    }

    pub(crate) fn open_mcp_server_actions(&mut self, name: String) {
        if !self.global_mcp_servers().contains_key(name.as_str()) {
            self.add_error_message(format!("No global MCP server named `{name}`."));
            return;
        }

        self.active_mcp_action_server = Some(name.clone());
        self.bottom_pane
            .show_selection_view(self.mcp_server_actions_popup_params(&name));
    }

    pub(crate) fn view_mcp_server_tools(&mut self, name: String) {
        if let Some(snapshot) = self.mcp_snapshot.clone() {
            self.add_mcp_server_history(name, snapshot);
            return;
        }

        self.pending_mcp_tools_view = Some(name);
        self.submit_op(Op::ListMcpTools);
    }

    pub(crate) fn sync_mcp_config(&mut self, config: &codex_core::config::Config) {
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

        if self.global_mcp_servers().contains_key(server_name.as_str()) {
            let _ = self.bottom_pane.replace_selection_view_if_active(
                MCP_SERVER_ACTIONS_VIEW_ID,
                self.mcp_server_actions_popup_params(&server_name),
            );
        } else {
            self.active_mcp_action_server = None;
        }
    }

    pub(crate) fn on_list_mcp_tools(&mut self, ev: McpListToolsResponseEvent) {
        self.mcp_snapshot = Some(ev.clone());

        if let Some(server_name) = self.pending_mcp_tools_view.take() {
            self.add_mcp_server_history(server_name, ev);
            return;
        }

        self.refresh_mcp_manager_views();
    }

    fn add_mcp_server_history(&mut self, server_name: String, snapshot: McpListToolsResponseEvent) {
        self.active_mcp_action_server = None;
        self.add_to_history(history_cell::new_filtered_mcp_tools_output(
            &self.config,
            snapshot.tools,
            snapshot.resources,
            snapshot.resource_templates,
            &snapshot.auth_statuses,
            server_name.as_str(),
        ));
    }

    fn mcp_manager_popup_params(&self) -> SelectionViewParams {
        let servers = self.global_mcp_servers();
        let mut names: Vec<String> = servers.keys().cloned().collect();
        names.sort();

        let mut header = ColumnRenderable::new();
        header.push(Line::from("MCP Servers".bold()));
        header.push(Line::from(
            "Manage globally configured MCP servers from /mcp.".dim(),
        ));

        let snapshot = self.mcp_snapshot.as_ref();
        let items = names
            .into_iter()
            .filter_map(|name| {
                let config = servers.get(name.as_str())?;
                let search_value = format!("{name} {}", mcp_transport_summary(&config.transport));
                let description = self.manager_row_description(name.as_str(), config, snapshot);
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
        let servers = self.global_mcp_servers();
        let Some(config) = servers.get(server_name) else {
            return self.mcp_manager_popup_params();
        };

        let auth_status = self
            .mcp_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.auth_statuses.get(server_name))
            .copied()
            .unwrap_or(McpAuthStatus::Unsupported);
        let tool_count = self
            .mcp_snapshot
            .as_ref()
            .map_or(0, |snapshot| server_tool_count(snapshot, server_name));
        let resource_count = self.mcp_snapshot.as_ref().map_or(0, |snapshot| {
            snapshot.resources.get(server_name).map_or(0, Vec::len)
        });
        let resource_template_count = self.mcp_snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .resource_templates
                .get(server_name)
                .map_or(0, Vec::len)
        });

        let mut header = ColumnRenderable::new();
        header.push(Line::from(server_name.to_string().bold()));
        header.push(Line::from(mcp_status_line(config).dim()));
        header.push(Line::from(
            format!("Transport: {}", mcp_transport_summary(&config.transport)).dim(),
        ));
        if let Some(path) = self.global_mcp_config_path() {
            header.push(Line::from(format!("Config: {}", path.display()).dim()));
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

        let toggle_name = if config.enabled { "Disable" } else { "Enable" };
        let toggle_description = if config.enabled {
            "Disable this MCP server."
        } else {
            "Enable this MCP server."
        };
        items.push(SelectionItem {
            name: toggle_name.to_string(),
            description: Some(toggle_description.to_string()),
            actions: vec![Box::new({
                let name = server_name.to_string();
                let enabled = !config.enabled;
                move |tx| {
                    tx.send(AppEvent::SetMcpServerEnabled {
                        name: name.clone(),
                        enabled,
                    })
                }
            })],
            ..Default::default()
        });

        SelectionViewParams {
            view_id: Some(MCP_SERVER_ACTIONS_VIEW_ID),
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            col_width_mode: ColumnWidthMode::AutoAllRows,
            ..Default::default()
        }
    }

    fn manager_row_description(
        &self,
        server_name: &str,
        config: &McpServerConfig,
        snapshot: Option<&McpListToolsResponseEvent>,
    ) -> String {
        let auth_status = snapshot
            .and_then(|current| current.auth_statuses.get(server_name))
            .copied()
            .unwrap_or(McpAuthStatus::Unsupported);
        let tool_count = snapshot.map_or(0, |current| server_tool_count(current, server_name));
        let status = mcp_status_line(config);
        format!(
            "{status} · Auth: {auth_status} · Tools: {tool_count} · {}",
            mcp_transport_summary(&config.transport)
        )
    }

    fn global_mcp_servers(&self) -> HashMap<String, McpServerConfig> {
        let Some(user_layer) = self.config.config_layer_stack.get_user_layer() else {
            return HashMap::new();
        };
        let Some(servers_value) = user_layer.config.get("mcp_servers") else {
            return HashMap::new();
        };
        servers_value.clone().try_into().unwrap_or_default()
    }

    fn global_mcp_config_path(&self) -> Option<std::path::PathBuf> {
        let user_layer = self.config.config_layer_stack.get_user_layer()?;
        match &user_layer.name {
            ConfigLayerSource::User { file } => Some(file.as_path().to_path_buf()),
            _ => None,
        }
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
                command.clone()
            } else {
                format!("{command} {}", args.join(" "))
            }
        }
        McpServerTransportConfig::StreamableHttp { url, .. } => url.clone(),
    }
}

fn server_tool_count(snapshot: &McpListToolsResponseEvent, server_name: &str) -> usize {
    let prefix = format!("mcp__{server_name}__");
    snapshot
        .tools
        .keys()
        .filter(|name| name.starts_with(prefix.as_str()))
        .count()
}

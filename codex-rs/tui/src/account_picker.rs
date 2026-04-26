use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Local;
use codex_login::SavedChatgptAccount;
use crossterm::event::KeyCode;
use ratatui::text::Line;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::SelectionAction;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionShortcutAction;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::key_hint;
use crate::status::RATE_LIMIT_STALE_THRESHOLD_MINUTES;
use crate::status::RateLimitSnapshotDisplay;
use crate::status::RateLimitWindowDisplay;

pub(crate) const ACCOUNT_SELECTION_VIEW_ID: &str = "saved-chatgpt-account-selection";

pub(crate) fn account_selection_params(
    accounts: Vec<SavedChatgptAccountPickerEntry>,
    app_event_tx: AppEventSender,
) -> SelectionViewParams {
    let items = if accounts.is_empty() {
        vec![SelectionItem {
            name: "No saved ChatGPT accounts".to_string(),
            description: Some("Sign in with another ChatGPT account to add it here.".to_string()),
            is_disabled: true,
            ..Default::default()
        }]
    } else {
        accounts
            .into_iter()
            .map(|account| selection_item_for_account(account, app_event_tx.clone()))
            .collect()
    };

    SelectionViewParams {
        view_id: Some(ACCOUNT_SELECTION_VIEW_ID),
        title: Some("Switch ChatGPT account".to_string()),
        subtitle: Some("Choose a saved ChatGPT account for this session.".to_string()),
        footer_hint: Some(account_picker_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search accounts".to_string()),
        ..Default::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SavedChatgptAccountPickerEntry {
    pub(crate) account: SavedChatgptAccount,
    pub(crate) five_hour_limit_remaining_percent: Option<u8>,
    pub(crate) seven_day_limit_remaining_percent: Option<u8>,
    pub(crate) limits_loading: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SavedChatgptAccountRateLimitCacheEntry {
    pub(crate) five_hour_limit_remaining_percent: Option<u8>,
    pub(crate) seven_day_limit_remaining_percent: Option<u8>,
    pub(crate) captured_at: DateTime<Local>,
}

impl SavedChatgptAccountRateLimitCacheEntry {
    pub(crate) fn is_stale(&self, now: DateTime<Local>) -> bool {
        now.signed_duration_since(self.captured_at)
            > ChronoDuration::minutes(RATE_LIMIT_STALE_THRESHOLD_MINUTES)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SavedChatgptAccountRateLimitCacheUpdate {
    pub(crate) account_id: String,
    pub(crate) limits: Option<SavedChatgptAccountRateLimitCacheEntry>,
}

pub(crate) fn picker_entry_for_saved_account(
    account: SavedChatgptAccount,
    rate_limits: Option<&SavedChatgptAccountRateLimitCacheEntry>,
    limits_loading: bool,
) -> SavedChatgptAccountPickerEntry {
    SavedChatgptAccountPickerEntry {
        account,
        five_hour_limit_remaining_percent: rate_limits
            .and_then(|limits| limits.five_hour_limit_remaining_percent),
        seven_day_limit_remaining_percent: rate_limits
            .and_then(|limits| limits.seven_day_limit_remaining_percent),
        limits_loading: limits_loading && rate_limits.is_none(),
    }
}

pub(crate) fn rate_limit_cache_entry_from_snapshot(
    snapshot: &RateLimitSnapshotDisplay,
) -> SavedChatgptAccountRateLimitCacheEntry {
    SavedChatgptAccountRateLimitCacheEntry {
        five_hour_limit_remaining_percent: remaining_percent(snapshot.primary.as_ref()),
        seven_day_limit_remaining_percent: remaining_percent(snapshot.secondary.as_ref()),
        captured_at: snapshot.captured_at,
    }
}

fn selection_item_for_account(
    entry: SavedChatgptAccountPickerEntry,
    app_event_tx: AppEventSender,
) -> SelectionItem {
    let SavedChatgptAccountPickerEntry {
        account,
        five_hour_limit_remaining_percent,
        seven_day_limit_remaining_percent,
        limits_loading,
    } = entry;
    let label = account_label(&account);
    let search_value = account_search_value(&account);
    let description = account_row_description(
        &account,
        five_hour_limit_remaining_percent,
        seven_day_limit_remaining_percent,
        limits_loading,
    );
    let selected_description = Some(account_selected_description(
        &account,
        &label,
        five_hour_limit_remaining_percent,
        seven_day_limit_remaining_percent,
        limits_loading,
    ));
    let is_current = account.is_active;
    let account_id = account.id;
    let action_label = label.clone();
    let delete_account_id = account_id.clone();
    let delete_label = label.clone();

    let actions: Vec<SelectionAction> = if is_current {
        Vec::new()
    } else {
        vec![Box::new(move |_tx: &AppEventSender| {
            app_event_tx.send(AppEvent::SwitchSavedChatgptAccount {
                account_id: account_id.clone(),
                label: action_label.clone(),
            });
        })]
    };
    let shortcut_actions = vec![SelectionShortcutAction {
        binding: key_hint::plain(KeyCode::Char('-')),
        action: Box::new(move |tx: &AppEventSender| {
            tx.send(AppEvent::DeleteSavedChatgptAccount {
                account_id: delete_account_id.clone(),
                label: delete_label.clone(),
            });
        }),
    }];

    SelectionItem {
        name: label,
        description,
        selected_description,
        is_current,
        actions,
        shortcut_actions,
        dismiss_on_select: true,
        search_value: Some(search_value),
        ..Default::default()
    }
}

fn account_picker_hint_line() -> Line<'static> {
    let mut spans = standard_popup_hint_line().spans;
    spans.extend([
        " · ".into(),
        "Press ".into(),
        key_hint::plain(KeyCode::Char('-')).into(),
        " to delete saved account".into(),
    ]);
    Line::from(spans)
}

pub(crate) fn account_label(account: &SavedChatgptAccount) -> String {
    account
        .email
        .clone()
        .or_else(|| account.account_id.clone())
        .or_else(|| account.chatgpt_user_id.clone())
        .unwrap_or_else(|| truncate_id(&account.id))
}

fn account_row_description(
    account: &SavedChatgptAccount,
    five_hour_limit_remaining_percent: Option<u8>,
    seven_day_limit_remaining_percent: Option<u8>,
    limits_loading: bool,
) -> Option<String> {
    let mut parts = compact_limit_summary_parts(
        five_hour_limit_remaining_percent,
        seven_day_limit_remaining_percent,
    );
    if parts.is_empty() && limits_loading {
        parts.push("Loading 5h/7d...".to_string());
    }
    if let Some(plan_type) = &account.plan_type {
        parts.push(plan_type.clone());
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn account_selected_description(
    account: &SavedChatgptAccount,
    label: &str,
    five_hour_limit_remaining_percent: Option<u8>,
    seven_day_limit_remaining_percent: Option<u8>,
    limits_loading: bool,
) -> String {
    if account.is_active {
        let mut parts = compact_limit_summary_parts(
            five_hour_limit_remaining_percent,
            seven_day_limit_remaining_percent,
        );
        if parts.is_empty() && limits_loading {
            parts.push("Loading 5h/7d...".to_string());
        }
        if parts.is_empty() {
            "Current account".to_string()
        } else {
            format!("Current account · {}", parts.join(" · "))
        }
    } else {
        let mut parts = compact_limit_summary_parts(
            five_hour_limit_remaining_percent,
            seven_day_limit_remaining_percent,
        );
        if parts.is_empty() && limits_loading {
            parts.push("Loading 5h/7d...".to_string());
        }
        parts.extend(account_detail_parts(account));
        if parts.is_empty() {
            format!("Press Enter to switch to {label}.")
        } else {
            format!("Press Enter to switch to {label}. {}", parts.join(" · "))
        }
    }
}

fn compact_limit_summary_parts(
    five_hour_limit_remaining_percent: Option<u8>,
    seven_day_limit_remaining_percent: Option<u8>,
) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(percent) = five_hour_limit_remaining_percent {
        parts.push(format!("5h {percent}%"));
    }
    if let Some(percent) = seven_day_limit_remaining_percent {
        parts.push(format!("7d {percent}%"));
    }
    parts
}

fn account_detail_parts(account: &SavedChatgptAccount) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(plan_type) = &account.plan_type {
        parts.push(plan_type.clone());
    }
    if let Some(workspace_id) = &account.chatgpt_workspace_id {
        parts.push(format!("WS {}", truncate_id(workspace_id)));
    }
    if let Some(account_id) = &account.account_id {
        parts.push(format!("Acct {}", truncate_id(account_id)));
    }
    parts
}

fn account_search_value(account: &SavedChatgptAccount) -> String {
    [
        Some(account.id.as_str()),
        account.email.as_deref(),
        account.account_id.as_deref(),
        account.chatgpt_user_id.as_deref(),
        account.chatgpt_workspace_id.as_deref(),
        account.plan_type.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

fn truncate_id(value: &str) -> String {
    const MAX_ID_LEN: usize = 12;

    if value.chars().count() <= MAX_ID_LEN {
        value.to_string()
    } else {
        let prefix = value.chars().take(MAX_ID_LEN).collect::<String>();
        format!("{prefix}...")
    }
}

fn remaining_percent(window: Option<&RateLimitWindowDisplay>) -> Option<u8> {
    let window = window?;
    let remaining = (100.0f64 - window.used_percent).clamp(0.0f64, 100.0f64);
    Some(remaining.round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use tokio::sync::mpsc::unbounded_channel;

    fn app_event_sender() -> AppEventSender {
        let (tx, _rx) = unbounded_channel();
        AppEventSender::new(tx)
    }

    fn saved_account(
        id: &str,
        email: Option<&str>,
        plan_type: Option<&str>,
        is_active: bool,
    ) -> SavedChatgptAccount {
        SavedChatgptAccount {
            id: id.to_string(),
            email: email.map(str::to_string),
            account_id: Some(format!("{id}-account")),
            chatgpt_user_id: Some(format!("{id}-user")),
            chatgpt_workspace_id: Some(format!("{id}-workspace")),
            plan_type: plan_type.map(str::to_string),
            is_active,
        }
    }

    fn snapshot_params(params: &SelectionViewParams) -> String {
        let mut lines = vec![
            format!("title={:?}", params.title),
            format!("subtitle={:?}", params.subtitle),
            format!("searchable={}", params.is_searchable),
            format!("placeholder={:?}", params.search_placeholder),
        ];
        for item in &params.items {
            lines.push(format!(
                "item name={:?} desc={:?} selected={:?} current={} disabled={} actions={} shortcut_actions={} dismiss={} search={:?}",
                item.name,
                item.description,
                item.selected_description,
                item.is_current,
                item.is_disabled,
                item.actions.len(),
                item.shortcut_actions.len(),
                item.dismiss_on_select,
                item.search_value
            ));
        }
        lines.join("\n")
    }

    #[test]
    fn account_selection_params_render_current_and_switchable_accounts() {
        let params = account_selection_params(
            vec![
                SavedChatgptAccountPickerEntry {
                    account: saved_account(
                        "active",
                        Some("active@example.com"),
                        Some("Plus"),
                        true,
                    ),
                    five_hour_limit_remaining_percent: Some(7),
                    seven_day_limit_remaining_percent: Some(80),
                    limits_loading: false,
                },
                SavedChatgptAccountPickerEntry {
                    account: saved_account("next", Some("next@example.com"), Some("Pro"), false),
                    five_hour_limit_remaining_percent: None,
                    seven_day_limit_remaining_percent: None,
                    limits_loading: true,
                },
            ],
            app_event_sender(),
        );

        assert_snapshot!(
            snapshot_params(&params),
            @r###"
title=Some("Switch ChatGPT account")
subtitle=Some("Choose a saved ChatGPT account for this session.")
searchable=true
placeholder=Some("Type to search accounts")
item name="active@example.com" desc=Some("5h 7% · 7d 80% · Plus") selected=Some("Current account · 5h 7% · 7d 80%") current=true disabled=false actions=0 shortcut_actions=1 dismiss=true search=Some("active active@example.com active-account active-user active-workspace Plus")
item name="next@example.com" desc=Some("Loading 5h/7d... · Pro") selected=Some("Press Enter to switch to next@example.com. Loading 5h/7d... · Pro · WS next-worksp... · Acct next-account") current=false disabled=false actions=1 shortcut_actions=1 dismiss=true search=Some("next next@example.com next-account next-user next-workspace Pro")
"###
        );
    }

    #[test]
    fn account_selection_params_render_empty_state() {
        let params = account_selection_params(Vec::new(), app_event_sender());

        assert_snapshot!(
            snapshot_params(&params),
            @r###"
title=Some("Switch ChatGPT account")
subtitle=Some("Choose a saved ChatGPT account for this session.")
searchable=true
placeholder=Some("Type to search accounts")
item name="No saved ChatGPT accounts" desc=Some("Sign in with another ChatGPT account to add it here.") selected=None current=false disabled=true actions=0 shortcut_actions=0 dismiss=false search=None
"###
        );
    }
}

use super::*;
use crate::account_picker::ACCOUNT_SELECTION_VIEW_ID;
use crate::account_picker::SavedChatgptAccountPickerEntry;
use crate::account_picker::SavedChatgptAccountRateLimitCacheEntry;
use crate::account_picker::SavedChatgptAccountRateLimitCacheUpdate;
use crate::account_picker::account_label;
use crate::account_picker::picker_entry_for_saved_account;
use crate::account_picker::rate_limit_cache_entry_from_snapshot;
use codex_backend_client::Client as BackendClient;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::SavedChatgptAccount;
use codex_login::SavedChatgptAccountAuth;
use tokio::task::JoinSet;
use tracing::debug;

const ACCOUNT_AUTO_SWITCH_REMAINING_THRESHOLD_PERCENT: u8 = 2;
const ACCOUNT_AUTO_SWITCH_MIN_SEVEN_DAY_REMAINING_PERCENT: u8 = 1;

impl ChatWidget {
    pub(crate) fn open_saved_chatgpt_account_picker(&mut self) {
        match self
            .saved_chatgpt_auth_manager()
            .list_saved_chatgpt_account_auths()
        {
            Ok(account_auths) if account_auths.is_empty() => {
                self.add_info_message(
                    "No saved ChatGPT accounts found. Run `codex login` to add one.".to_string(),
                    /*hint*/ None,
                );
            }
            Ok(account_auths) => {
                let accounts = account_auths
                    .iter()
                    .map(|saved| saved.account.clone())
                    .collect::<Vec<_>>();
                self.seed_saved_chatgpt_account_limit_cache_from_active_snapshot(
                    accounts.as_slice(),
                );
                let accounts_to_refresh =
                    self.saved_chatgpt_accounts_needing_limit_refresh(account_auths.as_slice());
                self.saved_chatgpt_account_rate_limit_pending_ids.extend(
                    accounts_to_refresh
                        .iter()
                        .map(|saved| saved.account.id.clone()),
                );
                let entries = self.saved_chatgpt_account_picker_entries(accounts.as_slice());
                if !self.bottom_pane.replace_selection_view_if_active(
                    ACCOUNT_SELECTION_VIEW_ID,
                    crate::account_picker::account_selection_params(
                        entries.clone(),
                        self.app_event_tx.clone(),
                    ),
                ) {
                    self.bottom_pane.show_selection_view(
                        crate::account_picker::account_selection_params(
                            entries,
                            self.app_event_tx.clone(),
                        ),
                    );
                }
                if !accounts_to_refresh.is_empty() {
                    let app_event_tx = self.app_event_tx.clone();
                    let base_url = self.config.chatgpt_base_url.clone();
                    tokio::spawn(async move {
                        let updates = load_saved_chatgpt_account_rate_limit_updates(
                            base_url,
                            accounts_to_refresh,
                        )
                        .await;
                        app_event_tx
                            .send(AppEvent::SavedChatgptAccountRateLimitsLoaded { updates });
                    });
                }
                self.request_redraw();
            }
            Err(err) => {
                self.add_error_message(format!("Failed to load saved ChatGPT accounts: {err}"));
            }
        }
    }

    pub(crate) fn on_saved_chatgpt_account_rate_limits_loaded(
        &mut self,
        updates: Vec<SavedChatgptAccountRateLimitCacheUpdate>,
    ) {
        for update in updates {
            self.saved_chatgpt_account_rate_limit_pending_ids
                .remove(&update.account_id);
            if let Some(limits) = update.limits {
                self.saved_chatgpt_account_rate_limit_cache
                    .insert(update.account_id, limits);
            }
        }

        if let Ok(accounts) = self
            .saved_chatgpt_auth_manager()
            .list_saved_chatgpt_accounts()
        {
            let params = crate::account_picker::account_selection_params(
                self.saved_chatgpt_account_picker_entries(accounts.as_slice()),
                self.app_event_tx.clone(),
            );
            if self
                .bottom_pane
                .replace_selection_view_if_active(ACCOUNT_SELECTION_VIEW_ID, params)
            {
                self.request_redraw();
            }
        }

        self.maybe_auto_switch_saved_chatgpt_account();
    }

    pub(crate) fn refresh_auth_state_after_account_switch(&mut self, label: String) {
        self.refresh_auth_state_after_account_switch_with_message(format!(
            "Switched ChatGPT account to {label}."
        ));
    }

    pub(crate) fn update_active_saved_chatgpt_account_limit_cache(
        &mut self,
        snapshot: &RateLimitSnapshotDisplay,
    ) {
        let Ok(accounts) = self
            .saved_chatgpt_auth_manager()
            .list_saved_chatgpt_accounts()
        else {
            return;
        };
        let Some(active_account) = accounts.into_iter().find(|account| account.is_active) else {
            return;
        };
        self.saved_chatgpt_account_rate_limit_cache.insert(
            active_account.id,
            rate_limit_cache_entry_from_snapshot(snapshot),
        );
    }

    fn saved_chatgpt_auth_manager(&self) -> Arc<AuthManager> {
        AuthManager::shared_unloaded(
            self.config.codex_home.to_path_buf(),
            self.config.cli_auth_credentials_store_mode,
            Some(self.config.chatgpt_base_url.clone()),
        )
    }

    fn saved_chatgpt_account_picker_entries(
        &self,
        accounts: &[SavedChatgptAccount],
    ) -> Vec<SavedChatgptAccountPickerEntry> {
        accounts
            .iter()
            .cloned()
            .map(|account| {
                let cached_limits = self.saved_chatgpt_account_rate_limit_cache.get(&account.id);
                let limits_loading = cached_limits.is_none()
                    && self
                        .saved_chatgpt_account_rate_limit_pending_ids
                        .contains(&account.id);
                picker_entry_for_saved_account(account, cached_limits, limits_loading)
            })
            .collect()
    }

    fn saved_chatgpt_accounts_needing_limit_refresh(
        &self,
        account_auths: &[SavedChatgptAccountAuth],
    ) -> Vec<SavedChatgptAccountAuth> {
        let now = Local::now();
        account_auths
            .iter()
            .filter(|saved| {
                if self
                    .saved_chatgpt_account_rate_limit_pending_ids
                    .contains(&saved.account.id)
                {
                    return false;
                }
                self.saved_chatgpt_account_rate_limit_cache
                    .get(&saved.account.id)
                    .is_none_or(|limits| limits.is_stale(now))
            })
            .cloned()
            .collect()
    }

    pub(crate) fn maybe_auto_switch_saved_chatgpt_account(&mut self) {
        let Ok(account_auths) = self
            .saved_chatgpt_auth_manager()
            .list_saved_chatgpt_account_auths()
        else {
            return;
        };
        let accounts = account_auths
            .iter()
            .map(|saved| saved.account.clone())
            .collect::<Vec<_>>();
        let Some(active_account) = accounts.iter().find(|account| account.is_active) else {
            return;
        };
        if self.plan_type == Some(PlanType::Pro)
            || active_account
                .plan_type
                .as_deref()
                .is_some_and(|plan_type| plan_type.eq_ignore_ascii_case("pro"))
        {
            self.saved_chatgpt_account_zero_limit_notice_account_id = None;
            return;
        }
        let Some(active_limits) = self
            .saved_chatgpt_account_rate_limit_cache
            .get(&active_account.id)
        else {
            return;
        };
        let five_hour_limit_low = active_limits
            .five_hour_limit_remaining_percent
            .is_some_and(|remaining| remaining <= ACCOUNT_AUTO_SWITCH_REMAINING_THRESHOLD_PERCENT);
        let seven_day_limit_low = active_limits
            .seven_day_limit_remaining_percent
            .is_some_and(|remaining| remaining <= ACCOUNT_AUTO_SWITCH_REMAINING_THRESHOLD_PERCENT);
        if !five_hour_limit_low && !seven_day_limit_low {
            self.saved_chatgpt_account_zero_limit_notice_account_id = None;
            return;
        }

        let accounts_to_refresh = self
            .saved_chatgpt_accounts_needing_limit_refresh(account_auths.as_slice())
            .into_iter()
            .filter(|saved| saved.account.id != active_account.id)
            .collect::<Vec<_>>();
        if !accounts_to_refresh.is_empty() {
            self.saved_chatgpt_account_rate_limit_pending_ids.extend(
                accounts_to_refresh
                    .iter()
                    .map(|saved| saved.account.id.clone()),
            );
            let app_event_tx = self.app_event_tx.clone();
            let base_url = self.config.chatgpt_base_url.clone();
            tokio::spawn(async move {
                let updates =
                    load_saved_chatgpt_account_rate_limit_updates(base_url, accounts_to_refresh)
                        .await;
                app_event_tx.send(AppEvent::SavedChatgptAccountRateLimitsLoaded { updates });
            });
            return;
        }

        let best_account = accounts
            .iter()
            .filter(|account| !account.is_active)
            .filter_map(|account| {
                self.saved_chatgpt_account_rate_limit_cache
                    .get(&account.id)
                    .and_then(|limits| {
                        let five_hour_remaining_percent =
                            limits.five_hour_limit_remaining_percent?;
                        let seven_day_remaining_percent =
                            limits.seven_day_limit_remaining_percent?;
                        if five_hour_remaining_percent > 0
                            && seven_day_remaining_percent
                                >= ACCOUNT_AUTO_SWITCH_MIN_SEVEN_DAY_REMAINING_PERCENT
                        {
                            Some((five_hour_remaining_percent, account))
                        } else {
                            None
                        }
                    })
            })
            .max_by(
                |(left_percent, left_account), (right_percent, right_account)| {
                    left_percent
                        .cmp(right_percent)
                        .then_with(|| right_account.id.cmp(&left_account.id))
                },
            );

        if let Some((remaining_percent, account)) = best_account {
            let label = account_label(account);
            let reason = match (five_hour_limit_low, seven_day_limit_low) {
                (true, true) => "the 5h and 7d limits",
                (true, false) => "the 5h limit",
                (false, true) => "the 7d limit",
                (false, false) => unreachable!("low-limit auto-switch should have returned early"),
            };
            self.app_event_tx.send(AppEvent::SwitchSavedChatgptAccount {
                account_id: account.id.clone(),
                label,
            });
            self.add_info_message(
                format!(
                    "Auto-switching ChatGPT account because the current account fell below {ACCOUNT_AUTO_SWITCH_REMAINING_THRESHOLD_PERCENT}% on {reason}. Best available account has {remaining_percent}% left on the 5h limit."
                ),
                /*hint*/ None,
            );
            return;
        }

        if self
            .saved_chatgpt_account_zero_limit_notice_account_id
            .as_deref()
            == Some(active_account.id.as_str())
        {
            return;
        }

        self.saved_chatgpt_account_zero_limit_notice_account_id = Some(active_account.id.clone());
        self.add_info_message(
            format!(
                "No saved ChatGPT accounts have both >0% left on the 5h limit and at least {ACCOUNT_AUTO_SWITCH_MIN_SEVEN_DAY_REMAINING_PERCENT}% left on the 7d limit."
            ),
            /*hint*/ None,
        );
    }

    fn seed_saved_chatgpt_account_limit_cache_from_active_snapshot(
        &mut self,
        accounts: &[SavedChatgptAccount],
    ) {
        let Some(snapshot) = self.rate_limit_snapshots_by_limit_id.get("codex") else {
            return;
        };
        let Some(active_account) = accounts.iter().find(|account| account.is_active) else {
            return;
        };
        self.saved_chatgpt_account_rate_limit_cache.insert(
            active_account.id.clone(),
            rate_limit_cache_entry_from_snapshot(snapshot),
        );
    }

    fn refresh_auth_state_after_account_switch_with_message(&mut self, message: String) {
        self.plan_type = self
            .saved_chatgpt_auth_manager()
            .auth_cached()
            .and_then(|auth| auth.account_plan_type());
        self.rate_limit_snapshots_by_limit_id.clear();
        self.saved_chatgpt_account_rate_limit_pending_ids.clear();
        self.saved_chatgpt_account_zero_limit_notice_account_id = None;
        self.rate_limit_warnings = RateLimitWarningState::default();
        self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Idle;
        self.stop_rate_limit_poller();
        self.prefetch_rate_limits();

        self.connectors_cache = ConnectorsCacheState::default();
        self.connectors_partial_snapshot = None;
        self.connectors_prefetch_in_flight = false;
        self.connectors_force_refetch_pending = false;
        self.bottom_pane.set_connectors_snapshot(/*snapshot*/ None);
        self.bottom_pane
            .set_connectors_enabled(self.connectors_enabled());
        self.refresh_plugin_mentions();
        if self.connectors_enabled() {
            self.refresh_connectors(/*force_refetch*/ true);
        }

        self.add_info_message(message, /*hint*/ None);
        self.refresh_status_surfaces();
        self.request_redraw();
    }
}

async fn load_saved_chatgpt_account_rate_limit_updates(
    base_url: String,
    saved_accounts: Vec<SavedChatgptAccountAuth>,
) -> Vec<SavedChatgptAccountRateLimitCacheUpdate> {
    let mut join_set = JoinSet::new();
    let update_count = saved_accounts.len();
    for saved in saved_accounts {
        let base_url = base_url.clone();
        join_set.spawn(async move {
            let limits =
                fetch_saved_chatgpt_account_rate_limit_cache_entry(base_url, &saved.auth).await;
            SavedChatgptAccountRateLimitCacheUpdate {
                account_id: saved.account.id,
                limits,
            }
        });
    }

    let mut updates = Vec::with_capacity(update_count);
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(update) => updates.push(update),
            Err(err) => debug!(error = ?err, "failed to join saved account limits refresh task"),
        }
    }
    updates.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    updates
}

async fn fetch_saved_chatgpt_account_rate_limit_cache_entry(
    base_url: String,
    auth: &CodexAuth,
) -> Option<SavedChatgptAccountRateLimitCacheEntry> {
    let client: BackendClient = match BackendClient::from_auth(base_url, auth) {
        Ok(client) => client,
        Err(err) => {
            debug!(error = ?err, "failed to construct backend client for saved account limits");
            return None;
        }
    };
    let snapshots: Vec<RateLimitSnapshot> = match client.get_rate_limits_many().await {
        Ok(snapshots) => snapshots,
        Err(err) => {
            debug!(error = ?err, "failed to fetch saved account limits");
            return None;
        }
    };
    let snapshot = snapshots.into_iter().find(|snapshot| {
        snapshot
            .limit_id
            .as_deref()
            .is_none_or(|limit_id: &str| limit_id.eq_ignore_ascii_case("codex"))
    })?;
    let limit_name = snapshot
        .limit_name
        .clone()
        .unwrap_or_else(|| "codex".to_string());
    let display = rate_limit_snapshot_display_for_limit(&snapshot, limit_name, Local::now());
    Some(rate_limit_cache_entry_from_snapshot(&display))
}

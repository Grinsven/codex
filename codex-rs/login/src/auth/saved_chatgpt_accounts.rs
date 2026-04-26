use sha2::Digest;
use sha2::Sha256;
use std::path::Path;

use codex_app_server_protocol::AuthMode as ApiAuthMode;
use codex_config::types::AuthCredentialsStoreMode;

use crate::auth::CodexAuth;
use crate::auth::storage::AuthDotJson;
use crate::auth::storage::SavedChatgptAccounts;
use crate::auth::storage::create_auth_storage;
use crate::auth::storage::create_saved_chatgpt_accounts_storage;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedChatgptAccount {
    pub id: String,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
    pub chatgpt_workspace_id: Option<String>,
    pub plan_type: Option<String>,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct SavedChatgptAccountAuth {
    pub account: SavedChatgptAccount,
    pub auth: CodexAuth,
}

struct SavedChatgptAccountEntry {
    account: SavedChatgptAccount,
    auth_dot_json: AuthDotJson,
}

pub(crate) fn upsert_saved_chatgpt_account(
    codex_home: &Path,
    auth: &AuthDotJson,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    if !is_managed_chatgpt_auth(auth) {
        return Ok(());
    }

    let storage = create_saved_chatgpt_accounts_storage(
        codex_home.to_path_buf(),
        auth_credentials_store_mode,
    );
    let mut saved_accounts = storage.load()?.unwrap_or_default();
    let changed = upsert_saved_chatgpt_account_in_registry(&mut saved_accounts.accounts, auth);
    if changed {
        storage.save(&saved_accounts)?;
    }
    Ok(())
}

pub(crate) fn list_saved_chatgpt_accounts(
    codex_home: &Path,
    runtime_active_auth: Option<&CodexAuth>,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<Vec<SavedChatgptAccount>> {
    Ok(list_saved_chatgpt_account_entries(
        codex_home,
        runtime_active_auth,
        auth_credentials_store_mode,
    )?
    .into_iter()
    .map(|entry| entry.account)
    .collect())
}

pub(crate) fn list_saved_chatgpt_account_auths(
    codex_home: &Path,
    runtime_active_auth: Option<&CodexAuth>,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<Vec<SavedChatgptAccountAuth>> {
    list_saved_chatgpt_account_entries(
        codex_home,
        runtime_active_auth,
        auth_credentials_store_mode,
    )?
    .into_iter()
    .map(|entry| {
        let auth = CodexAuth::from_auth_dot_json(
            codex_home,
            entry.auth_dot_json,
            auth_credentials_store_mode,
        )?;
        Ok(SavedChatgptAccountAuth {
            account: entry.account,
            auth,
        })
    })
    .collect()
}

pub(crate) fn switch_active_chatgpt_account(
    codex_home: &Path,
    account_id: &str,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    let saved_accounts =
        sync_saved_chatgpt_accounts_with_active_auth(codex_home, auth_credentials_store_mode)?;
    let auth = saved_accounts
        .accounts
        .into_iter()
        .find(|saved| saved_chatgpt_account_id(saved).as_deref() == Some(account_id))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("saved ChatGPT account not found: {account_id}"),
            )
        })?;

    super::save_auth(codex_home, &auth, auth_credentials_store_mode)?;
    if auth_credentials_store_mode != AuthCredentialsStoreMode::Ephemeral {
        super::logout(codex_home, AuthCredentialsStoreMode::Ephemeral)?;
    }
    Ok(())
}

fn sync_saved_chatgpt_accounts_with_active_auth(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<SavedChatgptAccounts> {
    let saved_accounts_storage = create_saved_chatgpt_accounts_storage(
        codex_home.to_path_buf(),
        auth_credentials_store_mode,
    );
    let mut saved_accounts = saved_accounts_storage.load()?.unwrap_or_default();

    if let Some(active_auth) =
        load_managed_chatgpt_auth_from_active_storage(codex_home, auth_credentials_store_mode)?
        && upsert_saved_chatgpt_account_in_registry(&mut saved_accounts.accounts, &active_auth)
    {
        saved_accounts_storage.save(&saved_accounts)?;
    }

    Ok(saved_accounts)
}

fn list_saved_chatgpt_account_entries(
    codex_home: &Path,
    runtime_active_auth: Option<&CodexAuth>,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<Vec<SavedChatgptAccountEntry>> {
    let saved_accounts =
        sync_saved_chatgpt_accounts_with_active_auth(codex_home, auth_credentials_store_mode)?;
    let active_id = runtime_active_auth.and_then(saved_chatgpt_account_id_for_runtime_auth);

    let mut entries = saved_accounts
        .accounts
        .iter()
        .filter_map(|auth| saved_chatgpt_account_entry(auth, active_id.as_deref()))
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .account
            .is_active
            .cmp(&left.account.is_active)
            .then_with(|| left.account.email.cmp(&right.account.email))
            .then_with(|| left.account.id.cmp(&right.account.id))
    });

    Ok(entries)
}

fn saved_chatgpt_account_entry(
    auth: &AuthDotJson,
    active_id: Option<&str>,
) -> Option<SavedChatgptAccountEntry> {
    let id = saved_chatgpt_account_id(auth)?;
    let tokens = auth.tokens.as_ref()?;
    Some(SavedChatgptAccountEntry {
        account: SavedChatgptAccount {
            is_active: active_id == Some(id.as_str()),
            id,
            email: tokens.id_token.email.clone(),
            account_id: tokens.account_id.clone(),
            chatgpt_user_id: tokens.id_token.chatgpt_user_id.clone(),
            chatgpt_workspace_id: tokens.id_token.chatgpt_account_id.clone(),
            plan_type: tokens.id_token.get_chatgpt_plan_type(),
        },
        auth_dot_json: auth.clone(),
    })
}

fn load_managed_chatgpt_auth_from_active_storage(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<Option<AuthDotJson>> {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    let auth = storage.load()?;
    Ok(auth.filter(is_managed_chatgpt_auth))
}

fn upsert_saved_chatgpt_account_in_registry(
    accounts: &mut Vec<AuthDotJson>,
    auth: &AuthDotJson,
) -> bool {
    if !is_managed_chatgpt_auth(auth) || saved_chatgpt_account_id(auth).is_none() {
        return false;
    }

    if let Some(index) = accounts
        .iter()
        .position(|existing| is_same_saved_chatgpt_account(existing, auth))
    {
        if accounts[index] == *auth {
            return false;
        }
        accounts[index] = auth.clone();
        return true;
    }

    accounts.push(auth.clone());
    true
}

fn is_same_saved_chatgpt_account(existing: &AuthDotJson, candidate: &AuthDotJson) -> bool {
    if !is_managed_chatgpt_auth(existing) || !is_managed_chatgpt_auth(candidate) {
        return false;
    }

    if let Some(existing_id) = saved_chatgpt_account_id(existing)
        && Some(existing_id) == saved_chatgpt_account_id(candidate)
    {
        return true;
    }

    let Some(existing_tokens) = existing.tokens.as_ref() else {
        return false;
    };
    let Some(candidate_tokens) = candidate.tokens.as_ref() else {
        return false;
    };

    if let (Some(existing_account_id), Some(candidate_account_id)) = (
        existing_tokens.account_id.as_deref(),
        candidate_tokens.account_id.as_deref(),
    ) {
        return existing_account_id == candidate_account_id;
    }

    if let (Some(existing_workspace_id), Some(candidate_workspace_id)) = (
        existing_tokens.id_token.chatgpt_account_id.as_deref(),
        candidate_tokens.id_token.chatgpt_account_id.as_deref(),
    ) {
        return existing_workspace_id == candidate_workspace_id;
    }

    if let (Some(existing_user_id), Some(candidate_user_id)) = (
        existing_tokens.id_token.chatgpt_user_id.as_deref(),
        candidate_tokens.id_token.chatgpt_user_id.as_deref(),
    ) {
        return existing_user_id == candidate_user_id;
    }

    if let (Some(existing_email), Some(candidate_email)) = (
        existing_tokens.id_token.email.as_deref(),
        candidate_tokens.id_token.email.as_deref(),
    ) {
        return existing_email == candidate_email;
    }

    false
}

fn saved_chatgpt_account_id_for_runtime_auth(auth: &CodexAuth) -> Option<String> {
    match auth {
        CodexAuth::Chatgpt(_) => auth
            .get_current_auth_json()
            .as_ref()
            .and_then(saved_chatgpt_account_id),
        CodexAuth::ApiKey(_) | CodexAuth::ChatgptAuthTokens(_) | CodexAuth::AgentIdentity(_) => {
            None
        }
    }
}

fn saved_chatgpt_account_id(auth: &AuthDotJson) -> Option<String> {
    if !is_managed_chatgpt_auth(auth) {
        return None;
    }

    let tokens = auth.tokens.as_ref()?;
    if let Some(account_id) = tokens.account_id.as_deref() {
        return Some(format!("account:{account_id}"));
    }
    if let Some(workspace_id) = tokens.id_token.chatgpt_account_id.as_deref() {
        return Some(format!("workspace:{workspace_id}"));
    }
    if let Some(user_id) = tokens.id_token.chatgpt_user_id.as_deref() {
        return Some(format!("user:{user_id}"));
    }
    if let Some(email) = tokens.id_token.email.as_deref() {
        return Some(format!("email:{email}"));
    }

    let mut hasher = Sha256::new();
    hasher.update(tokens.id_token.raw_jwt.as_bytes());
    let digest = hasher.finalize();
    let fallback = format!("{digest:x}");
    let truncated = fallback.get(..16).unwrap_or(&fallback);
    Some(format!("token:{truncated}"))
}

fn is_managed_chatgpt_auth(auth: &AuthDotJson) -> bool {
    auth.resolved_mode() == ApiAuthMode::Chatgpt
}

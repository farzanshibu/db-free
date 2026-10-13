// SOT: ai-profiles, ai-provider-profiles, ai-key-vault, ai-model-cache, ai-model-resolution

use crate::adapters::crypto;
use crate::error::{AppError, AppResult};
use crate::model::{
    AiDataAccess, AiKeyChange, AiModel, AiModelList, AiProfile, AiProvider, AiSettings, AiTestResult, AppSettings, ModelChoice,
};
use crate::model::settings::{MAX_AGENT_STEPS, MIN_AGENT_STEPS};
use crate::services::ai;
use crate::state::AppState;
use crate::store::{now_rfc3339, Store};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, Instant};

// WHAT:  Several AI providers at once, each with its own sealed key, plus the
//        cached list of models each one offers.
// WHY:   People use more than one provider (a hosted one for hard questions, a
//        local Ollama for private ones) and want to pick per message. Settings
//        keep the profile list; the keys never live in the settings document.
// HOW:   A key is sealed with the master key under `ai_key:<profile id>` in the
//        settings table, the same way connection secrets are sealed, and only
//        ever travels UI → here. `hydrate` derives what the UI shows (which
//        profile has a key, the default selection mirrored into the legacy
//        fields) on every read, so there is nothing to keep in sync. Settings
//        saved before profiles existed carry one provider, one model and one
//        key under `ai_key`; that becomes a profile with the id `default`.
// WHERE: src-tauri/src/services/settings.rs (calls `hydrate`),
//        src-tauri/src/commands/ai.rs, src/features/settings/AiSection.tsx

/// Id the pre-profile single provider is migrated to.
pub const LEGACY_PROFILE_ID: &str = "default";
/// The single key the app stored before profiles (services/settings.rs).
const LEGACY_KEY_SETTING: &str = "ai_key";
/// The settings document (services/settings.rs).
const APP_SETTING: &str = "app";

const MAX_PROFILES: usize = 12;
/// A refresh may be slow on a cold network; Test must answer quickly or say it did not.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn key_setting(profile_id: &str) -> String {
    format!("ai_key:{profile_id}")
}

fn models_setting(profile_id: &str) -> String {
    format!("ai_models:{profile_id}")
}

pub fn provider_label(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::None => "None",
        AiProvider::Anthropic => "Anthropic",
        AiProvider::Openai => "OpenAI",
        AiProvider::Openrouter => "OpenRouter",
        AiProvider::Ollama => "Ollama",
        AiProvider::Google => "Google Gemini",
    }
}

/// Ollama runs on the user's machine and takes no credential.
pub fn needs_key(provider: AiProvider) -> bool {
    !matches!(provider, AiProvider::Ollama | AiProvider::None)
}

// ---------------------------------------------------------------------------
// Reading: what the UI sees
// ---------------------------------------------------------------------------

fn has_key(store: &Store, profile_id: &str) -> AppResult<bool> {
    if store.get_setting(&key_setting(profile_id))?.is_some() {
        return Ok(true);
    }
    if profile_id == LEGACY_PROFILE_ID {
        return Ok(store.get_setting(LEGACY_KEY_SETTING)?.is_some());
    }
    Ok(false)
}

// WHAT:  Fill in everything that is derived rather than stored.
// WHY:   `has_api_key` per profile comes from the key store; the legacy
//        `provider` / `base_url` / `has_api_key` fields mirror the default
//        profile so code that only asks "is AI on?" keeps working; and a
//        settings document from before profiles gets its one provider lifted
//        into a profile.
// HOW:   Idempotent. `provider` reads `none` when the assistant is switched off
//        or no default is chosen — that is what "AI is on" means to a caller
//        that checks only the provider.
pub fn hydrate(store: &Store, ai: &mut AiSettings) -> AppResult<()> {
    if ai.profiles.is_empty() && ai.provider != AiProvider::None {
        ai.profiles.push(AiProfile {
            id: LEGACY_PROFILE_ID.to_string(),
            provider: ai.provider,
            label: provider_label(ai.provider).to_string(),
            base_url: ai.base_url.clone(),
            has_api_key: false,
        });
        ai.default_profile = Some(LEGACY_PROFILE_ID.to_string());
    }
    for profile in &mut ai.profiles {
        profile.has_api_key = has_key(store, &profile.id)?;
    }
    mirror_default(ai);
    Ok(())
}

fn mirror_default(ai: &mut AiSettings) {
    let default = ai.default_profile.as_ref().and_then(|id| ai.profiles.iter().find(|p| &p.id == id)).cloned();
    match default {
        Some(profile) => {
            ai.provider = if ai.data_access == AiDataAccess::Off { AiProvider::None } else { profile.provider };
            ai.base_url = profile.base_url;
            ai.has_api_key = profile.has_api_key;
        }
        None => {
            ai.default_profile = None;
            ai.provider = AiProvider::None;
        }
    }
}

// WHAT:  Keep the stored assistant settings when the general settings page saves.
// WHY:   The assistant's settings carry secrets and remote state (keys, model
//        lists), so they are saved by their own commands. A page that still
//        holds an older copy of them must not be able to roll them back.
pub fn keep_stored(store: &Store, incoming: &mut AiSettings) -> AppResult<()> {
    if let Some(stored) = stored_app_settings(store)? {
        *incoming = stored.ai;
    }
    Ok(())
}

fn stored_app_settings(store: &Store) -> AppResult<Option<AppSettings>> {
    match store.get_setting(APP_SETTING)? {
        Some(raw) => Ok(serde_json::from_str::<AppSettings>(&raw).ok()),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

fn seal_for_store(state: &AppState, key: &str) -> AppResult<String> {
    let sealed = crypto::seal(state.master_key()?, key.as_bytes())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(sealed))
}

fn unseal_from_store(state: &AppState, encoded: &str) -> AppResult<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| AppError::crypto("a stored AI key is corrupt"))?;
    let plain = crypto::open(state.master_key()?, &bytes)?;
    String::from_utf8(plain).map_err(|_| AppError::crypto("a stored AI key is not UTF-8"))
}

// WHAT:  The unsealed key for one profile, for the length of one request.
pub fn api_key(state: &AppState, profile_id: &str) -> AppResult<Option<String>> {
    let sealed = state.with_store(|store| {
        let own = store.get_setting(&key_setting(profile_id))?;
        match own {
            Some(value) => Ok(Some(value)),
            None if profile_id == LEGACY_PROFILE_ID => store.get_setting(LEGACY_KEY_SETTING),
            None => Ok(None),
        }
    })?;
    sealed.map(|encoded| unseal_from_store(state, &encoded)).transpose()
}

/// The default profile's key, for the paths that use the default provider as-is
/// (explaining a plan).
pub fn default_key(state: &AppState, ai: &AiSettings) -> AppResult<Option<String>> {
    match &ai.default_profile {
        Some(id) => api_key(state, id),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

fn clean_base_url(raw: Option<String>) -> AppResult<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::invalid_input("A base URL starts with http:// or https://."));
    }
    Ok(Some(trimmed.to_string()))
}

// WHAT:  Validate and tidy an incoming settings document before it is stored.
// WHY:   The UI is not trusted to keep ids unique or the default pointing at a
//        profile that exists; a bad document would otherwise surface later as
//        a chat that cannot find its provider.
pub fn normalize(mut ai: AiSettings) -> AppResult<AiSettings> {
    if ai.profiles.len() > MAX_PROFILES {
        return Err(AppError::invalid_input(format!("At most {MAX_PROFILES} providers can be configured.")));
    }
    let mut seen = HashSet::new();
    for profile in &mut ai.profiles {
        profile.id = profile.id.trim().to_string();
        if profile.id.is_empty() {
            return Err(AppError::invalid_input("Every provider needs an id."));
        }
        if !seen.insert(profile.id.clone()) {
            return Err(AppError::invalid_input(format!("Provider \"{}\" is listed twice.", profile.id)));
        }
        if profile.provider == AiProvider::None {
            return Err(AppError::invalid_input("Pick a provider for every entry."));
        }
        profile.label = profile.label.trim().to_string();
        if profile.label.is_empty() {
            profile.label = provider_label(profile.provider).to_string();
        }
        profile.base_url = clean_base_url(profile.base_url.take())?;
        profile.has_api_key = false;
    }
    ai.default_profile = ai.default_profile.map(|id| id.trim().to_string()).filter(|id| !id.is_empty());
    if let Some(id) = &ai.default_profile {
        if !ai.profiles.iter().any(|p| &p.id == id) {
            return Err(AppError::invalid_input("The default provider is not in the list."));
        }
    }
    ai.model = ai.model.trim().to_string();
    if ai.default_profile.is_some() && ai.model.is_empty() {
        return Err(AppError::invalid_input("Pick a default model."));
    }
    ai.has_api_key = false;
    ai.max_steps = ai.max_steps.clamp(MIN_AGENT_STEPS, MAX_AGENT_STEPS);
    mirror_default(&mut ai);
    Ok(ai)
}

// WHAT:  Store the assistant's settings and apply key changes, then hand back
//        the hydrated settings document.
// HOW:   Keys are sealed first (a failure there changes nothing); then one pass
//        writes the document, the keys, and removes the keys and cached model
//        lists of providers that were dropped.
pub fn save(state: &AppState, ai: AiSettings, keys: &[AiKeyChange]) -> AppResult<AppSettings> {
    let ai = normalize(ai)?;
    for change in keys {
        if !ai.profiles.iter().any(|p| p.id == change.profile_id) {
            return Err(AppError::invalid_input("A key was supplied for a provider that is not in the list."));
        }
    }
    let mut sealed: Vec<(String, Option<String>)> = Vec::new();
    for change in keys {
        let key = change.key.trim();
        let value = if key.is_empty() { None } else { Some(seal_for_store(state, key)?) };
        sealed.push((change.profile_id.clone(), value));
    }

    state.with_store(|store| {
        let mut app = stored_app_settings(store)?.unwrap_or_default();
        // What exists now includes the pre-profile provider that `hydrate` lifts
        // into a profile, so dropping it also drops its key.
        let mut current = app.ai.clone();
        hydrate(store, &mut current)?;
        let previous: Vec<String> = current.profiles.iter().map(|p| p.id.clone()).collect();
        app.ai = ai.clone();
        let raw = serde_json::to_string(&app).map_err(AppError::internal)?;
        store.set_setting(APP_SETTING, &raw)?;

        for (profile_id, value) in &sealed {
            match value {
                Some(encoded) => {
                    store.set_setting(&key_setting(profile_id), encoded)?;
                    // The pre-profile key is superseded, not left to shadow a later removal.
                    if profile_id == LEGACY_PROFILE_ID {
                        store.delete_setting(LEGACY_KEY_SETTING)?;
                    }
                }
                None => {
                    store.delete_setting(&key_setting(profile_id))?;
                    if profile_id == LEGACY_PROFILE_ID {
                        store.delete_setting(LEGACY_KEY_SETTING)?;
                    }
                }
            }
        }
        for id in previous.iter().filter(|id| !ai.profiles.iter().any(|p| &p.id == *id)) {
            store.delete_setting(&key_setting(id))?;
            store.delete_setting(&models_setting(id))?;
            if id == LEGACY_PROFILE_ID {
                store.delete_setting(LEGACY_KEY_SETTING)?;
            }
        }
        Ok(())
    })?;
    crate::services::settings::get(state)
}

// ---------------------------------------------------------------------------
// Model lists
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedModels {
    fetched_at: String,
    models: Vec<AiModel>,
}

fn cached_models(store: &Store, profile_id: &str) -> AppResult<Option<AiModelList>> {
    let Some(raw) = store.get_setting(&models_setting(profile_id))? else { return Ok(None) };
    Ok(serde_json::from_str::<CachedModels>(&raw).ok().map(|cache| AiModelList {
        profile_id: profile_id.to_string(),
        models: cache.models,
        fetched_at: Some(cache.fetched_at),
        cached: true,
    }))
}

fn write_models(store: &Store, profile_id: &str, models: &[AiModel]) -> AppResult<String> {
    let fetched_at = now_rfc3339();
    let raw = serde_json::to_string(&CachedModels { fetched_at: fetched_at.clone(), models: models.to_vec() })
        .map_err(AppError::internal)?;
    store.set_setting(&models_setting(profile_id), &raw)?;
    Ok(fetched_at)
}

/// The key to use: one typed into the form wins over the stored one.
fn key_for(state: &AppState, profile_id: &str, typed: Option<&str>) -> AppResult<Option<String>> {
    match typed.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => Ok(Some(key.to_string())),
        None => api_key(state, profile_id),
    }
}

// WHAT:  A profile's chat models: the cache when it has one, the provider when
//        it does not or when a refresh is asked for.
pub async fn models(state: &AppState, profile: &AiProfile, typed_key: Option<&str>, refresh: bool) -> AppResult<AiModelList> {
    if !refresh {
        if let Some(cached) = state.with_store(|store| cached_models(store, &profile.id))? {
            return Ok(cached);
        }
    }
    let key = key_for(state, &profile.id, typed_key)?;
    let fetched = ai::fetch_models(profile.provider, profile.base_url.as_deref(), key.as_deref(), REFRESH_TIMEOUT).await?;
    let fetched_at = state.with_store(|store| write_models(store, &profile.id, &fetched))?;
    Ok(AiModelList { profile_id: profile.id.clone(), models: fetched, fetched_at: Some(fetched_at), cached: false })
}

// WHAT:  "Test": can this endpoint be reached with this key, within 5 seconds?
// WHY:   The model list is the cheapest authenticated call every provider has,
//        so it answers both questions and, on success, fills the cache.
pub async fn test(state: &AppState, profile: &AiProfile, typed_key: Option<&str>, model: Option<&str>) -> AppResult<AiTestResult> {
    let started = Instant::now();
    let key = key_for(state, &profile.id, typed_key)?;
    let outcome = ai::fetch_models(profile.provider, profile.base_url.as_deref(), key.as_deref(), TEST_TIMEOUT).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(match outcome {
        Ok(list) => {
            // Only a profile that is saved owns a cache; a draft being tested must not leave one behind.
            let saved = state.with_store(|store| {
                let stored = stored_app_settings(store)?;
                Ok(stored.is_some_and(|app| app.ai.profiles.iter().any(|p| p.id == profile.id)))
            })?;
            if saved {
                state.with_store(|store| write_models(store, &profile.id, &list))?;
            }
            let count = u32::try_from(list.len()).unwrap_or(u32::MAX);
            let wanted = model.map(str::trim).filter(|m| !m.is_empty());
            let note = match wanted {
                Some(m) if !list.is_empty() && !list.iter().any(|candidate| candidate.id == m) => {
                    format!(" \"{m}\" is not in the list, so check the spelling.")
                }
                _ => String::new(),
            };
            AiTestResult {
                ok: true,
                message: format!("Connected in {elapsed_ms} ms — {count} model{}.{note}", if count == 1 { "" } else { "s" }),
                model_count: count,
                elapsed_ms,
            }
        }
        Err(err) => AiTestResult { ok: false, message: err.message().to_string(), model_count: 0, elapsed_ms },
    })
}

// ---------------------------------------------------------------------------
// Choosing a model for a run
// ---------------------------------------------------------------------------

/// Everything a run needs to talk to one model. Owned, so it can be held across awaits.
#[derive(Clone)]
pub struct ResolvedModel {
    pub profile_id: String,
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for ResolvedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedModel")
            .field("profile_id", &self.profile_id)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Refuses to run when the assistant is off or nothing is configured.
pub fn ensure_enabled(ai: &AiSettings) -> AppResult<()> {
    if ai.data_access == AiDataAccess::Off {
        return Err(AppError::invalid_input("The assistant is turned off. Turn it on in Settings → AI."));
    }
    if ai.default_profile.is_none() && ai.profiles.is_empty() {
        return Err(AppError::invalid_input("Add an AI provider in Settings → AI first."));
    }
    Ok(())
}

// WHAT:  The model a message uses: the one the chat picked, else the default.
// WHY:   The picker in the chat input chooses per message, so a profile can be
//        gone (deleted since) by the time a message is sent; that is a clear
//        error, not a silent fall back to a different provider.
pub fn resolve(state: &AppState, ai: &AiSettings, choice: Option<&ModelChoice>) -> AppResult<ResolvedModel> {
    ensure_enabled(ai)?;
    let (profile_id, model) = match choice {
        Some(choice) => (choice.profile_id.clone(), choice.model.trim().to_string()),
        None => match &ai.default_profile {
            Some(id) => (id.clone(), ai.model.trim().to_string()),
            None => return Err(AppError::invalid_input("Choose a default AI provider in Settings → AI.")),
        },
    };
    let profile = ai
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| AppError::not_found("That AI provider was removed. Pick another model."))?;
    if model.is_empty() {
        return Err(AppError::invalid_input("Set an AI model in Settings → AI."));
    }
    let key = api_key(state, &profile.id)?;
    if needs_key(profile.provider) && key.is_none() {
        return Err(AppError::invalid_input(format!("Add your {} API key in Settings → AI.", provider_label(profile.provider))));
    }
    Ok(ResolvedModel {
        profile_id: profile.id.clone(),
        provider: profile.provider,
        model,
        base_url: profile.base_url.clone(),
        api_key: key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::AgentAutonomy;

    fn state() -> AppState {
        AppState::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")), Box::new(MemoryKeyProvider::default()))
    }

    fn profile(id: &str, provider: AiProvider) -> AiProfile {
        AiProfile { id: id.into(), provider, label: String::new(), base_url: None, has_api_key: true }
    }

    fn settings(profiles: Vec<AiProfile>, default: Option<&str>, model: &str) -> AiSettings {
        AiSettings {
            provider: AiProvider::None,
            model: model.into(),
            base_url: None,
            has_api_key: false,
            autonomy: AgentAutonomy::AskOnWrite,
            data_access: AiDataAccess::Full,
            default_profile: default.map(str::to_string),
            profiles,
            max_steps: 12,
        }
    }

    fn change(profile_id: &str, key: &str) -> AiKeyChange {
        AiKeyChange { profile_id: profile_id.into(), key: key.into() }
    }

    #[test]
    fn normalize_rejects_a_broken_document() {
        let dup = settings(vec![profile("a", AiProvider::Openai), profile("a", AiProvider::Google)], None, "");
        assert!(normalize(dup).is_err(), "duplicate ids");
        let none = settings(vec![profile("a", AiProvider::None)], None, "");
        assert!(normalize(none).is_err(), "a profile needs a provider");
        let dangling = settings(vec![profile("a", AiProvider::Openai)], Some("b"), "gpt-4.1");
        assert!(normalize(dangling).is_err(), "default must exist");
        let no_model = settings(vec![profile("a", AiProvider::Openai)], Some("a"), "  ");
        assert!(normalize(no_model).is_err(), "default needs a model");
        let mut bad_url = profile("a", AiProvider::Openai);
        bad_url.base_url = Some("ftp://proxy".into());
        assert!(normalize(settings(vec![bad_url], None, "")).is_err());
        let many: Vec<AiProfile> = (0..=MAX_PROFILES).map(|i| profile(&format!("p{i}"), AiProvider::Openai)).collect();
        assert!(normalize(settings(many, None, "")).is_err());
    }

    #[test]
    fn normalize_keeps_the_step_limit_within_its_range() {
        let mut ai = settings(Vec::new(), None, "");
        ai.max_steps = 0;
        assert_eq!(normalize(ai.clone()).unwrap_or_else(|e| panic!("{e}")).max_steps, MIN_AGENT_STEPS);
        ai.max_steps = 5_000;
        assert_eq!(normalize(ai.clone()).unwrap_or_else(|e| panic!("{e}")).max_steps, MAX_AGENT_STEPS);
        ai.max_steps = 30;
        assert_eq!(normalize(ai).unwrap_or_else(|e| panic!("{e}")).max_steps, 30);
    }

    #[test]
    fn normalize_tidies_labels_urls_and_mirrors_the_default() {
        let mut p = profile(" work ", AiProvider::Openai);
        p.base_url = Some(" https://proxy.example/v1/ ".into());
        let ai = normalize(settings(vec![p], Some("work"), " gpt-4.1 ")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ai.profiles[0].id, "work");
        assert_eq!(ai.profiles[0].label, "OpenAI", "an empty label takes the provider's name");
        assert_eq!(ai.profiles[0].base_url.as_deref(), Some("https://proxy.example/v1"));
        assert!(!ai.profiles[0].has_api_key, "the client's flag is never trusted");
        assert_eq!(ai.model, "gpt-4.1");
        assert_eq!(ai.provider, AiProvider::Openai);
        assert_eq!(ai.base_url.as_deref(), Some("https://proxy.example/v1"));
    }

    #[test]
    fn switching_the_assistant_off_reads_as_no_provider_and_back_on_restores_it() {
        let mut ai = normalize(settings(vec![profile("a", AiProvider::Google)], Some("a"), "gemini-2.5-pro")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ai.provider, AiProvider::Google);
        ai.data_access = AiDataAccess::Off;
        mirror_default(&mut ai);
        assert_eq!(ai.provider, AiProvider::None);
        ai.data_access = AiDataAccess::Schema;
        mirror_default(&mut ai);
        assert_eq!(ai.provider, AiProvider::Google, "the provider is derived, so nothing was lost");
    }

    #[test]
    fn a_pre_profile_document_becomes_a_default_profile_with_its_key() {
        let state = state();
        // What an older build stored: one provider on the document, one sealed key.
        let legacy = AiSettings { provider: AiProvider::Anthropic, model: "claude-opus-5".into(), ..AiSettings::default() };
        let app = AppSettings { ai: legacy, ..AppSettings::default() };
        let encoded = seal_for_store(&state, "sk-old").unwrap_or_else(|e| panic!("{e}"));
        state
            .with_store(|store| {
                store.set_setting(APP_SETTING, &serde_json::to_string(&app).map_err(AppError::internal)?)?;
                store.set_setting(LEGACY_KEY_SETTING, &encoded)
            })
            .unwrap_or_else(|e| panic!("{e}"));

        let loaded = crate::services::settings::get(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loaded.ai.profiles.len(), 1);
        assert_eq!(loaded.ai.profiles[0].id, LEGACY_PROFILE_ID);
        assert!(loaded.ai.profiles[0].has_api_key);
        assert_eq!(loaded.ai.default_profile.as_deref(), Some(LEGACY_PROFILE_ID));
        assert_eq!(loaded.ai.provider, AiProvider::Anthropic);
        assert_eq!(api_key(&state, LEGACY_PROFILE_ID).unwrap_or_default().as_deref(), Some("sk-old"));
    }

    #[test]
    fn each_profile_keeps_its_own_key_and_removing_a_profile_removes_its_key() {
        let state = state();
        let ai = settings(vec![profile("work", AiProvider::Openai), profile("home", AiProvider::Google)], Some("work"), "gpt-4.1");
        let saved = save(&state, ai, &[change("work", "sk-work"), change("home", "g-home")]).unwrap_or_else(|e| panic!("{e}"));
        assert!(saved.ai.profiles.iter().all(|p| p.has_api_key));
        assert_eq!(saved.ai.provider, AiProvider::Openai);
        assert!(saved.ai.has_api_key);
        assert_eq!(api_key(&state, "work").unwrap_or_default().as_deref(), Some("sk-work"));
        assert_eq!(api_key(&state, "home").unwrap_or_default().as_deref(), Some("g-home"));

        // Dropping "home" drops its key and cached models; "work" is untouched.
        state
            .with_store(|store| write_models(store, "home", &[AiModel { id: "gemini-2.5-pro".into(), label: "x".into() }]).map(|_| ()))
            .unwrap_or_else(|e| panic!("{e}"));
        let kept = settings(vec![profile("work", AiProvider::Openai)], Some("work"), "gpt-4.1");
        let after = save(&state, kept, &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.ai.profiles.len(), 1);
        assert_eq!(api_key(&state, "home").unwrap_or_default(), None);
        assert_eq!(api_key(&state, "work").unwrap_or_default().as_deref(), Some("sk-work"));
        assert!(state.with_store(|s| cached_models(s, "home")).unwrap_or_default().is_none());
    }

    #[test]
    fn an_empty_key_change_removes_the_key_and_the_settings_page_cannot_roll_ai_back() {
        let state = state();
        let ai = settings(vec![profile("work", AiProvider::Openai)], Some("work"), "gpt-4.1");
        save(&state, ai, &[change("work", "sk-work")]).unwrap_or_else(|e| panic!("{e}"));

        // The general settings page saves a stale copy of the document.
        let mut stale = crate::services::settings::get(&state).unwrap_or_else(|e| panic!("{e}"));
        stale.ai = AiSettings::default();
        let after = crate::services::settings::save(&state, &stale, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.ai.profiles.len(), 1, "the assistant's settings survive a general save");
        assert_eq!(after.ai.provider, AiProvider::Openai);

        let cleared = save(&state, after.ai, &[change("work", "")]).unwrap_or_else(|e| panic!("{e}"));
        assert!(!cleared.ai.profiles[0].has_api_key);
        assert_eq!(api_key(&state, "work").unwrap_or_default(), None);
    }

    #[test]
    fn a_key_for_an_unknown_profile_is_refused() {
        let state = state();
        let ai = settings(vec![profile("work", AiProvider::Openai)], Some("work"), "gpt-4.1");
        assert!(save(&state, ai, &[change("ghost", "k")]).is_err());
    }

    #[test]
    fn resolve_prefers_the_chats_choice_and_demands_a_key_where_one_is_needed() {
        let state = state();
        let ai = settings(vec![profile("work", AiProvider::Openai), profile("local", AiProvider::Ollama)], Some("work"), "gpt-4.1");
        let saved = save(&state, ai, &[change("work", "sk-work")]).unwrap_or_else(|e| panic!("{e}"));

        let default = resolve(&state, &saved.ai, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((default.provider, default.model.as_str()), (AiProvider::Openai, "gpt-4.1"));
        assert_eq!(default.api_key.as_deref(), Some("sk-work"));

        let pick = ModelChoice { profile_id: "local".into(), model: "llama3.1".into() };
        let local = resolve(&state, &saved.ai, Some(&pick)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((local.provider, local.api_key), (AiProvider::Ollama, None), "Ollama needs no key");

        let gone = ModelChoice { profile_id: "deleted".into(), model: "m".into() };
        assert!(matches!(resolve(&state, &saved.ai, Some(&gone)).err(), Some(AppError::NotFound { .. })));

        let unkeyed = save(&state, settings(vec![profile("work", AiProvider::Openai)], Some("work"), "gpt-4.1"), &[change("work", "")])
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(resolve(&state, &unkeyed.ai, None).err(), Some(AppError::InvalidInput { .. })));
    }

    #[test]
    fn off_and_empty_settings_refuse_to_run() {
        let mut ai = settings(vec![profile("a", AiProvider::Openai)], Some("a"), "m");
        assert!(ensure_enabled(&ai).is_ok());
        ai.data_access = AiDataAccess::Off;
        assert!(ensure_enabled(&ai).is_err());
        assert!(ensure_enabled(&AiSettings::default()).is_err());
    }

    #[test]
    fn model_cache_round_trips() {
        let state = state();
        let list = vec![AiModel { id: "a".into(), label: "A".into() }];
        let stamp = state.with_store(|s| write_models(s, "p", &list)).unwrap_or_default();
        let back = state.with_store(|s| cached_models(s, "p")).unwrap_or_default();
        assert_eq!(back.as_ref().map(|b| b.models.clone()), Some(list));
        assert_eq!(back.and_then(|b| b.fetched_at), Some(stamp));
    }

    #[test]
    fn removing_the_pre_profile_provider_removes_its_key_too() {
        let state = state();
        let legacy = AiSettings { provider: AiProvider::Openai, model: "gpt-4.1".into(), ..AiSettings::default() };
        let encoded = seal_for_store(&state, "sk-old").unwrap_or_else(|e| panic!("{e}"));
        let app = AppSettings { ai: legacy, ..AppSettings::default() };
        state
            .with_store(|store| {
                store.set_setting(APP_SETTING, &serde_json::to_string(&app).map_err(AppError::internal)?)?;
                store.set_setting(LEGACY_KEY_SETTING, &encoded)
            })
            .unwrap_or_else(|e| panic!("{e}"));

        // The first save the new settings page makes: the migrated provider is dropped.
        let saved = save(&state, AiSettings::default(), &[]).unwrap_or_else(|e| panic!("{e}"));
        assert!(saved.ai.profiles.is_empty());
        assert_eq!(saved.ai.provider, AiProvider::None, "nothing left to resurrect from the old fields");
        assert_eq!(api_key(&state, LEGACY_PROFILE_ID).unwrap_or_default(), None);
        assert!(state.with_store(|s| s.get_setting(LEGACY_KEY_SETTING)).unwrap_or_default().is_none());
    }
}

// SOT: update-commands, ipc-self-update, updater-endpoint

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::{UpdateProgress, UpdateStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

/// Event the UI listens on while an update downloads.
const PROGRESS_EVENT: &str = "update:progress";

// WHAT:  An update that has finished downloading and is waiting for a restart.
// WHY:   Downloading and installing are one call in the plugin, but they are two
//        moments for the user: the bytes can arrive quietly in the background,
//        while replacing the running binary has to wait for their say-so.
//        Holding the bytes here is what lets the toast offer "Restart" with
//        nothing left to download.
// HOW:   Managed by Tauri (src-tauri/src/lib.rs). The lock is only ever held to
//        swap the Option, never across an await.
#[derive(Default)]
pub struct StagedUpdate(Mutex<Option<Vec<u8>>>);

impl StagedUpdate {
    fn put(&self, bytes: Vec<u8>) -> AppResult<()> {
        let mut slot = self.0.lock().map_err(|_| AppError::internal("staged update lock poisoned"))?;
        *slot = Some(bytes);
        Ok(())
    }

    fn take(&self) -> AppResult<Option<Vec<u8>>> {
        let mut slot = self.0.lock().map_err(|_| AppError::internal("staged update lock poisoned"))?;
        Ok(slot.take())
    }
}

// WHAT:  Self-update: what the signed release feed offers, and installing it.
// WHY:   Installers were download-by-hand; the feed is the latest.json the
//        release workflow publishes next to every bundle.
// HOW:   The plugin verifies each release against the public key in
//        tauri.conf.json before anything is offered, so a compromised endpoint
//        cannot serve an unsigned build. This lives in the command layer rather
//        than in a service because it orchestrates neither store nor
//        integrations — it is the Tauri runtime itself, which services may not
//        import (scripts/guardrail.py).
// WHERE: .github/workflows/release.yml (publishes latest.json), tauri.conf.json
#[tauri::command]
pub async fn check_update(app: AppHandle) -> AppResult<UpdateStatus> {
    guard::local("check_update", async move {
        let current = app.package_info().version.to_string();
        let updater = app.updater().map_err(|e| AppError::internal(format!("updater unavailable: {e}")))?;
        let found = updater.check().await.map_err(|e| AppError::internal(format!("update check failed: {e}")))?;
        Ok(match found {
            Some(update) => UpdateStatus {
                current,
                available: Some(update.version.clone()),
                notes: update.body.clone(),
                published: update.date.map(|d| d.to_string()),
            },
            None => UpdateStatus { current, available: None, notes: None, published: None },
        })
    })
    .await
}

// WHAT:  Fetches the update's bytes and holds them, without touching the
//        installed binary. Reports what it staged so the caller can name the
//        version; `available: None` means the feed had nothing newer.
// WHY:   The app downloads on launch so that, by the time anyone is told an
//        update exists, restarting is instant rather than the start of a
//        multi-megabyte wait on someone who was about to stop working.
// WHERE: src/App.tsx (calls this at startup, then offers Restart on the toast)
#[tauri::command]
pub async fn download_update(app: AppHandle, staged: State<'_, StagedUpdate>) -> AppResult<UpdateStatus> {
    guard::local("download_update", async move {
        let current = app.package_info().version.to_string();
        let updater = app.updater().map_err(|e| AppError::internal(format!("updater unavailable: {e}")))?;
        let found = updater.check().await.map_err(|e| AppError::internal(format!("update check failed: {e}")))?;
        let Some(update) = found else {
            return Ok(UpdateStatus { current, available: None, notes: None, published: None });
        };
        let (progress, finished) = progress_handlers(&app);
        let bytes = update
            .download(progress, finished)
            .await
            .map_err(|e| AppError::internal(format!("update download failed: {e}")))?;
        staged.put(bytes)?;
        Ok(UpdateStatus {
            current,
            available: Some(update.version.clone()),
            notes: update.body.clone(),
            published: update.date.map(|d| d.to_string()),
        })
    })
    .await
}

// WHAT:  Installs the update and restarts into it. Uses the staged bytes when
//        `download_update` already fetched them, and downloads first otherwise
//        (Settings → Updates, where the user asked without waiting for startup).
#[tauri::command]
pub async fn install_update(app: AppHandle, staged: State<'_, StagedUpdate>) -> AppResult<()> {
    let ready = staged.take()?;
    guard::local("install_update", async move {
        let updater = app.updater().map_err(|e| AppError::internal(format!("updater unavailable: {e}")))?;
        let update = updater
            .check()
            .await
            .map_err(|e| AppError::internal(format!("update check failed: {e}")))?
            .ok_or_else(|| AppError::not_found("no update available"))?;
        match ready {
            Some(bytes) => update
                .install(bytes)
                .map_err(|e| AppError::internal(format!("update install failed: {e}")))?,
            None => {
                let (progress, finished) = progress_handlers(&app);
                update
                    .download_and_install(progress, finished)
                    .await
                    .map_err(|e| AppError::internal(format!("update install failed: {e}")))?;
            }
        }
        // Restarting is what makes the installed binary the running one, so it
        // belongs here rather than in something the UI must remember to call.
        app.restart();
    })
    .await
}

// WHAT:  The pair of callbacks the plugin reports download progress through.
// WHY:   Bytes arrive per chunk, so the running total lives outside the closure;
//        an emit that fails costs a progress bar, not the update.
fn progress_handlers(app: &AppHandle) -> (impl Fn(usize, Option<u64>), impl FnOnce()) {
    let downloaded = Arc::new(AtomicU64::new(0));
    let progress = {
        let app = app.clone();
        let downloaded = Arc::clone(&downloaded);
        move |chunk: usize, total: Option<u64>| {
            let sent = downloaded.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
            let _ = app.emit(PROGRESS_EVENT, UpdateProgress { downloaded: sent, total, done: false });
        }
    };
    let finished = {
        let app = app.clone();
        move || {
            let sent = downloaded.load(Ordering::Relaxed);
            let _ = app.emit(PROGRESS_EVENT, UpdateProgress { downloaded: sent, total: Some(sent), done: true });
        }
    };
    (progress, finished)
}

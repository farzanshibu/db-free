// SOT: query-session-service, close-query-session

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::integrations::pooling::valid_session_key;

// WHAT:  Releases the dedicated database connection of a query tab that closed.
// WHY:   A tab that names a session key keeps one connection for as long as it is
//        open (`ExecuteQueryRequest::session_key`). Closing the tab is the moment
//        to give it back, rather than leaving it to the idle timeout.
// HOW:   Closing a key nothing opened, or one the engine never pinned, is not an
//        error: the tab may have closed before its first Run.
// WHERE: src-tauri/src/commands/sessions.rs, src/stores/sessions.ts
pub async fn close(ctx: &SessionCtx, key: &str) -> AppResult<()> {
    let key = key.trim();
    if !valid_session_key(key) {
        return Err(AppError::invalid_input("The query tab's session id is not valid."));
    }
    ctx.integration.close_session(key).await;
    Ok(())
}

// SOT: keys-service, key-scan, key-info

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::model::{RedisKeyInfo, RedisKeyPage, RedisKeyScan};

// WHAT:  One page of the key browser's cursor-paged scan.
// WHY:   A key-value store's keyspace is too large to list as a catalog; the
//        sidebar walks it a page at a time and searches it on the server.
// HOW:   Thin on purpose: the adapter owns SCAN, MATCH and the type / preview
//        pipelines. The service is where a second key-value family would be
//        told apart from the first.
// WHERE: src-tauri/src/integrations/redis.rs (scan_keys), src/features/keys/keyBrowser.ts
pub async fn scan(ctx: &SessionCtx, scan: &RedisKeyScan) -> AppResult<RedisKeyPage> {
    ctx.integration.scan_keys(scan).await
}

// WHAT:  The metadata strip of the key tab: type, TTL, memory, encoding, idle time, size.
pub async fn info(ctx: &SessionCtx, key: &str) -> AppResult<RedisKeyInfo> {
    if key.is_empty() {
        return Err(AppError::invalid_input("A key name is required."));
    }
    ctx.integration.key_info(key).await
}

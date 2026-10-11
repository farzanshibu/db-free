// SOT: connections-migrations, store-migration-steps

// WHAT:  Connection folders, tags and transient connections: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[];

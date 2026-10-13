// SOT: auth-migrations, store-migration-steps

// WHAT:  Connection sign-in methods: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
//        1 — the sign-in method (`auth_config`, JSON of model::auth::ConnectionAuth,
//            NULL = password), which of its secrets are set (`auth_secret_flags`,
//            JSON) and the sealed secrets themselves (`auth_secret_ciphertext`,
//            AES-256-GCM over a JSON document). Existing connections stay NULL:
//            they keep signing in with their password.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list),
//        src-tauri/src/store/connection_auth.rs (reads and writes the columns)
pub const STEPS: &[&str] = &[
    "ALTER TABLE connections ADD COLUMN auth_config TEXT;
     ALTER TABLE connections ADD COLUMN auth_secret_flags TEXT;
     ALTER TABLE connections ADD COLUMN auth_secret_ciphertext BLOB;",
];

#[cfg(test)]
mod tests {
    use super::STEPS;

    // A connection saved by a build before sign-in methods existed keeps working:
    // the new columns are NULL, which reads as "password, no secrets".
    #[test]
    fn existing_connections_gain_null_auth_columns() {
        let conn = rusqlite::Connection::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        conn.execute_batch(
            "CREATE TABLE connections (id TEXT PRIMARY KEY, name TEXT NOT NULL, secret_ciphertext BLOB);
             INSERT INTO connections (id, name, secret_ciphertext) VALUES ('old', 'legacy', x'0102');",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        for step in STEPS {
            conn.execute_batch(step).unwrap_or_else(|e| panic!("{e}"));
        }
        // True when the column is NULL for the legacy row.
        let is_null = |name: &str| -> bool {
            let sql = format!("SELECT {name} IS NULL FROM connections WHERE id = 'old'");
            conn.query_row(&sql, [], |row| row.get::<_, bool>(0)).unwrap_or_else(|e| panic!("{name}: {e}"))
        };
        let (config, flags, secret) = (is_null("auth_config"), is_null("auth_secret_flags"), is_null("auth_secret_ciphertext"));
        let password: Vec<u8> = conn
            .query_row("SELECT secret_ciphertext FROM connections WHERE id = 'old'", [], |row| row.get(0))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(config && flags && secret, "the new columns start NULL");
        assert_eq!(password, vec![1, 2], "the password is untouched");
    }
}

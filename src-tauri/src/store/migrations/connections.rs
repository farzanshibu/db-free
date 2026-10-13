// SOT: connections-migrations, store-migration-steps

// WHAT:  Connection folders, tags and transient connections: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
//        1 — per-connection URI parameters, SRV flag, TLS file paths and the
//            transient flag. Saved MongoDB connections that had a user used to be
//            forced onto `authSource=admin`; the adapter no longer hard-codes it,
//            so that value is written down once here and nothing changes for them.
//        2 — folders as entities (empty folders, several per connection), tags
//            with the five seeded presets, and the old free-text `folder` column
//            moved into the new tables (the column stays, unused).
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    "ALTER TABLE connections ADD COLUMN uri_params TEXT;
     ALTER TABLE connections ADD COLUMN srv INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE connections ADD COLUMN tls_ca_path TEXT;
     ALTER TABLE connections ADD COLUMN tls_cert_path TEXT;
     ALTER TABLE connections ADD COLUMN tls_key_path TEXT;
     ALTER TABLE connections ADD COLUMN transient INTEGER NOT NULL DEFAULT 0;
     UPDATE connections SET uri_params = 'authSource=admin'
      WHERE engine = 'mongodb' AND username IS NOT NULL AND trim(username) != '';",
    "CREATE TABLE connection_folders (
         id TEXT PRIMARY KEY,
         name TEXT NOT NULL COLLATE NOCASE UNIQUE,
         sort_order INTEGER NOT NULL DEFAULT 0,
         created_at TEXT NOT NULL
     );
     CREATE TABLE connection_folder_members (
         connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
         folder_id TEXT NOT NULL REFERENCES connection_folders(id) ON DELETE CASCADE,
         PRIMARY KEY (connection_id, folder_id)
     );
     CREATE INDEX connection_folder_members_folder_idx ON connection_folder_members (folder_id);
     CREATE TABLE connection_tags (
         id TEXT PRIMARY KEY,
         name TEXT NOT NULL COLLATE NOCASE UNIQUE,
         color TEXT NOT NULL,
         is_preset INTEGER NOT NULL DEFAULT 0,
         sort_order INTEGER NOT NULL DEFAULT 0,
         created_at TEXT NOT NULL
     );
     CREATE TABLE connection_tag_links (
         connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
         tag_id TEXT NOT NULL REFERENCES connection_tags(id) ON DELETE CASCADE,
         PRIMARY KEY (connection_id, tag_id)
     );
     CREATE INDEX connection_tag_links_tag_idx ON connection_tag_links (tag_id);
     INSERT INTO connection_tags (id, name, color, is_preset, sort_order, created_at) VALUES
         ('preset-production', 'Production', 'red', 1, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
         ('preset-staging', 'Staging', 'amber', 1, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
         ('preset-development', 'Development', 'blue', 1, 2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
         ('preset-testing', 'Testing', 'purple', 1, 3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
         ('preset-local', 'Local', 'gray', 1, 4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
     INSERT INTO connection_folders (id, name, sort_order, created_at)
         SELECT lower(hex(randomblob(16))), name, ROW_NUMBER() OVER (ORDER BY name COLLATE NOCASE) - 1,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
           FROM (SELECT MIN(trim(folder)) AS name FROM connections
                  WHERE folder IS NOT NULL AND trim(folder) != ''
                  GROUP BY lower(trim(folder)));
     INSERT INTO connection_folder_members (connection_id, folder_id)
         SELECT c.id, f.id FROM connections c
           JOIN connection_folders f ON f.name = trim(c.folder)
          WHERE c.folder IS NOT NULL AND trim(c.folder) != '';",
];

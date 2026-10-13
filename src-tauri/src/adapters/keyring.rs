// SOT: master-key-provider, os-keychain, keyring-adapter, key-provider-trait, master-key-file-fallback, key-lookup

use crate::adapters::crypto::MasterKey;
use crate::error::{AppError, AppResult};
use crate::model::KeyStorage;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

// WHAT:  The answer to "where is the master key?", with the three outcomes kept apart.
// WHY:   A missing key and an unreadable keychain look alike to a caller that
//        only sees `Err`, but they call for opposite responses. `Missing` is
//        final: nothing was ever stored, or it was deleted. `Unavailable` is
//        transient: the keychain is locked, denied or absent right now and the
//        key may well be inside. Treating the second as the first is how a
//        denied prompt used to mint a new key and orphan every saved password.
pub enum KeyLookup {
    Found(MasterKey),
    Missing,
    Unavailable(String),
}

// WHAT:  Where the master key comes from.
// WHY:   A trait so tests use an in-memory key and never touch the real keychain.
// HOW:   `lookup` only reads. `create` generates and stores a new key and is the
//        caller's decision: `AppState::master_key` calls it only when no sealed
//        secret exists that a new key would orphan.
// WHERE: src-tauri/src/state.rs (cached once per process)
pub trait KeyProvider: Send + Sync {
    fn lookup(&self) -> AppResult<KeyLookup>;
    fn create(&self) -> AppResult<MasterKey>;
    /// Where the key lives right now.
    fn storage(&self) -> KeyStorage;
    /// The key file's path, when the key lives in one.
    fn key_path(&self) -> Option<PathBuf> {
        None
    }
}

fn encode_key(key: &MasterKey) -> String {
    base64::engine::general_purpose::STANDARD.encode(key.as_bytes())
}

fn decode_key(encoded: &str) -> AppResult<MasterKey> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| AppError::keyring("stored master key is not valid base64"))?;
    MasterKey::from_bytes(&bytes)
}

// WHAT:  The master key as a file, for machines with no usable OS keychain.
// WHY:   Headless Linux, containers and locked-down desktops have no Secret
//        Service; without a fallback the app could not save a password at all.
// HOW:   Created with mode 0600 and never widened: a looser mode found on load
//        is tightened. This is weaker than a keychain (anyone who can read the
//        file can open the saved passwords), which is why the UI shows a
//        standing warning while the key lives here.
struct KeyFile {
    path: PathBuf,
}

impl KeyFile {
    fn load(&self) -> AppResult<Option<MasterKey>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                restrict_permissions(&self.path);
                decode_key(&text).map(Some)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(AppError::keyring(format!("could not read the master key file {}: {err}", self.path.display()))),
        }
    }

    fn store(&self, key: &MasterKey) -> AppResult<()> {
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::keyring)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|err| AppError::keyring(format!("could not create the master key file {}: {err}", self.path.display())))?;
        file.write_all(encode_key(key).as_bytes()).map_err(AppError::keyring)?;
        file.write_all(b"\n").map_err(AppError::keyring)?;
        file.sync_all().map_err(AppError::keyring)
    }
}

// WHAT:  Owner read/write only. Best effort: a filesystem without modes (FAT,
//        some network shares) cannot do better, and Windows relies on the ACL
//        of the user profile directory.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

// WHAT:  macOS Keychain / Windows Credential Manager / Linux Secret Service,
//        with an optional 0600 file when the keychain cannot be used.
pub struct OsKeyring {
    service: String,
    account: String,
    file: Option<KeyFile>,
    using_file: AtomicBool,
}

impl OsKeyring {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> OsKeyring {
        OsKeyring { service: service.into(), account: account.into(), file: None, using_file: AtomicBool::new(false) }
    }

    /// Falls back to a key file at `path` when the OS keychain is unusable.
    pub fn with_file_fallback(mut self, path: PathBuf) -> OsKeyring {
        self.file = Some(KeyFile { path });
        self
    }

    fn entry(&self) -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(&self.service, &self.account)
    }

    // WHAT:  Reads the keychain entry: Some(key), None for "no such entry", Err for "could not ask".
    fn os_load(&self) -> Result<Option<MasterKey>, OsFailure> {
        let entry = self.entry().map_err(|e| OsFailure::Unavailable(e.to_string()))?;
        match entry.get_password() {
            Ok(encoded) => decode_key(&encoded).map(Some).map_err(OsFailure::Corrupt),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(OsFailure::Unavailable(err.to_string())),
        }
    }
}

enum OsFailure {
    /// The keychain answered with something that is not a key.
    Corrupt(AppError),
    /// The keychain could not be asked (locked, denied, not installed).
    Unavailable(String),
}

impl Default for OsKeyring {
    fn default() -> Self {
        OsKeyring::new("app.dbfree.desktop", "master-key")
    }
}

impl KeyProvider for OsKeyring {
    fn lookup(&self) -> AppResult<KeyLookup> {
        let in_file = || -> AppResult<Option<MasterKey>> {
            match &self.file {
                Some(file) => file.load(),
                None => Ok(None),
            }
        };
        match self.os_load() {
            Ok(Some(key)) => Ok(KeyLookup::Found(key)),
            Ok(None) => match in_file()? {
                Some(key) => {
                    self.using_file.store(true, Ordering::Relaxed);
                    Ok(KeyLookup::Found(key))
                }
                None => Ok(KeyLookup::Missing),
            },
            Err(OsFailure::Corrupt(err)) => Err(err),
            Err(OsFailure::Unavailable(reason)) => match in_file()? {
                Some(key) => {
                    self.using_file.store(true, Ordering::Relaxed);
                    Ok(KeyLookup::Found(key))
                }
                None => Ok(KeyLookup::Unavailable(reason)),
            },
        }
    }

    fn create(&self) -> AppResult<MasterKey> {
        let key = MasterKey::generate();
        let stored = self.entry().and_then(|entry| entry.set_password(&encode_key(&key)));
        match (stored, &self.file) {
            (Ok(()), _) => {
                self.using_file.store(false, Ordering::Relaxed);
                Ok(key)
            }
            (Err(err), Some(file)) => {
                log::warn!("OS keychain unavailable ({err}); keeping the master key in {}", file.path.display());
                file.store(&key)?;
                self.using_file.store(true, Ordering::Relaxed);
                Ok(key)
            }
            (Err(err), None) => Err(AppError::keyring(err)),
        }
    }

    fn storage(&self) -> KeyStorage {
        if self.using_file.load(Ordering::Relaxed) {
            KeyStorage::File
        } else {
            KeyStorage::Keychain
        }
    }

    fn key_path(&self) -> Option<PathBuf> {
        self.file.as_ref().filter(|_| self.using_file.load(Ordering::Relaxed)).map(|f| f.path.clone())
    }
}

// WHAT:  Test double. Generates once, hands back the same key for the process.
#[derive(Default)]
pub struct MemoryKeyProvider {
    key: std::sync::Mutex<Option<MasterKey>>,
}

impl KeyProvider for MemoryKeyProvider {
    fn lookup(&self) -> AppResult<KeyLookup> {
        let slot = self.key.lock().map_err(|_| AppError::internal("memory key provider poisoned"))?;
        Ok(match slot.as_ref() {
            Some(key) => KeyLookup::Found(key.clone()),
            None => KeyLookup::Missing,
        })
    }

    fn create(&self) -> AppResult<MasterKey> {
        let mut slot = self.key.lock().map_err(|_| AppError::internal("memory key provider poisoned"))?;
        Ok(slot.get_or_insert_with(MasterKey::generate).clone())
    }

    fn storage(&self) -> KeyStorage {
        KeyStorage::Memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("db-free-keyring-{}", uuid::Uuid::new_v4()));
        dir.join(name)
    }

    #[test]
    fn memory_provider_is_missing_until_created_then_stable() {
        let provider = MemoryKeyProvider::default();
        assert!(matches!(provider.lookup(), Ok(KeyLookup::Missing)));
        let created = provider.create().unwrap_or_else(|e| panic!("{e}"));
        let found = match provider.lookup() {
            Ok(KeyLookup::Found(key)) => key,
            _ => panic!("expected the created key"),
        };
        assert_eq!(found.as_bytes(), created.as_bytes());
    }

    #[test]
    fn key_file_round_trips_and_refuses_to_overwrite() {
        let file = KeyFile { path: temp_path("master.key") };
        assert!(file.load().unwrap_or_else(|e| panic!("{e}")).is_none(), "no file, no key");
        let key = MasterKey::generate();
        file.store(&key).unwrap_or_else(|e| panic!("{e}"));
        let loaded = file.load().unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("key missing"));
        assert_eq!(loaded.as_bytes(), key.as_bytes());
        assert!(file.store(&MasterKey::generate()).is_err(), "an existing key is never replaced");
        let again = file.load().unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("key missing"));
        assert_eq!(again.as_bytes(), key.as_bytes());
        let _ = std::fs::remove_dir_all(file.path.parent().unwrap_or(Path::new("/nonexistent")));
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_owner_only_and_a_loose_mode_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let file = KeyFile { path: temp_path("master.key") };
        file.store(&MasterKey::generate()).unwrap_or_else(|e| panic!("{e}"));
        let mode = |p: &Path| std::fs::metadata(p).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
        assert_eq!(mode(&file.path), 0o600);
        std::fs::set_permissions(&file.path, std::fs::Permissions::from_mode(0o644)).unwrap_or_else(|e| panic!("{e}"));
        let _ = file.load().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(mode(&file.path), 0o600, "loading tightens a world-readable key file");
        let _ = std::fs::remove_dir_all(file.path.parent().unwrap_or(Path::new("/nonexistent")));
    }

    #[test]
    fn a_corrupt_key_file_is_an_error_not_a_missing_key() {
        let path = temp_path("master.key");
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/nonexistent"))).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&path, "not base64 !!").unwrap_or_else(|e| panic!("{e}"));
        let file = KeyFile { path };
        assert!(file.load().is_err(), "silently replacing it would orphan every saved password");
        std::fs::write(&file.path, encode_key(&MasterKey::generate()).get(..10).unwrap_or("x")).unwrap_or_else(|e| panic!("{e}"));
        assert!(file.load().is_err(), "a truncated key is wrong, not absent");
        let _ = std::fs::remove_dir_all(file.path.parent().unwrap_or(Path::new("/nonexistent")));
    }
}

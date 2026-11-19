// Trop/LastFM/LastFMTokenStore.swift. The session key lives in the keyring,
// the user name in `LastFMSettings`.
//
// Not ported: the migration of a session key kept in `UserDefaults` and the
// keychain `username` and `subscriber` items. The desktop app never wrote them.

use std::path::PathBuf;
use std::sync::Mutex;

use swiftytube::auth::secure_store::{SecureStore, SecureStoreError};

use super::last_fm_defaults::LastFMSettings;

pub struct LastFMTokenStore {
    store: SecureStore,
    defaults_path: PathBuf,
    /// `Some("")` caches a missing value, so a logged-out check skips the keyring.
    cached_session_key: Mutex<Option<String>>,
}

impl LastFMTokenStore {
    pub const SERVICE: &str = "com.trop.lastfm.token";
    const SESSION_KEY_ACCOUNT: &str = "sessionKey";

    /// `defaults_path` is the `LastFMSettings` file that keeps the user name.
    pub fn new(defaults_path: PathBuf) -> Self {
        Self {
            store: SecureStore::new(Self::SERVICE),
            defaults_path,
            cached_session_key: Mutex::new(None),
        }
    }

    /// Credentials are read only from memory or the keyring.
    pub fn retrieve_session_key(&self) -> Option<String> {
        let mut cached = self.cached_session_key.lock().unwrap();
        if let Some(key) = cached.as_ref() {
            return (!key.is_empty()).then(|| key.clone());
        }
        let value = self
            .store
            .load(Self::SESSION_KEY_ACCOUNT)
            .ok()
            .and_then(|data| String::from_utf8(data).ok());
        *cached = Some(value.clone().unwrap_or_default());
        value
    }

    pub fn retrieve_username(&self) -> Option<String> {
        LastFMSettings::load(&self.defaults_path)
            .username
            .filter(|name| !name.is_empty())
    }

    pub fn store(&self, session_key: &str, username: &str) -> Result<(), SecureStoreError> {
        let mut cached = self.cached_session_key.lock().unwrap();
        self.store
            .save(session_key.as_bytes(), Self::SESSION_KEY_ACCOUNT)?;
        *cached = Some(session_key.to_string());
        self.set_username(Some(username.to_string()));
        Ok(())
    }

    pub fn clear(&self) {
        let mut cached = self.cached_session_key.lock().unwrap();
        if let Err(error) = self.store.delete(Self::SESSION_KEY_ACCOUNT) {
            log::error!("Could not delete the Last.fm session: {error}");
        }
        // Logged out now, even if the keyring kept the key: no second read.
        *cached = Some(String::new());
        self.set_username(None);
    }

    fn set_username(&self, username: Option<String>) {
        let mut settings = LastFMSettings::load(&self.defaults_path);
        settings.username = username;
        if let Err(error) = settings.save(&self.defaults_path) {
            log::error!("Could not save the Last.fm user name: {error}");
        }
    }
}

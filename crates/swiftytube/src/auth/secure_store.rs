// Replaces the Keychain-backed SecureStore.swift with the `keyring` crate:
// Secret Service on Linux, Credential Manager on Windows.

use std::collections::BTreeMap;

use keyring_core::{Entry, Error};

use super::auth_state::AuthState;
use super::auth_state_provider::AuthStateProvider;

/// Generic secret store. Fallible; callers map errors.
/// Entries use keyring-core's default store. Tests install `keyring_core::mock::Store` first;
/// otherwise the first call installs the platform store through `keyring`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecureStore {
    pub service: String,
}

impl SecureStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn save(&self, data: &[u8], key: &str) -> Result<(), SecureStoreError> {
        self.entry(key)?
            .set_secret(data)
            .map_err(SecureStoreError::from)
    }

    pub fn load(&self, key: &str) -> Result<Vec<u8>, SecureStoreError> {
        self.entry(key)?
            .get_secret()
            .map_err(SecureStoreError::from)
    }

    pub fn delete(&self, key: &str) -> Result<(), SecureStoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Swift scopes items by `kSecAttrService` + `kSecAttrAccount`; keyring uses service + user.
    fn entry(&self, key: &str) -> Result<Entry, SecureStoreError> {
        if keyring_core::get_default_store().is_none() {
            // Installs the platform store as keyring-core's default; failures surface on `Entry::new`.
            let _ = keyring::Entry::store_status();
        }
        Ok(Entry::new(&self.service, key)?)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecureStoreError {
    #[error("secure store item not found")]
    NotFound,
    /// Swift `unhandled(OSStatus)`.
    #[error("secure store error: {0}")]
    Unhandled(Error),
}

impl From<Error> for SecureStoreError {
    fn from(error: Error) -> Self {
        match error {
            Error::NoEntry => SecureStoreError::NotFound,
            other => SecureStoreError::Unhandled(other),
        }
    }
}

/// Cookie/auth state store: one JSON `AuthState` in `SecureStore`.
/// Feeds `InnerTubeClient::load_state`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecureAuthStore {
    pub store: SecureStore,
}

impl SecureAuthStore {
    pub const KEY: &str = "authState";

    pub fn new(store: SecureStore) -> Self {
        Self { store }
    }

    pub fn save(&self, auth: &AuthState) -> Result<(), SecureStoreError> {
        let json = serde_json::to_vec(auth).expect("AuthState always serializes");
        self.store.save(&json, Self::KEY)
    }

    /// Guest state when nothing is stored or the stored value is unreadable.
    pub fn load(&self) -> Result<AuthState, SecureStoreError> {
        match self.store.load(Self::KEY) {
            Ok(data) => Ok(serde_json::from_slice(&data).unwrap_or_default()),
            Err(SecureStoreError::NotFound) => Ok(AuthState::guest()),
            Err(error) => Err(error),
        }
    }

    pub fn clear(&self) -> Result<(), SecureStoreError> {
        self.store.delete(Self::KEY)
    }

    /// `AuthStateProvider` reads fall back to guest when the keyring fails.
    fn load_or_guest(&self) -> AuthState {
        self.load().unwrap_or_default()
    }
}

impl AuthStateProvider for SecureAuthStore {
    async fn cookies(&self) -> BTreeMap<String, String> {
        self.load_or_guest().cookies
    }

    async fn sapisid(&self) -> Option<String> {
        self.load_or_guest().sapisid
    }

    async fn visitor_data(&self) -> Option<String> {
        self.load_or_guest().visitor_data
    }

    async fn data_sync_id(&self) -> Option<String> {
        self.load_or_guest().data_sync_id
    }
}

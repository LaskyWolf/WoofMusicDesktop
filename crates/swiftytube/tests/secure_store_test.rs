//! Port of the Dart `test/swiftytube/secure_store_test.dart`. Uses keyring-core's
//! in-memory mock store, so no system keyring is touched.

use std::collections::BTreeMap;

use swiftytube::auth::auth_state::AuthState;
use swiftytube::auth::auth_state_provider::AuthStateProvider;
use swiftytube::auth::secure_store::{SecureAuthStore, SecureStore};

#[tokio::test]
async fn secure_auth_store_round_trips_auth_state_and_defaults_to_guest() {
    keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
    let store = SecureAuthStore::new(SecureStore::new("test"));
    assert!(store.load().unwrap().cookies.is_empty());

    store
        .save(&AuthState {
            cookies: BTreeMap::from([("SID".to_string(), "a".to_string())]),
            sapisid: Some("S".into()),
            visitor_data: Some("V".into()),
            data_sync_id: None,
        })
        .unwrap();
    assert_eq!(
        store.cookies().await,
        BTreeMap::from([("SID".to_string(), "a".to_string())])
    );
    assert_eq!(store.sapisid().await.as_deref(), Some("S"));
    assert_eq!(store.visitor_data().await.as_deref(), Some("V"));
    assert_eq!(store.data_sync_id().await, None);

    store.clear().unwrap();
    assert_eq!(store.sapisid().await, None);
}

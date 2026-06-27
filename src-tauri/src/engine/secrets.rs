//! Thin wrapper around the macOS Keychain (Security.framework) via the
//! `keyring` crate. API keys for LLM providers are stored here; the database
//! stores only a masked key + metadata. The real key NEVER appears in DB
//! columns, logs, error messages, or IPC responses.
//!
//! The `account` for every provider Keychain entry is the provider NAME
//! ("anthropic"/"openai"/"local"/"custom") — unique because the provider table
//! holds one row per name, and it is the same identifier the chat runtime reads
//! by (`runtime::provider::from_config` → `get_key(provider_id)`). The account
//! is never the raw secret or any derivation of it.

use thiserror::Error;

const SERVICE: &str = "com.conclave.app";

/// Errors from Keychain operations.
///
/// The underlying `keyring::Error` message is forwarded as a string. The
/// error text never includes any secret value — `keyring` itself does not
/// embed secrets in its error variants.
#[derive(Debug, Error)]
pub enum SecretError {
    #[error("keychain error: {0}")]
    Keychain(String),
}

impl From<keyring::Error> for SecretError {
    fn from(e: keyring::Error) -> Self {
        SecretError::Keychain(e.to_string())
    }
}

/// Store `secret` under `account` (the provider name, e.g. `"anthropic"`) in
/// the macOS Keychain. Overwrites any existing value for this account.
pub fn set_key(account: &str, secret: &str) -> Result<(), SecretError> {
    keyring::Entry::new(SERVICE, account)?.set_password(secret)?;
    Ok(())
}

/// Retrieve the secret for `account`. Returns `Ok(None)` when no entry
/// exists in the Keychain.
pub fn get_key(account: &str) -> Result<Option<String>, SecretError> {
    match keyring::Entry::new(SERVICE, account)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Delete the Keychain entry for `account`. Missing entries are treated as
/// success (idempotent) so callers can clean up unconditionally.
///
/// Called by a future "remove provider" flow; present here for completeness
/// of the Keychain CRUD surface.
#[allow(dead_code)]
pub fn delete_key(account: &str) -> Result<(), SecretError> {
    match keyring::Entry::new(SERVICE, account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full set→get→delete→get roundtrip against the real macOS Keychain.
    ///
    /// Marked `#[ignore]` because `keyring::mock` v3.6.3 stores credentials
    /// **per `Entry` object** ("There is no persistence other than in the entry
    /// itself"), so mock state does not survive across independent `Entry::new()`
    /// calls. Our `set_key` / `get_key` / `delete_key` each create their own
    /// `Entry::new()`, making a mock roundtrip impossible with this version.
    ///
    /// Normal `cargo test --lib` NEVER runs this test (and thus never touches
    /// the Keychain). Run `cargo test --lib -- --ignored` explicitly on a
    /// developer machine to exercise the real Keychain path.
    #[ignore]
    #[test]
    fn roundtrip_real_keychain() {
        let account = "com.conclave.app.test.roundtrip";

        // Best-effort cleanup of any stale entry from a previous run.
        let _ = delete_key(account);

        // Initially absent.
        assert_eq!(get_key(account).unwrap(), None);

        // Set, then retrieve.
        set_key(account, "super-secret-key").unwrap();
        assert_eq!(
            get_key(account).unwrap(),
            Some("super-secret-key".to_owned())
        );

        // Delete, then absent again.
        delete_key(account).unwrap();
        assert_eq!(get_key(account).unwrap(), None);

        // Second delete is idempotent.
        delete_key(account).unwrap();
    }
}

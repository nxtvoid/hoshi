//! Credentials hoshi keeps for the user, in the system's own store (Windows Credential
//! Manager, the macOS Keychain or the Secret Service), never in the database.

use keyring::Entry;

const SERVICE: &str = "dev.hoshi.app";

/// What a secret is for. Each one is a single entry in the system store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKey {
    /// `email\ntoken` of a Bitbucket Cloud API token.
    Bitbucket,
}

impl SecretKey {
    fn account(self) -> &'static str {
        match self {
            Self::Bitbucket => "forge:bitbucket.org",
        }
    }
}

fn entry(key: SecretKey) -> Option<Entry> {
    Entry::new(SERVICE, key.account()).ok()
}

/// `None` when nothing is stored or the store can't be read.
pub fn read(key: SecretKey) -> Option<String> {
    entry(key)?.get_password().ok()
}

pub fn write(key: SecretKey, secret: &str) -> Result<(), String> {
    entry(key)
        .ok_or_else(|| "the system credential store is unavailable".to_owned())?
        .set_password(secret)
        .map_err(|error| error.to_string())
}

pub fn delete(key: SecretKey) -> Result<(), String> {
    let Some(entry) = entry(key) else {
        return Ok(());
    };
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

//! Credentials hoshi keeps for the user, in the system's own store (Windows Credential
//! Manager, the macOS Keychain or the Secret Service), never in the database.

use keyring::Entry;

const SERVICE: &str = "dev.hoshi.app";

/// What a secret is for. Each one is a single entry in the system store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKey<'a> {
    /// `email\ntoken` of a Bitbucket Cloud API token.
    Bitbucket,
    /// A value of one of the vault's secrets, by the key it is kept under.
    Vault(&'a str),
}

impl SecretKey<'_> {
    fn account(self) -> String {
        match self {
            Self::Bitbucket => "forge:bitbucket.org".to_owned(),
            Self::Vault(id) => format!("vault:{id}"),
        }
    }
}

fn entry(key: SecretKey) -> Option<Entry> {
    Entry::new(SERVICE, &key.account()).ok()
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

/// Where the values of the vault's secrets are kept, each under a key of its own.
pub(crate) trait SecretValues: Send + Sync {
    /// `None` when nothing is stored or the store can't be read.
    fn read(&self, id: &str) -> Option<String>;
    fn write(&self, id: &str, value: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// The system's credential store.
pub(crate) struct Keychain;

impl SecretValues for Keychain {
    fn read(&self, id: &str) -> Option<String> {
        read(SecretKey::Vault(id))
    }

    fn write(&self, id: &str, value: &str) -> Result<(), String> {
        write(SecretKey::Vault(id), value)
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        delete(SecretKey::Vault(id))
    }
}

/// Values kept in memory, so tests never touch the system's store.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemoryValues {
    values: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Makes every write fail, as a locked or missing store does.
    pub refuses: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl SecretValues for MemoryValues {
    fn read(&self, id: &str) -> Option<String> {
        self.values.lock().unwrap().get(id).cloned()
    }

    fn write(&self, id: &str, value: &str) -> Result<(), String> {
        if self.refuses.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("the store is locked".into());
        }
        self.values
            .lock()
            .unwrap()
            .insert(id.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.values.lock().unwrap().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes to the system's credential store"]
    fn the_system_store_keeps_a_vault_value_until_it_is_deleted() {
        let id = format!("selftest-{}", std::process::id());
        let value = "p4ss wörd/with=symbols+and-ünicode";

        Keychain.write(&id, value).unwrap();
        let read = Keychain.read(&id);
        Keychain.delete(&id).unwrap();

        assert_eq!(read.as_deref(), Some(value));
        assert_eq!(Keychain.read(&id), None);
        assert_eq!(Keychain.delete(&id), Ok(()));
    }
}

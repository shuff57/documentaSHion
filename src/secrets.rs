//! API keys in the desktop keychain (Secret Service: KWallet or GNOME Keyring). Keys are never
//! written to settings.json. If no keychain is running, keys stay in memory for the session.
use keyring::{Entry, Error};

const SERVICE: &str = "documentaSHion";

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(describe)
}

fn describe(e: Error) -> String {
    match e {
        Error::NoStorageAccess(_) | Error::PlatformFailure(_) => format!(
            "The system keychain isn't available ({e}). Unlock KWallet or GNOME Keyring, or keep the key for this session only."
        ),
        e => format!("Keychain error: {e}"),
    }
}

/// The saved key for an account, if there is one.
pub fn load(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(describe(e)),
    }
}

pub fn store(account: &str, key: &str) -> Result<(), String> {
    entry(account)?.set_password(key).map_err(describe)
}

pub fn forget(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(describe(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_keys_are_not_errors() {
        // The mock store starts empty for every entry and never touches the real keychain.
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        assert_eq!(load("OpenRouter"), Ok(None));
        assert_eq!(forget("OpenRouter"), Ok(()));
        let e = Entry::new(SERVICE, "OpenRouter").unwrap();
        e.set_password("sk-test").unwrap();
        assert_eq!(e.get_password().unwrap(), "sk-test");
    }
    /// Stores, reads and deletes a throwaway entry in the desktop keychain. Run it alone, in a
    /// desktop session: `cargo test live_keychain -- --ignored`.
    #[test]
    #[ignore]
    fn live_keychain_round_trip() {
        let account = "self-test (safe to delete)";
        store(account, "not-a-real-key").unwrap();
        assert_eq!(load(account).unwrap().as_deref(), Some("not-a-real-key"));
        forget(account).unwrap();
        assert_eq!(load(account).unwrap(), None);
    }
}

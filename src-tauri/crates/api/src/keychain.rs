use keyring::Entry;
use crate::error::ApiError;

const SERVICE_NAME: &str = "rustyagent";

/// Thin wrapper around the OS keychain for securely storing API keys.
pub struct ApiKeyStore;

impl ApiKeyStore {
    /// Store an API key for the given provider (e.g. "anthropic", "openrouter").
    pub fn set(provider: &str, key: &str) -> Result<(), ApiError> {
        let entry = Entry::new(SERVICE_NAME, provider)
            .map_err(|e| ApiError::Keychain(e.to_string()))?;
        entry.set_password(key)
            .map_err(|e| ApiError::Keychain(e.to_string()))
    }

    /// Retrieve the stored API key for the given provider.
    pub fn get(provider: &str) -> Result<String, ApiError> {
        let entry = Entry::new(SERVICE_NAME, provider)
            .map_err(|e| ApiError::Keychain(e.to_string()))?;
        entry.get_password()
            .map_err(|e| ApiError::Keychain(e.to_string()))
    }

    /// Delete the stored API key for the given provider.
    pub fn delete(provider: &str) -> Result<(), ApiError> {
        let entry = Entry::new(SERVICE_NAME, provider)
            .map_err(|e| ApiError::Keychain(e.to_string()))?;
        entry.delete_credential()
            .map_err(|e| ApiError::Keychain(e.to_string()))
    }

    /// Returns true if a key is stored for this provider.
    pub fn exists(provider: &str) -> bool {
        Self::get(provider).is_ok()
    }
}

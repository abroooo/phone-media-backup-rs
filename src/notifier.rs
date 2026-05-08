use reqwest::blocking::Client;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("notification request failed")]
    Request(#[from] reqwest::Error),
}

pub struct BackupNotifier {
    client: Client,
    url: String,
    token: String,
}

impl BackupNotifier {
    #[must_use]
    pub fn new(url: &str, token: &str) -> Self {
        BackupNotifier {
            client: Client::new(),
            url: url.to_string(),
            token: token.to_string(),
        }
    }

    /// Send a notification to the configured URL with the given state.
    ///
    /// # Errors
    ///
    /// - network errors
    /// - invalid URL
    /// - error status code from server (e.g., 4xx or 5xx)
    pub fn notify(&self, state: &str) -> Result<(), NotifyError> {
        let payload = json!({ "state": state });

        self.client
            .post(&self.url)
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()?
            .error_for_status()?;
        Ok(())
    }
}

use reqwest::blocking::Client;
use reqwest::Error;
use serde_json::json;

pub struct BackupNotifier {
    client: Client,
    url: String,
    token: String,
}

impl BackupNotifier {
    pub fn new(url: &str, token: &str) -> Self {
        BackupNotifier {
            client: Client::new(),
            url: url.to_string(),
            token: token.to_string(),
        }
    }

    pub fn notify(&self, state: &str) -> Result<(), Error> {
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

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

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn notify_success_posts_expected_payload() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .header("authorization", "Bearer tok123")
                .header("content-type", "application/json")
                .json_body(serde_json::json!({ "state": "success" }));
            then.status(200);
        });

        let n = BackupNotifier::new(&server.url("/"), "tok123");
        assert!(n.notify("success").is_ok());
        m.assert();
    }

    #[test]
    fn notify_errors_on_server_5xx() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST);
            then.status(500);
        });

        let n = BackupNotifier::new(&server.url("/"), "tok");
        assert!(n.notify("boom").is_err());
    }

    #[test]
    fn notify_passes_state_string_through() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .json_body(serde_json::json!({ "state": "in progress" }));
            then.status(200);
        });

        let n = BackupNotifier::new(&server.url("/"), "tok");
        assert!(n.notify("in progress").is_ok());
        m.assert(); // the mock server actually received a request
    }

    #[test]
    fn notify_errors_on_server_4xx() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST);
            then.status(404);
        });

        let n = BackupNotifier::new(&server.url("/"), "tok");
        let err = n.notify("success").unwrap_err();
        assert!(matches!(err, NotifyError::Request(_)));
    }

    #[test]
    fn notify_errors_on_connection_failure() {
        // Reserve a port, drop the listener -> nothing accepts the POST.
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let url = format!("http://{addr}/");

        let n = BackupNotifier::new(&url, "tok");
        assert!(n.notify("success").is_err()); // exercises From<reqwest::Error>
    }

    #[test]
    fn notify_sends_exact_bearer_token() {
        let server = MockServer::start();
        // Mock only matches the correct token.
        let m = server.mock(|when, then| {
            when.method(POST)
                .header("authorization", "Bearer right-token");
            then.status(200);
        });

        let n = BackupNotifier::new(&server.url("/"), "right-token");
        assert!(n.notify("success").is_ok());
        m.assert(); // the mock server actually received a request
    }
}

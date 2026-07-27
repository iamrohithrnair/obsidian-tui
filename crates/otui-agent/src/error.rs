//! Agent errors.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// No API key was found for the configured provider.
    MissingCredentials {
        provider: &'static str,
        env_var: &'static str,
    },
    /// The provider returned a non-2xx status. The body is included because
    /// provider error messages are the only useful diagnostic a user gets.
    Http { status: u16, body: String },
    /// The network call failed before a response.
    Transport(String),
    /// A response didn't match the shape the provider is documented to return.
    Protocol(String),
    /// The model declined the request. This is a normal outcome, not a bug —
    /// safety classifiers return HTTP 200 with a refusal stop reason.
    Refused { category: Option<String> },
    /// The user cancelled the turn.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredentials { provider, env_var } => write!(
                f,
                "no API key for {provider}: set {env_var} and restart, or switch provider in the config"
            ),
            Self::Http { status, body } => {
                let detail = extract_message(body);
                write!(f, "provider returned {status}: {detail}")
            }
            Self::Transport(msg) => write!(f, "network error: {msg}"),
            Self::Protocol(msg) => write!(f, "unexpected response: {msg}"),
            Self::Refused { category } => match category {
                Some(c) => write!(f, "the model declined this request ({c})"),
                None => write!(f, "the model declined this request"),
            },
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for Error {}

/// Digs the human-readable message out of a provider error body.
///
/// Both Anthropic and OpenAI-compatible servers nest it under `error.message`;
/// anything else is shown raw so an unfamiliar server still tells the user
/// something.
fn extract_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "no details".to_string()
            } else {
                trimmed.chars().take(300).collect()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_error_surfaces_the_provider_message() {
        let err = Error::Http {
            status: 400,
            body: r#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens too large"}}"#
                .into(),
        };
        assert!(err.to_string().contains("max_tokens too large"));
    }

    #[test]
    fn http_error_falls_back_to_raw_body() {
        let err = Error::Http {
            status: 502,
            body: "<html>Bad Gateway</html>".into(),
        };
        assert!(err.to_string().contains("Bad Gateway"));
    }

    #[test]
    fn missing_credentials_names_the_env_var() {
        let err = Error::MissingCredentials {
            provider: "anthropic",
            env_var: "ANTHROPIC_API_KEY",
        };
        assert!(err.to_string().contains("ANTHROPIC_API_KEY"));
    }
}

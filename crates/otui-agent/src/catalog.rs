//! The providers on offer, and what they will answer to.
//!
//! Two protocols cover nearly everything worth talking to — Anthropic's Messages
//! API and OpenAI's chat completions — but each service that speaks the second
//! one needs a different base URL and a differently named key, and expecting
//! people to know those is expecting a lot. So each entry here is one ready-made
//! answer to "where is it, and what is the variable called", and the picker in
//! the UI is that list.
//!
//! Models are deliberately *not* listed. They are renamed and retired faster than
//! any table would survive, a local server's list is unknowable from here, and a
//! stale name fails as a confusing 404. [`models`] asks the provider instead.

use serde_json::Value;

use crate::ProviderKind;
use crate::error::{Error, Result};

/// One configurable backend.
pub struct Preset {
    /// What goes in the config file.
    pub id: &'static str,
    /// What the picker shows.
    pub label: &'static str,
    /// Which wire protocol it speaks.
    pub kind: ProviderKind,
    /// The variable its key is conventionally exported as, if it needs a key.
    pub env_var: Option<&'static str>,
    /// Where it lives. `None` means the protocol's own default.
    pub base_url: Option<&'static str>,
    /// A line of orientation for the picker: what this is, or where to get a key.
    pub note: &'static str,
}

/// Every backend the reader knows how to reach.
///
/// Ordered as most people will want them: the two hosted services almost
/// everyone means, then the local servers, then the gateways, then off.
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "anthropic",
        label: "Anthropic",
        kind: ProviderKind::Anthropic,
        env_var: Some("ANTHROPIC_API_KEY"),
        base_url: None,
        note: "Claude models · console.anthropic.com",
    },
    Preset {
        id: "openai",
        label: "OpenAI",
        kind: ProviderKind::OpenAiCompatible,
        env_var: Some("OPENAI_API_KEY"),
        base_url: None,
        note: "GPT models · platform.openai.com",
    },
    Preset {
        id: "ollama",
        label: "Ollama",
        kind: ProviderKind::OpenAiCompatible,
        env_var: None,
        base_url: Some("http://localhost:11434/v1"),
        note: "runs on your machine · no key needed",
    },
    Preset {
        id: "lmstudio",
        label: "LM Studio",
        kind: ProviderKind::OpenAiCompatible,
        env_var: None,
        base_url: Some("http://localhost:1234/v1"),
        note: "runs on your machine · no key needed",
    },
    Preset {
        id: "openrouter",
        label: "OpenRouter",
        kind: ProviderKind::OpenAiCompatible,
        env_var: Some("OPENROUTER_API_KEY"),
        base_url: Some("https://openrouter.ai/api/v1"),
        note: "one key, many models · openrouter.ai",
    },
    Preset {
        id: "groq",
        label: "Groq",
        kind: ProviderKind::OpenAiCompatible,
        env_var: Some("GROQ_API_KEY"),
        base_url: Some("https://api.groq.com/openai/v1"),
        note: "open models, very fast · console.groq.com",
    },
    Preset {
        id: "custom",
        label: "Custom endpoint",
        kind: ProviderKind::OpenAiCompatible,
        env_var: Some("OPENAI_API_KEY"),
        base_url: None,
        note: "any OpenAI-compatible server · set base_url yourself",
    },
    Preset {
        id: "offline",
        label: "Offline",
        kind: ProviderKind::Offline,
        env_var: None,
        base_url: None,
        note: "no model; the panel just explains how to set one up",
    },
];

/// The preset a provider name refers to.
///
/// Aliases are accepted so the names people already type — `claude`, `local`,
/// `none` — land somewhere sensible rather than being rejected.
#[must_use]
pub fn find(name: &str) -> Option<&'static Preset> {
    let name = name.trim().to_lowercase();
    let canonical = match name.as_str() {
        "claude" => "anthropic",
        "local" => "ollama",
        "lm-studio" | "lm studio" => "lmstudio",
        "openai-compatible" => "custom",
        "none" | "mock" => "offline",
        other => other,
    };
    PRESETS.iter().find(|preset| preset.id == canonical)
}

/// The variable a provider's key is read from, if it uses one.
#[must_use]
pub fn env_var(provider: &str) -> Option<&'static str> {
    find(provider).and_then(|preset| preset.env_var)
}

/// Whether this provider needs a key at all.
///
/// A local server does not, and telling someone their Ollama is unconfigured
/// because they have no API key would be nonsense.
#[must_use]
pub fn needs_key(provider: &str) -> bool {
    find(provider).is_some_and(|preset| preset.env_var.is_some())
}

/// The models a provider will accept, straight from the provider.
///
/// Both APIs answer `GET …/models` with `{"data": [{"id": …}]}`, which is the one
/// piece of luck in this area. Names come back in whatever order the service
/// chose; they are sorted here so the picker is predictable.
pub fn models(
    preset: &Preset,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<String>> {
    let url = models_url(preset, base_url);
    let key = api_key.unwrap_or_default();

    let headers: Vec<(&str, &str)> = match preset.kind {
        ProviderKind::Anthropic => vec![
            ("x-api-key", key),
            ("anthropic-version", "2023-06-01"),
            ("accept", "application/json"),
        ],
        // A local server is happy to be sent an empty bearer token, so there is
        // no need to special-case the keyless ones.
        ProviderKind::OpenAiCompatible => vec![("accept", "application/json")],
        ProviderKind::Offline => {
            return Err(Error::Protocol("the offline provider has no models".into()));
        }
    };

    let body =
        crate::provider::get_json(&url, &headers, api_key.filter(|key| !key.trim().is_empty()))?;
    let mut names: Vec<String> = body
        .get("data")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    names.sort_unstable();
    names.dedup();

    if names.is_empty() {
        return Err(Error::Protocol(format!(
            "{} listed no models",
            preset.label
        )));
    }
    Ok(names)
}

/// Where to ask for the model list.
///
/// Anthropic's configured URL is the full `/v1/messages` endpoint rather than a
/// base, so the last segment is swapped rather than appended to.
fn models_url(preset: &Preset, base_url: Option<&str>) -> String {
    let configured = base_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .or(preset.base_url);

    match preset.kind {
        ProviderKind::Anthropic => match configured {
            Some(url) => match url.trim_end_matches('/').rsplit_once('/') {
                Some((prefix, "messages")) => format!("{prefix}/models"),
                _ => format!("{}/models", url.trim_end_matches('/')),
            },
            None => "https://api.anthropic.com/v1/models".to_string(),
        },
        _ => format!(
            "{}/models",
            configured
                .unwrap_or(crate::provider::openai::DEFAULT_BASE_URL)
                .trim_end_matches('/')
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_reachable_by_its_own_id() {
        for preset in PRESETS {
            let found = find(preset.id).expect(preset.id);
            assert_eq!(found.id, preset.id);
            assert!(!preset.note.is_empty(), "{} needs a note", preset.id);
            assert_eq!(
                ProviderKind::parse(preset.id).as_ref(),
                Some(&found.kind),
                "{} must parse to the kind it declares, or it will be built wrong",
                preset.id
            );
        }
    }

    #[test]
    fn the_names_people_already_type_land_somewhere_sensible() {
        assert_eq!(find("claude").map(|p| p.id), Some("anthropic"));
        assert_eq!(find("  Ollama ").map(|p| p.id), Some("ollama"));
        assert_eq!(find("LM Studio").map(|p| p.id), Some("lmstudio"));
        assert_eq!(find("none").map(|p| p.id), Some("offline"));
        assert!(
            find("gemini").is_none(),
            "and an unknown one is honestly unknown"
        );
    }

    #[test]
    fn only_the_hosted_services_ask_for_a_key() {
        assert!(needs_key("anthropic"));
        assert!(needs_key("groq"));
        assert!(
            !needs_key("ollama"),
            "a local server has no account to bill"
        );
        assert!(!needs_key("lmstudio"));
        assert!(!needs_key("offline"));
        assert_eq!(env_var("openrouter"), Some("OPENROUTER_API_KEY"));
    }

    #[test]
    fn the_model_list_is_asked_for_beside_the_endpoint_that_answers_chats() {
        let anthropic = find("anthropic").expect("preset");
        assert_eq!(
            models_url(anthropic, None),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            models_url(anthropic, Some("https://gateway.internal/v1/messages")),
            "https://gateway.internal/v1/models",
            "a proxied endpoint keeps its own prefix"
        );

        let ollama = find("ollama").expect("preset");
        assert_eq!(
            models_url(ollama, None),
            "http://localhost:11434/v1/models",
            "a preset's own address is used when nothing is configured"
        );
        assert_eq!(
            models_url(ollama, Some("http://box:11434/v1/")),
            "http://box:11434/v1/models",
            "and a trailing slash doesn't double up"
        );

        assert_eq!(
            models_url(find("custom").expect("preset"), Some("   ")),
            format!("{}/models", crate::provider::openai::DEFAULT_BASE_URL),
            "a blank base URL is not a base URL"
        );
    }

    #[test]
    fn the_offline_provider_has_nothing_to_list() {
        let offline = find("offline").expect("preset");
        assert!(models(offline, None, None).is_err());
    }
}

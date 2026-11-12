//! Provider-agnostic chat completions with optional vision input.
//!
//! API keys are read from the environment and are never persisted or included
//! in `Debug` output. Requests are retried on transient failures, and provider
//! error bodies are redacted before they reach logs or callers.

#![forbid(unsafe_code)]

mod error;

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use error::{LlmError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
    OpenAiCompatible,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" | "open-ai" => Some(Self::OpenAi),
            "anthropic" | "claude" => Some(Self::Anthropic),
            "gemini" | "google" => Some(Self::Gemini),
            "ollama" | "local" => Some(Self::Ollama),
            "openai-compatible" | "openrouter" | "compatible" => Some(Self::OpenAiCompatible),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "openai-compatible",
        }
    }

    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Anthropic => Some("https://api.anthropic.com/v1"),
            Self::Gemini => Some("https://generativelanguage.googleapis.com/v1beta"),
            Self::Ollama => Some("http://localhost:11434"),
            Self::OpenAiCompatible => None,
        }
    }

    pub fn default_model(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("gpt-4o-mini"),
            Self::Anthropic => Some("claude-sonnet-4-5"),
            Self::Gemini => Some("gemini-2.5-flash"),
            Self::Ollama | Self::OpenAiCompatible => None,
        }
    }

    pub fn supports_vision(self) -> bool {
        matches!(self, Self::OpenAi | Self::Anthropic | Self::Gemini)
    }
}

#[derive(Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub timeout: Duration,
    pub max_output_tokens: u32,
    pub temperature: f32,
    pub vision: bool,
    pub json_mode: bool,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("kind", &self.kind)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "••••••"))
            .field("timeout", &self.timeout)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("temperature", &self.temperature)
            .field("vision", &self.vision)
            .field("json_mode", &self.json_mode)
            .finish()
    }
}

impl ProviderConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_env_with(None, None, None)
    }

    /// Precedence: explicit arguments, then `DOONCE_LLM_*`, then `DOONCE_AI_*`,
    /// then provider-specific API key variables.
    pub fn from_env_with(
        provider: Option<&str>,
        model: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<Self> {
        let provider_name = provider
            .map(str::to_string)
            .or_else(|| env("DOONCE_LLM_PROVIDER"))
            .or_else(|| env("DOONCE_AI_PROVIDER"))
            .ok_or_else(|| {
                LlmError::NotConfigured(
                    "set DOONCE_LLM_PROVIDER (openai, anthropic, gemini, ollama, openai-compatible)"
                        .into(),
                )
            })?;
        let kind = ProviderKind::parse(&provider_name).ok_or_else(|| {
            LlmError::NotConfigured(format!("unknown provider \"{provider_name}\""))
        })?;

        let model = model
            .map(str::to_string)
            .or_else(|| env("DOONCE_LLM_MODEL"))
            .or_else(|| env("DOONCE_AI_MODEL"))
            .or_else(|| kind.default_model().map(str::to_string))
            .ok_or_else(|| LlmError::NotConfigured("set DOONCE_LLM_MODEL".into()))?;

        let base_url = base_url
            .map(str::to_string)
            .or_else(|| env("DOONCE_LLM_BASE_URL"))
            .or_else(|| env("DOONCE_AI_BASE_URL"))
            .or_else(|| {
                if provider_name.eq_ignore_ascii_case("openrouter") {
                    Some("https://openrouter.ai/api/v1".to_string())
                } else {
                    kind.default_base_url().map(str::to_string)
                }
            });

        let api_key = env("DOONCE_LLM_API_KEY")
            .or_else(|| env("DOONCE_AI_API_KEY"))
            .or_else(|| match kind {
                ProviderKind::OpenAi => env("OPENAI_API_KEY"),
                ProviderKind::Anthropic => env("ANTHROPIC_API_KEY"),
                ProviderKind::Gemini => env("GEMINI_API_KEY").or_else(|| env("GOOGLE_API_KEY")),
                ProviderKind::Ollama | ProviderKind::OpenAiCompatible => None,
            });

        if api_key.is_none()
            && !matches!(kind, ProviderKind::Ollama | ProviderKind::OpenAiCompatible)
        {
            return Err(LlmError::NotConfigured(
                "set DOONCE_LLM_API_KEY (or the provider's API key variable)".into(),
            ));
        }

        let vision = env("DOONCE_LLM_VISION")
            .and_then(|value| parse_bool(&value))
            .unwrap_or_else(|| kind.supports_vision());
        let json_mode = env("DOONCE_LLM_JSON_MODE")
            .and_then(|value| parse_bool(&value))
            .unwrap_or(true);
        let timeout = env("DOONCE_LLM_TIMEOUT_SECS")
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(120));
        let max_output_tokens = env("DOONCE_LLM_MAX_TOKENS")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(4096);
        let temperature = env("DOONCE_LLM_TEMPERATURE")
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0);

        Ok(ProviderConfig {
            kind,
            model,
            base_url,
            api_key,
            timeout,
            max_output_tokens,
            temperature,
            vision,
            json_mode,
        })
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ContentPart {
    Text(String),
    ImagePng(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

impl Message {
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Message {
            role,
            content: vec![ContentPart::Text(text.into())],
        }
    }

    pub fn user_with_image(text: impl Into<String>, png: Vec<u8>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentPart::Text(text.into()), ContentPart::ImagePng(png)],
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    /// Enables the provider's JSON mode when possible. The schema itself is
    /// normally included in the system prompt by the caller.
    pub response_schema: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub text: String,
    pub usage: Option<Usage>,
}

pub trait LlmProvider: Send + Sync {
    fn complete<'a>(&'a self, request: &'a CompletionRequest) -> BoxFuture<'a, Result<Completion>>;
}

pub struct Provider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl Provider {
    pub fn new(config: ProviderConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| LlmError::Request(e.to_string()))?;
        Ok(Provider { client, config })
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }
}

impl LlmProvider for Provider {
    fn complete<'a>(&'a self, request: &'a CompletionRequest) -> BoxFuture<'a, Result<Completion>> {
        Box::pin(async move {
            match self.config.kind {
                ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
                    openai_like(&self.client, &self.config, request).await
                }
                ProviderKind::Anthropic => anthropic(&self.client, &self.config, request).await,
                ProviderKind::Gemini => gemini(&self.client, &self.config, request).await,
                ProviderKind::Ollama => ollama(&self.client, &self.config, request).await,
            }
        })
    }
}

fn base(config: &ProviderConfig) -> Result<String> {
    config
        .base_url
        .clone()
        .or_else(|| config.kind.default_base_url().map(str::to_string))
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| LlmError::NotConfigured("set DOONCE_LLM_BASE_URL".into()))
}

fn image_data(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(png))
}

fn text_of(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.as_str()),
            ContentPart::ImagePng(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_image(message: &Message) -> bool {
    message
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::ImagePng(_)))
}

fn json_mode(config: &ProviderConfig, request: &CompletionRequest) -> bool {
    config.json_mode && request.response_schema.is_some()
}

async fn send_json(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    secret: Option<&str>,
) -> Result<Value> {
    let mut last_error = String::new();
    let payload = serde_json::to_vec(body).map_err(|error| LlmError::Request(error.to_string()))?;
    for attempt in 0..3 {
        let mut builder = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.clone());
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("content-type") {
                continue;
            }
            builder = builder.header(name, value);
        }
        match builder.send().await {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if status.is_success() {
                    return serde_json::from_str(&text).map_err(|e| {
                        LlmError::Response(format!("{e}: {}", redact(&text, secret)))
                    });
                }
                if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                    && attempt < 2
                {
                    last_error = format!("HTTP {}: {}", status.as_u16(), redact(&text, secret));
                    tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
                    continue;
                }
                return Err(LlmError::Provider {
                    status: status.as_u16(),
                    message: redact(&text, secret),
                });
            }
            Err(error) if attempt < 2 => {
                last_error = error.to_string();
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
            }
            Err(error) => return Err(LlmError::Request(redact(&error.to_string(), secret))),
        }
    }
    Err(LlmError::Request(last_error))
}

fn redact(text: &str, secret: Option<&str>) -> String {
    let mut output = text.chars().take(600).collect::<String>();
    if let Some(secret) = secret.filter(|value| value.len() >= 4) {
        output = output.replace(secret, "••••••");
    }
    output
}

async fn openai_like(
    client: &reqwest::Client,
    config: &ProviderConfig,
    request: &CompletionRequest,
) -> Result<Completion> {
    let secret = config.api_key.as_deref();
    let url = format!("{}/chat/completions", base(config)?);
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            let content = if has_image(message) {
                Value::Array(
                    message
                        .content
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text(text) => json!({ "type": "text", "text": text }),
                            ContentPart::ImagePng(png) => json!({
                                "type": "image_url",
                                "image_url": { "url": image_data(png) }
                            }),
                        })
                        .collect(),
                )
            } else {
                Value::String(text_of(message))
            };
            json!({ "role": message.role.as_str(), "content": content })
        })
        .collect();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": config.temperature,
        "max_tokens": config.max_output_tokens,
    });
    if json_mode(config, request) {
        body["response_format"] = json!({ "type": "json_object" });
    }
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(key) = &config.api_key {
        headers.push(("Authorization".to_string(), format!("Bearer {key}")));
    }
    let value = send_json(client, &url, &headers, &body, secret).await?;
    let text = value["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            value["choices"][0]["message"]["content"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
        })
        .ok_or_else(|| LlmError::Response("openai: missing choices[0].message.content".into()))?;
    let usage = Usage {
        prompt_tokens: value["usage"]["prompt_tokens"].as_u64(),
        completion_tokens: value["usage"]["completion_tokens"].as_u64(),
    };
    Ok(Completion {
        text,
        usage: Some(usage),
    })
}

async fn anthropic(
    client: &reqwest::Client,
    config: &ProviderConfig,
    request: &CompletionRequest,
) -> Result<Completion> {
    let secret = config.api_key.as_deref();
    let key = config
        .api_key
        .as_deref()
        .ok_or_else(|| LlmError::NotConfigured("anthropic requires an API key".into()))?;
    let url = format!("{}/messages", base(config)?);
    let system = request
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(text_of)
        .collect::<Vec<_>>()
        .join("\n\n");
    let messages: Vec<Value> = request
        .messages
        .iter()
        .filter(|message| message.role != Role::System)
        .map(|message| {
            let content: Vec<Value> = message
                .content
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => json!({ "type": "text", "text": text }),
                    ContentPart::ImagePng(png) => json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": STANDARD.encode(png)
                        }
                    }),
                })
                .collect();
            json!({
                "role": if message.role == Role::Assistant { "assistant" } else { "user" },
                "content": content
            })
        })
        .collect();
    let body = json!({
        "model": config.model,
        "max_tokens": config.max_output_tokens,
        "temperature": config.temperature,
        "system": system,
        "messages": messages,
    });
    let headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("x-api-key".to_string(), key.to_string()),
        ("anthropic-version".to_string(), "2023-06-01".to_string()),
    ];
    let value = send_json(client, &url, &headers, &body, secret).await?;
    let text = value["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
        .ok_or_else(|| LlmError::Response("anthropic: missing content text".into()))?;
    let usage = Usage {
        prompt_tokens: value["usage"]["input_tokens"].as_u64(),
        completion_tokens: value["usage"]["output_tokens"].as_u64(),
    };
    Ok(Completion {
        text,
        usage: Some(usage),
    })
}

async fn gemini(
    client: &reqwest::Client,
    config: &ProviderConfig,
    request: &CompletionRequest,
) -> Result<Completion> {
    let secret = config.api_key.as_deref();
    let url = format!("{}/models/{}:generateContent", base(config)?, config.model);
    let system = request
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(text_of)
        .collect::<Vec<_>>()
        .join("\n\n");
    let contents: Vec<Value> = request
        .messages
        .iter()
        .filter(|message| message.role != Role::System)
        .map(|message| {
            let parts: Vec<Value> = message
                .content
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => json!({ "text": text }),
                    ContentPart::ImagePng(png) => json!({
                        "inline_data": { "mime_type": "image/png", "data": STANDARD.encode(png) }
                    }),
                })
                .collect();
            json!({
                "role": if message.role == Role::Assistant { "model" } else { "user" },
                "parts": parts
            })
        })
        .collect();
    let mut body = json!({
        "systemInstruction": { "parts": [ { "text": system } ] },
        "contents": contents,
        "generationConfig": {
            "temperature": config.temperature,
            "maxOutputTokens": config.max_output_tokens,
        }
    });
    if json_mode(config, request) {
        body["generationConfig"]["responseMimeType"] = json!("application/json");
    }
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(key) = &config.api_key {
        headers.push(("x-goog-api-key".to_string(), key.to_string()));
    }
    let value = send_json(client, &url, &headers, &body, secret).await?;
    let text = value["candidates"][0]["content"]["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            LlmError::Response("gemini: missing candidates[0].content.parts text".into())
        })?;
    let usage = Usage {
        prompt_tokens: value["usageMetadata"]["promptTokenCount"].as_u64(),
        completion_tokens: value["usageMetadata"]["candidatesTokenCount"].as_u64(),
    };
    Ok(Completion {
        text,
        usage: Some(usage),
    })
}

async fn ollama(
    client: &reqwest::Client,
    config: &ProviderConfig,
    request: &CompletionRequest,
) -> Result<Completion> {
    let url = format!("{}/api/chat", base(config)?);
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            let images: Vec<String> = message
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::ImagePng(png) => Some(STANDARD.encode(png)),
                    ContentPart::Text(_) => None,
                })
                .collect();
            let mut value = json!({
                "role": message.role.as_str(),
                "content": text_of(message),
            });
            if !images.is_empty() {
                value["images"] = json!(images);
            }
            value
        })
        .collect();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "stream": false,
        "options": { "temperature": config.temperature, "num_predict": config.max_output_tokens }
    });
    if json_mode(config, request) {
        body["format"] = json!("json");
    }
    let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    let value = send_json(client, &url, &headers, &body, config.api_key.as_deref()).await?;
    let text = value["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| LlmError::Response("ollama: missing message.content".into()))?;
    let usage = Usage {
        prompt_tokens: value["prompt_eval_count"].as_u64(),
        completion_tokens: value["eval_count"].as_u64(),
    };
    Ok(Completion {
        text,
        usage: Some(usage),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_parsing_and_defaults() {
        assert_eq!(ProviderKind::parse("OpenAI"), Some(ProviderKind::OpenAi));
        assert_eq!(ProviderKind::parse("claude"), Some(ProviderKind::Anthropic));
        assert_eq!(
            ProviderKind::parse("openrouter"),
            Some(ProviderKind::OpenAiCompatible)
        );
        assert_eq!(ProviderKind::parse("unknown"), None);
        assert!(ProviderKind::OpenAi.supports_vision());
        assert_eq!(
            ProviderKind::Ollama.default_base_url(),
            Some("http://localhost:11434")
        );
    }

    #[test]
    fn config_debug_never_leaks_the_key() {
        let config = ProviderConfig {
            kind: ProviderKind::OpenAi,
            model: "gpt-4o-mini".into(),
            base_url: None,
            api_key: Some("super-secret-key".into()),
            timeout: Duration::from_secs(30),
            max_output_tokens: 100,
            temperature: 0.0,
            vision: true,
            json_mode: true,
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("••••••"));
        assert!(!debug.contains("super-secret-key"));
    }

    #[test]
    fn image_messages_are_detected() {
        let message = Message::user_with_image("look", vec![1, 2, 3]);
        assert!(has_image(&message));
        assert!(image_data(&[1, 2, 3]).starts_with("data:image/png;base64,"));
        assert_eq!(text_of(&message), "look");
    }
}

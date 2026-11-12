//! External credential providers: Bitwarden, 1Password, LastPass and custom HTTP.

use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct ExternalCredential {
    pub username: Option<String>,
    pub password: String,
    pub totp_secret: Option<String>,
}

#[derive(Clone)]
pub struct CommandProvider {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct HttpProvider {
    pub base_url: String,
    pub token: Option<String>,
    pub allow_private: bool,
    pub client: Client,
}

#[derive(Clone, Default)]
pub struct CredentialProviders {
    pub bitwarden: Option<CommandProvider>,
    pub onepassword: Option<CommandProvider>,
    pub lastpass: Option<CommandProvider>,
    pub http: Option<HttpProvider>,
}

impl CredentialProviders {
    pub fn from_env() -> Self {
        let bw_session = env("BW_SESSION");
        let op_token = env("OP_SERVICE_ACCOUNT_TOKEN");
        let http_url = env("DOONCE_CREDENTIAL_HTTP_URL");
        CredentialProviders {
            bitwarden: Some(CommandProvider {
                command: env("DOONCE_BITWARDEN_CLI").unwrap_or_else(|| "bw".into()),
                args: vec!["get".into(), "item".into(), "{ref}".into(), "--raw".into()],
                env: bw_session
                    .into_iter()
                    .map(|value| ("BW_SESSION".into(), value))
                    .collect(),
            }),
            onepassword: Some(CommandProvider {
                command: env("DOONCE_1PASSWORD_CLI").unwrap_or_else(|| "op".into()),
                args: vec![
                    "item".into(),
                    "get".into(),
                    "{ref}".into(),
                    "--format".into(),
                    "json".into(),
                ],
                env: op_token
                    .into_iter()
                    .map(|value| ("OP_SERVICE_ACCOUNT_TOKEN".into(), value))
                    .collect(),
            }),
            lastpass: Some(CommandProvider {
                command: env("DOONCE_LASTPASS_CLI").unwrap_or_else(|| "lpass".into()),
                args: vec!["show".into(), "--json".into(), "{ref}".into()],
                env: vec![("LPASS_DISABLE_PINENTRY".into(), "1".into())],
            }),
            http: http_url.map(|base_url| HttpProvider {
                base_url,
                token: env("DOONCE_CREDENTIAL_HTTP_TOKEN"),
                allow_private: env("DOONCE_CREDENTIAL_HTTP_ALLOW_PRIVATE").is_some_and(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes"
                    )
                }),
                client: Client::builder()
                    .timeout(Duration::from_secs(15))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap_or_default(),
            }),
        }
    }

    pub async fn resolve(
        &self,
        provider: &str,
        external_ref: &str,
    ) -> std::result::Result<ExternalCredential, String> {
        match provider {
            "bitwarden" => {
                let provider = self
                    .bitwarden
                    .as_ref()
                    .ok_or_else(|| "Bitwarden is not configured".to_string())?;
                let output = provider.run(external_ref).await?;
                parse_bitwarden_item(&output)
            }
            "1password" => {
                let provider = self
                    .onepassword
                    .as_ref()
                    .ok_or_else(|| "1Password is not configured".to_string())?;
                let output = provider.run(external_ref).await?;
                parse_onepassword_item(&output)
            }
            "lastpass" => {
                let provider = self
                    .lastpass
                    .as_ref()
                    .ok_or_else(|| "LastPass is not configured".to_string())?;
                let output = provider.run(external_ref).await?;
                parse_lastpass_item(&output)
            }
            "http" => {
                let provider = self.http.as_ref().ok_or_else(|| {
                    "custom HTTP credential service is not configured".to_string()
                })?;
                provider.resolve(external_ref).await
            }
            other => Err(format!("unknown credential provider \"{other}\"")),
        }
    }
}

impl CommandProvider {
    async fn run(&self, reference: &str) -> std::result::Result<String, String> {
        let mut command = tokio::process::Command::new(&self.command);
        command.args(self.args.iter().map(|arg| arg.replace("{ref}", reference)));
        command.envs(self.env.iter().map(|(key, value)| (key, value)));
        command.kill_on_drop(true);
        let output = match tokio::time::timeout(Duration::from_secs(20), command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return Err(format!("could not run {}: {error}", self.command)),
            Err(_) => return Err(format!("{} timed out", self.command)),
        };
        if !output.status.success() {
            // Never include stdout/stderr: credential managers may print secrets.
            return Err(format!(
                "{} failed with exit code {}",
                self.command,
                output.status.code().unwrap_or(-1)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl HttpProvider {
    async fn resolve(&self, reference: &str) -> std::result::Result<ExternalCredential, String> {
        let encoded: String = url::form_urlencoded::byte_serialize(reference.as_bytes()).collect();
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), encoded);
        if !self.allow_private {
            let parsed =
                Url::parse(&url).map_err(|error| format!("invalid credential URL: {error}"))?;
            if !doonce_core::validation::is_safe_outbound_url(parsed.as_str()) {
                return Err("credential service URL must be public https/http".into());
            }
        }
        let mut request = self.client.get(url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("credential service request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("could not read credential response: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "credential service returned HTTP {}",
                status.as_u16()
            ));
        }
        parse_http_item(&body)
    }
}

pub fn parse_bitwarden_item(json: &str) -> std::result::Result<ExternalCredential, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid Bitwarden JSON: {error}"))?;
    let login = value.get("login").and_then(Value::as_object);
    let username = login
        .and_then(|login| login.get("username"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let password = login
        .and_then(|login| login.get("password"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if password.is_empty() {
        return Err("Bitwarden item has no password".into());
    }
    let totp_secret = login
        .and_then(|login| login.get("totp"))
        .and_then(Value::as_str)
        .and_then(extract_totp_secret);
    Ok(ExternalCredential {
        username,
        password,
        totp_secret,
    })
}

pub fn parse_onepassword_item(json: &str) -> std::result::Result<ExternalCredential, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid 1Password JSON: {error}"))?;
    let fields = value
        .get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let find = |needle: &str| {
        fields.iter().find_map(|field| {
            let id = field
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let label = field
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let kind = field
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if id.contains(needle) || label.contains(needle) || kind.contains(needle) {
                field
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            }
        })
    };
    let username = find("username").or_else(|| find("email"));
    let password =
        find("password").ok_or_else(|| "1Password item has no password field".to_string())?;
    let totp_secret = find("one-time password")
        .or_else(|| find("otp"))
        .or_else(|| find("totp"))
        .and_then(|value| extract_totp_secret(&value));
    Ok(ExternalCredential {
        username,
        password,
        totp_secret,
    })
}

pub fn parse_lastpass_item(json: &str) -> std::result::Result<ExternalCredential, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid LastPass JSON: {error}"))?;
    let item = value
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&value);
    let username = item
        .get("username")
        .and_then(Value::as_str)
        .map(str::to_string);
    let password = item
        .get("password")
        .and_then(Value::as_str)
        .ok_or_else(|| "LastPass item has no password".to_string())?
        .to_string();
    let totp_secret = item
        .get("totp")
        .or_else(|| item.get("otp"))
        .and_then(Value::as_str)
        .and_then(extract_totp_secret);
    Ok(ExternalCredential {
        username,
        password,
        totp_secret,
    })
}

fn parse_http_item(json: &str) -> std::result::Result<ExternalCredential, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("invalid credential service JSON: {error}"))?;
    let username = value
        .get("username")
        .and_then(Value::as_str)
        .map(str::to_string);
    let password = value
        .get("password")
        .or_else(|| value.get("secret"))
        .and_then(Value::as_str)
        .ok_or_else(|| "credential service response has no password".to_string())?
        .to_string();
    let totp_secret = value
        .get("totp")
        .or_else(|| value.get("otp"))
        .and_then(Value::as_str)
        .and_then(extract_totp_secret);
    Ok(ExternalCredential {
        username,
        password,
        totp_secret,
    })
}

fn extract_totp_secret(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with("otpauth://") {
        let url = Url::parse(value).ok()?;
        return url
            .query_pairs()
            .find(|(key, _)| key == "secret")
            .map(|(_, value)| value.to_string());
    }
    Some(value.to_string())
}

fn env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bitwarden_item() {
        let json = r#"{"login":{"username":"ada","password":"hunter2","totp":"otpauth://totp/DoOnce:ada?secret=JBSWY3DPEHPK3PXP&issuer=DoOnce"}}"#;
        let credential = parse_bitwarden_item(json).unwrap();
        assert_eq!(credential.username.as_deref(), Some("ada"));
        assert_eq!(credential.password, "hunter2");
        assert_eq!(credential.totp_secret.as_deref(), Some("JBSWY3DPEHPK3PXP"));
    }

    #[test]
    fn parses_onepassword_item() {
        let json = r#"{"fields":[
            {"id":"username","value":"ada"},
            {"id":"password","value":"hunter2"},
            {"id":"otp","type":"OTP","value":"otpauth://totp/DoOnce:ada?secret=JBSWY3DPEHPK3PXP"}
        ]}"#;
        let credential = parse_onepassword_item(json).unwrap();
        assert_eq!(credential.username.as_deref(), Some("ada"));
        assert_eq!(credential.password, "hunter2");
        assert_eq!(credential.totp_secret.as_deref(), Some("JBSWY3DPEHPK3PXP"));
    }

    #[test]
    fn parses_lastpass_item() {
        let json = r#"[{"username":"ada","password":"hunter2","totp":"JBSWY3DPEHPK3PXP"}]"#;
        let credential = parse_lastpass_item(json).unwrap();
        assert_eq!(credential.username.as_deref(), Some("ada"));
        assert_eq!(credential.password, "hunter2");
        assert_eq!(credential.totp_secret.as_deref(), Some("JBSWY3DPEHPK3PXP"));
    }

    #[test]
    fn rejects_missing_passwords() {
        assert!(parse_bitwarden_item(r#"{"login":{"username":"ada"}}"#).is_err());
        assert!(parse_lastpass_item(r#"[{"username":"ada"}]"#).is_err());
    }
}

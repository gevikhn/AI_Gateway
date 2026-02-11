use serde::Deserialize;
use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub listen: String,
    pub gateway_auth: GatewayAuthConfig,
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub cors: Option<CorsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayAuthConfig {
    pub tokens: Vec<String>,
    #[serde(default = "default_token_sources")]
    pub token_sources: Vec<TokenSourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenSourceConfig {
    AuthorizationBearer,
    Header { name: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub prefix: String,
    pub upstream: UpstreamConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamConfig {
    pub base_url: String,
    #[serde(default = "default_true")]
    pub strip_prefix: bool,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default)]
    pub inject_headers: Vec<HeaderInjection>,
    #[serde(default)]
    pub remove_headers: Vec<String>,
    #[serde(default)]
    pub forward_xff: bool,
    #[serde(default)]
    pub proxy: Option<UpstreamProxyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeaderInjection {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamProxyConfig {
    pub protocol: ProxyProtocol,
    pub address: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Http,
    Https,
    Socks,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CorsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allow_origins: Vec<String>,
    #[serde(default)]
    pub allow_headers: Vec<String>,
    #[serde(default)]
    pub allow_methods: Vec<String>,
    #[serde(default)]
    pub expose_headers: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    MissingEnvVar(String),
    Validation(String),
}

impl AppConfig {
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::from_yaml_str(&raw)
    }

    pub fn from_yaml_str(yaml: &str) -> Result<Self, ConfigError> {
        let interpolated = interpolate_env_vars(yaml)?;
        let config: Self = serde_yaml::from_str(&interpolated).map_err(ConfigError::Yaml)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.listen.trim().is_empty() {
            return Err(ConfigError::Validation(
                "`listen` must not be empty".to_string(),
            ));
        }

        if self.gateway_auth.tokens.is_empty() {
            return Err(ConfigError::Validation(
                "`gateway_auth.tokens` must not be empty".to_string(),
            ));
        }

        if self
            .gateway_auth
            .tokens
            .iter()
            .any(|token| token.trim().is_empty())
        {
            return Err(ConfigError::Validation(
                "`gateway_auth.tokens` must not contain empty values".to_string(),
            ));
        }

        if self.routes.is_empty() {
            return Err(ConfigError::Validation(
                "`routes` must contain at least one route".to_string(),
            ));
        }

        let mut ids = HashSet::new();
        let mut prefixes = HashSet::new();

        for route in &self.routes {
            if route.id.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "route `id` must not be empty".to_string(),
                ));
            }

            if !ids.insert(route.id.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate route id `{}`",
                    route.id
                )));
            }

            if !route.prefix.starts_with('/') {
                return Err(ConfigError::Validation(format!(
                    "route `{}` prefix must start with `/`",
                    route.id
                )));
            }

            if route.prefix.len() > 1 && route.prefix.ends_with('/') {
                return Err(ConfigError::Validation(format!(
                    "route `{}` prefix must not end with `/`",
                    route.id
                )));
            }

            if !prefixes.insert(route.prefix.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate route prefix `{}`",
                    route.prefix
                )));
            }

            if route.upstream.base_url.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "route `{}` upstream.base_url must not be empty",
                    route.id
                )));
            }

            if route.upstream.connect_timeout_ms == 0 {
                return Err(ConfigError::Validation(format!(
                    "route `{}` upstream.connect_timeout_ms must be > 0",
                    route.id
                )));
            }

            if route.upstream.request_timeout_ms == 0 {
                return Err(ConfigError::Validation(format!(
                    "route `{}` upstream.request_timeout_ms must be > 0",
                    route.id
                )));
            }

            if let Some(proxy) = &route.upstream.proxy {
                if proxy.address.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` upstream.proxy.address must not be empty",
                        route.id
                    )));
                }

                match (&proxy.username, &proxy.password) {
                    (Some(username), Some(password))
                        if username.trim().is_empty() || password.trim().is_empty() =>
                    {
                        return Err(ConfigError::Validation(format!(
                            "route `{}` upstream.proxy.username/password must not be empty",
                            route.id
                        )));
                    }
                    (Some(_), Some(_)) | (None, None) => {}
                    _ => {
                        return Err(ConfigError::Validation(format!(
                            "route `{}` upstream.proxy.username and upstream.proxy.password must be set together",
                            route.id
                        )));
                    }
                }
            }

            for header in &route.upstream.inject_headers {
                if header.name.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` has empty inject_headers.name",
                        route.id
                    )));
                }
            }
        }

        Ok(())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Yaml(err) => write!(f, "yaml parse error: {err}"),
            Self::MissingEnvVar(name) => write!(f, "missing environment variable `{name}`"),
            Self::Validation(msg) => write!(f, "config validation error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn interpolate_env_vars(input: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(rel_start) = input[cursor..].find("${") {
        let start = cursor + rel_start;
        out.push_str(&input[cursor..start]);

        let key_start = start + 2;
        let rel_end = input[key_start..].find('}').ok_or_else(|| {
            ConfigError::Validation("unterminated `${...}` expression".to_string())
        })?;
        let end = key_start + rel_end;
        let key = &input[key_start..end];

        if key.is_empty() {
            return Err(ConfigError::Validation(
                "empty environment variable name in `${}`".to_string(),
            ));
        }

        let value = env::var(key).map_err(|_| ConfigError::MissingEnvVar(key.to_string()))?;
        out.push_str(&value);
        cursor = end + 1;
    }

    out.push_str(&input[cursor..]);
    Ok(out)
}

fn default_true() -> bool {
    true
}

fn default_token_sources() -> Vec<TokenSourceConfig> {
    vec![TokenSourceConfig::AuthorizationBearer]
}

fn default_connect_timeout_ms() -> u64 {
    10_000
}

fn default_request_timeout_ms() -> u64 {
    60_000
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, ProxyProtocol};

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  tokens:
    - "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        assert_eq!(config.routes.len(), 1);
        assert!(config.cors.is_none());
    }

    #[test]
    fn parse_config_with_env_interpolation() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  tokens:
    - '${PATH}'
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        assert!(!config.gateway_auth.tokens[0].is_empty());
    }

    #[test]
    fn parse_config_with_proxy() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  tokens:
    - "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      proxy:
        protocol: "socks"
        address: "127.0.0.1:1080"
        username: "proxy-user"
        password: "proxy-pass"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let proxy = config.routes[0]
            .upstream
            .proxy
            .as_ref()
            .expect("proxy should exist");
        assert_eq!(proxy.protocol, ProxyProtocol::Socks);
        assert_eq!(proxy.address, "127.0.0.1:1080");
        assert_eq!(proxy.username.as_deref(), Some("proxy-user"));
        assert_eq!(proxy.password.as_deref(), Some("proxy-pass"));
    }

    #[test]
    fn reject_proxy_with_partial_auth() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  tokens:
    - "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      proxy:
        protocol: "http"
        address: "127.0.0.1:8080"
        username: "proxy-user"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error.to_string().contains(
                "upstream.proxy.username and upstream.proxy.password must be set together"
            )
        );
    }
}

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
    pub inbound_tls: Option<InboundTlsConfig>,
    #[serde(default)]
    pub cors: Option<CorsConfig>,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    #[serde(default)]
    pub concurrency: Option<ConcurrencyConfig>,
    #[serde(default)]
    pub observability: Option<ObservabilityConfig>,
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
    #[serde(default)]
    pub upstream_key_max_inflight: Option<usize>,
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
pub struct InboundTlsConfig {
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default = "default_self_signed_cert_path")]
    pub self_signed_cert_path: String,
    #[serde(default = "default_self_signed_key_path")]
    pub self_signed_key_path: String,
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

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    pub per_minute: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcurrencyConfig {
    #[serde(default)]
    pub downstream_max_inflight: Option<usize>,
    #[serde(default)]
    pub upstream_per_key_max_inflight: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub tracing: TracingConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_observability_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: LogFormat,
    #[serde(default = "default_true")]
    pub to_stdout: bool,
    #[serde(default)]
    pub file: Option<LogFileConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[default]
    Json,
    Text,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogFileConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_log_file_dir")]
    pub dir: String,
    #[serde(default = "default_log_file_prefix")]
    pub prefix: String,
    #[serde(default = "default_log_rotation")]
    pub rotation: LogRotation,
    #[serde(default = "default_log_file_max_files")]
    pub max_files: usize,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogRotation {
    Minutely,
    Hourly,
    #[default]
    Daily,
    Never,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_metrics_path")]
    pub path: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_trace_sample_ratio")]
    pub sample_ratio: f64,
    #[serde(default)]
    pub otlp: Option<OtlpConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtlpConfig {
    pub endpoint: String,
    #[serde(default = "default_otlp_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_observability_log_level(),
            format: default_log_format(),
            to_stdout: true,
            file: None,
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_metrics_path(),
            token: String::new(),
        }
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_ratio: default_trace_sample_ratio(),
            otlp: None,
        }
    }
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
        let mut has_route_upstream_key_concurrency = false;

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

            if let Some(limit) = route.upstream.upstream_key_max_inflight {
                has_route_upstream_key_concurrency = true;
                if limit == 0 {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` upstream.upstream_key_max_inflight must be > 0 when provided",
                        route.id
                    )));
                }
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

        let mut has_global_upstream_key_concurrency = false;
        if let Some(rate_limit) = &self.rate_limit
            && rate_limit.per_minute == 0
        {
            return Err(ConfigError::Validation(
                "`rate_limit.per_minute` must be > 0".to_string(),
            ));
        }

        if let Some(concurrency) = &self.concurrency {
            if let Some(limit) = concurrency.downstream_max_inflight
                && limit == 0
            {
                return Err(ConfigError::Validation(
                    "`concurrency.downstream_max_inflight` must be > 0 when provided".to_string(),
                ));
            }

            if let Some(limit) = concurrency.upstream_per_key_max_inflight {
                has_global_upstream_key_concurrency = true;
                if limit == 0 {
                    return Err(ConfigError::Validation(
                        "`concurrency.upstream_per_key_max_inflight` must be > 0 when provided"
                            .to_string(),
                    ));
                }
            }
        }

        if has_global_upstream_key_concurrency || has_route_upstream_key_concurrency {
            for route in &self.routes {
                let route_uses_upstream_key_concurrency = has_global_upstream_key_concurrency
                    || route.upstream.upstream_key_max_inflight.is_some();
                if !route_uses_upstream_key_concurrency {
                    continue;
                }

                if !route_has_upstream_key_injection(route) {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` must configure `upstream.inject_headers` with one of {:?} when upstream key concurrency is enabled",
                        route.id,
                        upstream_key_header_names(),
                    )));
                }
            }
        }

        if let Some(tls) = &self.inbound_tls {
            validate_optional_path(
                tls.cert_path.as_deref(),
                "`inbound_tls.cert_path` must not be empty when provided",
            )?;
            validate_optional_path(
                tls.key_path.as_deref(),
                "`inbound_tls.key_path` must not be empty when provided",
            )?;

            if tls.self_signed_cert_path.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`inbound_tls.self_signed_cert_path` must not be empty".to_string(),
                ));
            }
            if tls.self_signed_key_path.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`inbound_tls.self_signed_key_path` must not be empty".to_string(),
                ));
            }

            match (&tls.cert_path, &tls.key_path) {
                (Some(_), Some(_)) | (None, None) => {}
                _ => {
                    return Err(ConfigError::Validation(
                        "`inbound_tls.cert_path` and `inbound_tls.key_path` must be set together"
                            .to_string(),
                    ));
                }
            }
        }

        if let Some(observability) = &self.observability {
            if observability.logging.level.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`observability.logging.level` must not be empty".to_string(),
                ));
            }
            if !observability.logging.to_stdout
                && !matches!(
                    observability.logging.file.as_ref(),
                    Some(file) if file.enabled
                )
            {
                return Err(ConfigError::Validation(
                    "`observability.logging` must enable at least one sink (`to_stdout` or `file.enabled`)"
                        .to_string(),
                ));
            }
            if let Some(file) = &observability.logging.file
                && file.enabled
            {
                if file.dir.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "`observability.logging.file.dir` must not be empty".to_string(),
                    ));
                }
                if file.prefix.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "`observability.logging.file.prefix` must not be empty".to_string(),
                    ));
                }
                if file.max_files == 0 {
                    return Err(ConfigError::Validation(
                        "`observability.logging.file.max_files` must be > 0".to_string(),
                    ));
                }
            }

            if !observability.metrics.path.starts_with('/') {
                return Err(ConfigError::Validation(
                    "`observability.metrics.path` must start with `/`".to_string(),
                ));
            }
            if observability.metrics.path == "/healthz" {
                return Err(ConfigError::Validation(
                    "`observability.metrics.path` must not conflict with `/healthz`".to_string(),
                ));
            }
            if observability.metrics.enabled && observability.metrics.token.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`observability.metrics.token` must not be empty when metrics are enabled"
                        .to_string(),
                ));
            }

            if !(0.0..=1.0).contains(&observability.tracing.sample_ratio) {
                return Err(ConfigError::Validation(
                    "`observability.tracing.sample_ratio` must be within [0.0, 1.0]".to_string(),
                ));
            }

            if let Some(otlp) = &observability.tracing.otlp {
                if otlp.endpoint.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "`observability.tracing.otlp.endpoint` must not be empty".to_string(),
                    ));
                }
                if reqwest::Url::parse(otlp.endpoint.trim()).is_err() {
                    return Err(ConfigError::Validation(
                        "`observability.tracing.otlp.endpoint` must be a valid URL".to_string(),
                    ));
                }
                if otlp.timeout_ms == 0 {
                    return Err(ConfigError::Validation(
                        "`observability.tracing.otlp.timeout_ms` must be > 0".to_string(),
                    ));
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

fn default_observability_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> LogFormat {
    LogFormat::Json
}

fn default_log_file_dir() -> String {
    "logs".to_string()
}

fn default_log_file_prefix() -> String {
    "ai-gw-lite".to_string()
}

fn default_log_rotation() -> LogRotation {
    LogRotation::Daily
}

fn default_log_file_max_files() -> usize {
    7
}

fn default_metrics_path() -> String {
    "/metrics".to_string()
}

fn default_trace_sample_ratio() -> f64 {
    0.05
}

fn default_otlp_timeout_ms() -> u64 {
    3_000
}

fn default_self_signed_cert_path() -> String {
    "certs/gateway-selfsigned.crt".to_string()
}

fn default_self_signed_key_path() -> String {
    "certs/gateway-selfsigned.key".to_string()
}

fn upstream_key_header_names() -> &'static [&'static str] {
    &["authorization", "x-api-key"]
}

fn validate_optional_path(path: Option<&str>, message: &str) -> Result<(), ConfigError> {
    if matches!(path, Some(value) if value.trim().is_empty()) {
        return Err(ConfigError::Validation(message.to_string()));
    }
    Ok(())
}

fn route_has_upstream_key_injection(route: &RouteConfig) -> bool {
    route.upstream.inject_headers.iter().any(|header| {
        !header.value.trim().is_empty()
            && upstream_key_header_names()
                .iter()
                .any(|name| header.name.trim().eq_ignore_ascii_case(name.trim()))
    })
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, LogFormat, LogRotation, ProxyProtocol};

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
        assert!(config.rate_limit.is_none());
        assert!(config.concurrency.is_none());
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

    #[test]
    fn parse_config_with_inbound_tls() {
        let yaml = r#"
listen: "127.0.0.1:8443"
gateway_auth:
  tokens:
    - "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
inbound_tls:
  cert_path: "./tls/server.crt"
  key_path: "./tls/server.key"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let tls = config
            .inbound_tls
            .as_ref()
            .expect("inbound tls config should exist");
        assert_eq!(tls.cert_path.as_deref(), Some("./tls/server.crt"));
        assert_eq!(tls.key_path.as_deref(), Some("./tls/server.key"));
        assert_eq!(tls.self_signed_cert_path, "certs/gateway-selfsigned.crt");
        assert_eq!(tls.self_signed_key_path, "certs/gateway-selfsigned.key");
    }

    #[test]
    fn reject_inbound_tls_partial_cert_key() {
        let yaml = r#"
listen: "127.0.0.1:8443"
gateway_auth:
  tokens:
    - "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
inbound_tls:
  cert_path: "./tls/server.crt"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error.to_string().contains(
                "`inbound_tls.cert_path` and `inbound_tls.key_path` must be set together"
            )
        );
    }

    #[test]
    fn parse_config_with_rate_limit_and_concurrency() {
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
      upstream_key_max_inflight: 3
      inject_headers:
        - name: "authorization"
          value: "Bearer upstream-key"
rate_limit:
  per_minute: 120
concurrency:
  downstream_max_inflight: 40
  upstream_per_key_max_inflight: 8
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        assert_eq!(
            config.rate_limit.as_ref().expect("rate limit").per_minute,
            120
        );
        let concurrency = config.concurrency.as_ref().expect("concurrency");
        assert_eq!(concurrency.downstream_max_inflight, Some(40));
        assert_eq!(concurrency.upstream_per_key_max_inflight, Some(8));
        assert_eq!(config.routes[0].upstream.upstream_key_max_inflight, Some(3));
    }

    #[test]
    fn parse_config_with_observability() {
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
observability:
  logging:
    level: "debug"
    format: "text"
    to_stdout: true
    file:
      enabled: true
      dir: "./logs"
      prefix: "gateway"
      rotation: "hourly"
      max_files: 12
  metrics:
    enabled: true
    path: "/metrics"
    token: "metrics_token"
  tracing:
    enabled: true
    sample_ratio: 0.1
    otlp:
      endpoint: "http://127.0.0.1:4317"
      timeout_ms: 5000
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let observability = config
            .observability
            .as_ref()
            .expect("observability should exist");
        assert_eq!(observability.logging.level, "debug");
        assert_eq!(observability.logging.format, LogFormat::Text);
        assert!(observability.logging.to_stdout);
        let log_file = observability
            .logging
            .file
            .as_ref()
            .expect("file should exist");
        assert!(log_file.enabled);
        assert_eq!(log_file.dir, "./logs");
        assert_eq!(log_file.prefix, "gateway");
        assert_eq!(log_file.rotation, LogRotation::Hourly);
        assert_eq!(log_file.max_files, 12);
        assert!(observability.metrics.enabled);
        assert_eq!(observability.metrics.path, "/metrics");
        assert_eq!(observability.metrics.token, "metrics_token");
        assert!(observability.tracing.enabled);
        assert_eq!(observability.tracing.sample_ratio, 0.1);
        let otlp = observability
            .tracing
            .otlp
            .as_ref()
            .expect("otlp should exist");
        assert_eq!(otlp.endpoint, "http://127.0.0.1:4317");
        assert_eq!(otlp.timeout_ms, 5_000);
    }

    #[test]
    fn reject_enabled_metrics_without_token() {
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
observability:
  metrics:
    enabled: true
    path: "/metrics"
    token: ""
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`observability.metrics.token` must not be empty")
        );
    }

    #[test]
    fn reject_invalid_trace_sample_ratio() {
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
observability:
  tracing:
    enabled: true
    sample_ratio: 1.2
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`observability.tracing.sample_ratio` must be within [0.0, 1.0]")
        );
    }

    #[test]
    fn reject_logging_without_any_sink() {
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
observability:
  logging:
    level: "info"
    format: "json"
    to_stdout: false
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("must enable at least one sink (`to_stdout` or `file.enabled`)")
        );
    }

    #[test]
    fn reject_log_file_with_invalid_limits() {
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
observability:
  logging:
    level: "info"
    format: "json"
    to_stdout: false
    file:
      enabled: true
      dir: "./logs"
      prefix: "gateway"
      rotation: "daily"
      max_files: 0
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`observability.logging.file.max_files` must be > 0")
        );
    }

    #[test]
    fn reject_zero_rate_limit() {
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
rate_limit:
  per_minute: 0
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`rate_limit.per_minute` must be > 0")
        );
    }

    #[test]
    fn reject_removed_upstream_key_headers_field() {
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
concurrency:
  upstream_per_key_max_inflight: 8
  upstream_key_headers:
    - "bad header"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("unknown field `upstream_key_headers`")
        );
    }

    #[test]
    fn reject_upstream_concurrency_without_injected_key_value() {
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
concurrency:
  upstream_per_key_max_inflight: 8
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("must configure `upstream.inject_headers` with one of")
        );
    }
}

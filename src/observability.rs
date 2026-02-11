use crate::config::{
    LogFileConfig, LogFormat, LogRotation, LoggingConfig, ObservabilityConfig, TracingConfig,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use http::header::AUTHORIZATION;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::runtime::Tokio;
use opentelemetry_sdk::trace::{Sampler, TracerProvider};
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

type FileLogWriter = (
    Option<tracing_appender::non_blocking::NonBlocking>,
    Option<tracing_appender::non_blocking::WorkerGuard>,
);

#[derive(Clone, Default)]
pub struct ObservabilityRuntime {
    pub metrics: Option<Arc<GatewayMetrics>>,
    metrics_path: Option<String>,
    metrics_token: Option<String>,
}

impl ObservabilityRuntime {
    pub fn from_config(config: Option<&ObservabilityConfig>) -> Self {
        let Some(config) = config else {
            return Self::default();
        };

        if config.metrics.enabled {
            return Self {
                metrics: Some(Arc::new(GatewayMetrics::new())),
                metrics_path: Some(config.metrics.path.clone()),
                metrics_token: Some(config.metrics.token.clone()),
            };
        }

        Self::default()
    }

    pub fn metrics_path(&self) -> Option<&str> {
        self.metrics_path.as_deref()
    }

    pub fn is_metrics_request_authorized(&self, headers: &HeaderMap) -> bool {
        let Some(expected_token) = self.metrics_token.as_deref() else {
            return false;
        };
        matches!(
            extract_bearer_token(headers),
            Some(token) if token == expected_token
        )
    }

    pub fn encode_metrics(&self) -> Option<String> {
        let metrics = self.metrics.as_ref()?;
        Some(metrics.encode())
    }
}

#[derive(Debug)]
pub struct GatewayMetrics {
    registry: RwLock<Registry>,
    requests_total: Family<RequestCounterLabels, Counter>,
    request_duration_seconds: Family<RequestDurationLabels, Histogram>,
    upstream_duration_seconds: Family<UpstreamDurationLabels, Histogram>,
    inflight_requests: Family<RouteLabels, Gauge>,
    sse_streams_inflight: Family<RouteLabels, Gauge>,
}

impl GatewayMetrics {
    pub fn new() -> Self {
        let requests_total = Family::<RequestCounterLabels, Counter>::default();
        let request_duration_seconds =
            Family::<RequestDurationLabels, Histogram>::new_with_constructor(|| {
                Histogram::new(exponential_buckets(0.001, 2.0, 16))
            });
        let upstream_duration_seconds =
            Family::<UpstreamDurationLabels, Histogram>::new_with_constructor(|| {
                Histogram::new(exponential_buckets(0.001, 2.0, 16))
            });
        let inflight_requests = Family::<RouteLabels, Gauge>::default();
        let sse_streams_inflight = Family::<RouteLabels, Gauge>::default();

        let mut registry = Registry::default();
        registry.register(
            "gateway_requests_total",
            "Total number of handled gateway requests.",
            requests_total.clone(),
        );
        registry.register(
            "gateway_request_duration_seconds",
            "Gateway request duration in seconds.",
            request_duration_seconds.clone(),
        );
        registry.register(
            "gateway_upstream_duration_seconds",
            "Upstream request duration in seconds.",
            upstream_duration_seconds.clone(),
        );
        registry.register(
            "gateway_inflight_requests",
            "Current number of in-flight gateway requests.",
            inflight_requests.clone(),
        );
        registry.register(
            "gateway_sse_streams_inflight",
            "Current number of in-flight SSE streams.",
            sse_streams_inflight.clone(),
        );

        Self {
            registry: RwLock::new(registry),
            requests_total,
            request_duration_seconds,
            upstream_duration_seconds,
            inflight_requests,
            sse_streams_inflight,
        }
    }

    pub fn observe_request(
        &self,
        route_id: &str,
        method: &Method,
        outcome: &str,
        status: StatusCode,
        duration: Duration,
    ) {
        let status_class = format!("{}xx", status.as_u16() / 100);
        self.requests_total
            .get_or_create(&RequestCounterLabels {
                route_id: route_id.to_string(),
                method: method.as_str().to_string(),
                outcome: outcome.to_string(),
                status_class,
            })
            .inc();

        self.request_duration_seconds
            .get_or_create(&RequestDurationLabels {
                route_id: route_id.to_string(),
                method: method.as_str().to_string(),
                outcome: outcome.to_string(),
            })
            .observe(duration.as_secs_f64());
    }

    pub fn observe_upstream_duration(
        &self,
        route_id: &str,
        upstream_host: &str,
        result: &str,
        duration: Duration,
    ) {
        self.upstream_duration_seconds
            .get_or_create(&UpstreamDurationLabels {
                route_id: route_id.to_string(),
                upstream_host: upstream_host.to_string(),
                result: result.to_string(),
            })
            .observe(duration.as_secs_f64());
    }

    pub fn inc_inflight(&self, route_id: &str) {
        self.inflight_requests
            .get_or_create(&RouteLabels {
                route_id: route_id.to_string(),
            })
            .inc();
    }

    pub fn dec_inflight(&self, route_id: &str) {
        self.inflight_requests
            .get_or_create(&RouteLabels {
                route_id: route_id.to_string(),
            })
            .dec();
    }

    pub fn inc_sse_inflight(&self, route_id: &str) {
        self.sse_streams_inflight
            .get_or_create(&RouteLabels {
                route_id: route_id.to_string(),
            })
            .inc();
    }

    pub fn dec_sse_inflight(&self, route_id: &str) {
        self.sse_streams_inflight
            .get_or_create(&RouteLabels {
                route_id: route_id.to_string(),
            })
            .dec();
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        if let Ok(registry) = self.registry.read() {
            let _ = encode(&mut output, &registry);
        }
        output
    }
}

impl Default for GatewayMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RouteLabels {
    route_id: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RequestCounterLabels {
    route_id: String,
    method: String,
    outcome: String,
    status_class: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RequestDurationLabels {
    route_id: String,
    method: String,
    outcome: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct UpstreamDurationLabels {
    route_id: String,
    upstream_host: String,
    result: String,
}

pub fn extract_or_generate_request_id(headers: &HeaderMap) -> String {
    headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| is_valid_request_id(value))
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string())
}

pub fn insert_request_id_header(headers: &mut HeaderMap, request_id: &str) {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert(REQUEST_ID_HEADER, value);
    }
}

pub fn is_sensitive_header_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "authorization" | "x-api-key" | "proxy-authorization"
    )
}

pub fn init_tracing(config: Option<&ObservabilityConfig>) -> Result<(), String> {
    static TRACING_INITIALIZED: OnceLock<()> = OnceLock::new();
    static FILE_LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();
    if TRACING_INITIALIZED.get().is_some() {
        return Ok(());
    }

    let (logging, tracing_cfg) = if let Some(config) = config {
        (config.logging.clone(), config.tracing.clone())
    } else {
        (Default::default(), Default::default())
    };
    let env_filter = EnvFilter::try_new(logging.level.trim()).map_err(|err| {
        format!(
            "invalid `observability.logging.level` value `{}`: {err}",
            logging.level
        )
    })?;
    let otel_layer = if tracing_cfg.enabled {
        build_otel_layer(&tracing_cfg)?
    } else {
        None
    };
    let (file_writer, file_guard) = build_file_log_writer(&logging)?;
    if let Some(guard) = file_guard {
        let _ = FILE_LOG_GUARD.set(guard);
    }

    let init_result = match logging.format {
        LogFormat::Json => {
            let stdout_layer = logging.to_stdout.then(|| {
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_current_span(false)
                    .with_span_list(false)
            });
            let file_layer = file_writer.map(|writer| {
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_ansi(false)
                    .with_current_span(false)
                    .with_span_list(false)
                    .with_writer(writer)
            });

            tracing::subscriber::set_global_default(
                tracing_subscriber::registry()
                    .with(otel_layer)
                    .with(env_filter)
                    .with(stdout_layer)
                    .with(file_layer),
            )
        }
        LogFormat::Text => {
            let stdout_layer = logging.to_stdout.then(tracing_subscriber::fmt::layer);
            let file_layer = file_writer.map(|writer| {
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(writer)
            });

            tracing::subscriber::set_global_default(
                tracing_subscriber::registry()
                    .with(otel_layer)
                    .with(env_filter)
                    .with(stdout_layer)
                    .with(file_layer),
            )
        }
    };

    init_result.map_err(|err| format!("failed to initialize tracing subscriber: {err}"))?;

    let _ = TRACING_INITIALIZED.set(());
    Ok(())
}

fn build_file_log_writer(logging: &LoggingConfig) -> Result<FileLogWriter, String> {
    let Some(file) = &logging.file else {
        return Ok((None, None));
    };
    if !file.enabled {
        return Ok((None, None));
    }

    let dir = file.dir.trim();
    fs::create_dir_all(dir)
        .map_err(|err| format!("failed to create log directory `{dir}`: {err}"))?;
    prune_old_log_files(file)?;

    let appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_rotation(file.rotation.clone()),
        dir,
        file.prefix.trim(),
    );
    let (writer, guard) = tracing_appender::non_blocking(appender);
    Ok((Some(writer), Some(guard)))
}

fn tracing_rotation(rotation: LogRotation) -> tracing_appender::rolling::Rotation {
    match rotation {
        LogRotation::Minutely => tracing_appender::rolling::Rotation::MINUTELY,
        LogRotation::Hourly => tracing_appender::rolling::Rotation::HOURLY,
        LogRotation::Daily => tracing_appender::rolling::Rotation::DAILY,
        LogRotation::Never => tracing_appender::rolling::Rotation::NEVER,
    }
}

fn prune_old_log_files(file: &LogFileConfig) -> Result<(), String> {
    let prefix = file.prefix.trim();
    let dir = file.dir.trim();
    let entries =
        fs::read_dir(dir).map_err(|err| format!("failed to read log directory `{dir}`: {err}"))?;

    let mut candidates: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to inspect log file entry: {err}"))?;
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect log file type: {err}"))?;
        if !file_type.is_file() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(prefix) {
            continue;
        }

        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        candidates.push((entry.path(), modified));
    }

    candidates.sort_by(|left, right| right.1.cmp(&left.1));
    for (path, _) in candidates.into_iter().skip(file.max_files) {
        fs::remove_file(&path)
            .map_err(|err| format!("failed to remove old log file `{}`: {err}", path.display()))?;
    }
    Ok(())
}

fn build_otel_layer(
    config: &TracingConfig,
) -> Result<
    Option<
        tracing_opentelemetry::OpenTelemetryLayer<
            tracing_subscriber::Registry,
            opentelemetry_sdk::trace::Tracer,
        >,
    >,
    String,
> {
    let Some(otlp) = &config.otlp else {
        return Ok(None);
    };

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otlp.endpoint.trim().to_string())
        .with_timeout(Duration::from_millis(otlp.timeout_ms))
        .build()
        .map_err(|err| format!("failed to initialize OTLP exporter: {err}"))?;

    let provider = TracerProvider::builder()
        .with_sampler(Sampler::TraceIdRatioBased(config.sample_ratio))
        .with_batch_exporter(exporter, Tokio)
        .build();
    let tracer = provider.tracer("ai-gw-lite");
    opentelemetry::global::set_tracer_provider(provider);

    Ok(Some(tracing_opentelemetry::layer().with_tracer(tracer)))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(token)
}

fn is_valid_request_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::{extract_or_generate_request_id, is_sensitive_header_name, tracing_rotation};
    use crate::config::LogRotation;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn keep_valid_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("trace-123_abc"));

        let request_id = extract_or_generate_request_id(&headers);
        assert_eq!(request_id, "trace-123_abc");
    }

    #[test]
    fn generate_request_id_for_invalid_value() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("bad request id"));

        let request_id = extract_or_generate_request_id(&headers);
        assert!(!request_id.is_empty());
        assert_ne!(request_id, "bad request id");
    }

    #[test]
    fn sensitive_header_detection_works() {
        assert!(is_sensitive_header_name("authorization"));
        assert!(is_sensitive_header_name("X-API-Key"));
        assert!(is_sensitive_header_name("proxy-authorization"));
        assert!(!is_sensitive_header_name("content-type"));
    }

    #[test]
    fn tracing_rotation_mapping_works() {
        assert_eq!(
            tracing_rotation(LogRotation::Minutely),
            tracing_appender::rolling::Rotation::MINUTELY
        );
        assert_eq!(
            tracing_rotation(LogRotation::Hourly),
            tracing_appender::rolling::Rotation::HOURLY
        );
        assert_eq!(
            tracing_rotation(LogRotation::Daily),
            tracing_appender::rolling::Rotation::DAILY
        );
        assert_eq!(
            tracing_rotation(LogRotation::Never),
            tracing_appender::rolling::Rotation::NEVER
        );
    }
}

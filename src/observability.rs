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
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

pub const REQUEST_ID_HEADER: &str = "x-request-id";
const ONE_HOUR_MINUTES: u64 = 60;
const TWENTY_FOUR_HOURS_MINUTES: u64 = 24 * 60;

type FileLogWriter = (
    Option<tracing_appender::non_blocking::NonBlocking>,
    Option<tracing_appender::non_blocking::WorkerGuard>,
);

#[derive(Clone, Default)]
pub struct ObservabilityRuntime {
    pub metrics: Option<Arc<GatewayMetrics>>,
    metrics_path: Option<String>,
    metrics_ui_path: Option<String>,
    metrics_summary_path: Option<String>,
    metrics_token: Option<String>,
}

impl ObservabilityRuntime {
    pub fn from_config(config: Option<&ObservabilityConfig>) -> Self {
        let Some(config) = config else {
            return Self::default();
        };

        if config.metrics.enabled {
            let metrics_path = normalize_metrics_path(config.metrics.path.as_str());
            return Self {
                metrics: Some(Arc::new(GatewayMetrics::new())),
                metrics_path: Some(metrics_path.clone()),
                metrics_ui_path: Some(metrics_sub_path(metrics_path.as_str(), "ui")),
                metrics_summary_path: Some(metrics_sub_path(metrics_path.as_str(), "summary")),
                metrics_token: Some(config.metrics.token.clone()),
            };
        }

        Self::default()
    }

    pub fn metrics_path(&self) -> Option<&str> {
        self.metrics_path.as_deref()
    }

    pub fn metrics_ui_path(&self) -> Option<&str> {
        self.metrics_ui_path.as_deref()
    }

    pub fn metrics_summary_path(&self) -> Option<&str> {
        self.metrics_summary_path.as_deref()
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

    pub fn snapshot_summary(&self) -> Option<ObservabilitySummary> {
        let metrics = self.metrics.as_ref()?;
        Some(metrics.snapshot_summary())
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
    summary_state: Mutex<SummaryState>,
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
            summary_state: Mutex::new(SummaryState::default()),
        }
    }

    pub fn observe_request(
        &self,
        route_id: &str,
        token_label: Option<&str>,
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

        if let Ok(mut state) = self.summary_state.lock() {
            state.observe_request(route_id, token_label, epoch_minute_now());
        }
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
        if let Ok(mut state) = self.summary_state.lock() {
            state.adjust_inflight(route_id, 1, epoch_minute_now());
        }
    }

    pub fn dec_inflight(&self, route_id: &str) {
        self.inflight_requests
            .get_or_create(&RouteLabels {
                route_id: route_id.to_string(),
            })
            .dec();
        if let Ok(mut state) = self.summary_state.lock() {
            state.adjust_inflight(route_id, -1, epoch_minute_now());
        }
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

    pub fn snapshot_summary(&self) -> ObservabilitySummary {
        if let Ok(state) = self.summary_state.lock() {
            state.snapshot()
        } else {
            ObservabilitySummary::empty()
        }
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

#[derive(Clone, Debug, Serialize)]
pub struct ObservabilitySummary {
    pub generated_at_unix_ms: u64,
    pub total_requests_1h: u64,
    pub total_requests_24h: u64,
    pub routes: Vec<RouteWindowSummary>,
    pub tokens: Vec<TokenWindowSummary>,
}

impl ObservabilitySummary {
    fn empty() -> Self {
        Self {
            generated_at_unix_ms: epoch_millis_now(),
            total_requests_1h: 0,
            total_requests_24h: 0,
            routes: Vec::new(),
            tokens: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteWindowSummary {
    pub route_id: String,
    pub requests_1h: u64,
    pub requests_24h: u64,
    pub inflight_current: u64,
    pub inflight_peak_1h: u64,
    pub inflight_peak_24h: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TokenWindowSummary {
    pub token: String,
    pub requests_1h: u64,
    pub requests_24h: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct RequestMinuteBucket {
    minute_epoch: u64,
    requests: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct RouteMinuteBucket {
    minute_epoch: u64,
    requests: u64,
    max_inflight: u64,
}

#[derive(Debug, Default)]
struct SummaryState {
    route_buckets: HashMap<String, VecDeque<RouteMinuteBucket>>,
    token_buckets: HashMap<String, VecDeque<RequestMinuteBucket>>,
    route_inflight: HashMap<String, u64>,
}

impl SummaryState {
    fn observe_request(&mut self, route_id: &str, token_label: Option<&str>, minute_epoch: u64) {
        let current_inflight = self.route_inflight.get(route_id).copied().unwrap_or(0);
        if let Some(route_bucket) = ensure_route_bucket(
            self.route_buckets.entry(route_id.to_string()).or_default(),
            minute_epoch,
        ) {
            route_bucket.requests = route_bucket.requests.saturating_add(1);
            route_bucket.max_inflight = route_bucket.max_inflight.max(current_inflight);
        }

        if let Some(label) = token_label
            && let Some(token_bucket) = ensure_request_bucket(
                self.token_buckets.entry(label.to_string()).or_default(),
                minute_epoch,
            )
        {
            token_bucket.requests = token_bucket.requests.saturating_add(1);
        }

        self.prune(minute_epoch);
    }

    fn adjust_inflight(&mut self, route_id: &str, delta: i64, minute_epoch: u64) {
        let entry = self.route_inflight.entry(route_id.to_string()).or_insert(0);
        if delta >= 0 {
            *entry = entry.saturating_add(delta as u64);
        } else {
            *entry = entry.saturating_sub(delta.unsigned_abs());
        }
        let current_inflight = *entry;
        if let Some(route_bucket) = ensure_route_bucket(
            self.route_buckets.entry(route_id.to_string()).or_default(),
            minute_epoch,
        ) {
            route_bucket.max_inflight = route_bucket.max_inflight.max(current_inflight);
        }

        self.prune(minute_epoch);
    }

    fn prune(&mut self, minute_epoch: u64) {
        let cutoff = minute_epoch.saturating_sub(TWENTY_FOUR_HOURS_MINUTES.saturating_sub(1));

        let mut empty_routes = Vec::new();
        for (route_id, buckets) in &mut self.route_buckets {
            prune_old_route_buckets(buckets, cutoff);
            let inflight = self.route_inflight.get(route_id).copied().unwrap_or(0);
            if buckets.is_empty() && inflight == 0 {
                empty_routes.push(route_id.clone());
            }
        }
        for route_id in empty_routes {
            self.route_buckets.remove(route_id.as_str());
            self.route_inflight.remove(route_id.as_str());
        }
        self.route_inflight.retain(|route_id, inflight| {
            *inflight > 0 || self.route_buckets.contains_key(route_id)
        });

        let mut empty_tokens = Vec::new();
        for (token, buckets) in &mut self.token_buckets {
            prune_old_request_buckets(buckets, cutoff);
            if buckets.is_empty() {
                empty_tokens.push(token.clone());
            }
        }
        for token in empty_tokens {
            self.token_buckets.remove(token.as_str());
        }
    }

    fn snapshot(&self) -> ObservabilitySummary {
        let now_minute = epoch_minute_now();
        let minute_1h = now_minute.saturating_sub(ONE_HOUR_MINUTES.saturating_sub(1));
        let minute_24h = now_minute.saturating_sub(TWENTY_FOUR_HOURS_MINUTES.saturating_sub(1));

        let mut route_ids: Vec<String> = self.route_buckets.keys().cloned().collect();
        for route_id in self.route_inflight.keys() {
            if !self.route_buckets.contains_key(route_id) {
                route_ids.push(route_id.clone());
            }
        }
        route_ids.sort();
        route_ids.dedup();

        let mut routes = Vec::with_capacity(route_ids.len());
        for route_id in route_ids {
            let mut requests_1h = 0_u64;
            let mut requests_24h = 0_u64;
            let mut inflight_peak_1h = 0_u64;
            let mut inflight_peak_24h = 0_u64;
            if let Some(buckets) = self.route_buckets.get(route_id.as_str()) {
                for bucket in buckets {
                    if bucket.minute_epoch >= minute_24h {
                        requests_24h = requests_24h.saturating_add(bucket.requests);
                        inflight_peak_24h = inflight_peak_24h.max(bucket.max_inflight);
                    }
                    if bucket.minute_epoch >= minute_1h {
                        requests_1h = requests_1h.saturating_add(bucket.requests);
                        inflight_peak_1h = inflight_peak_1h.max(bucket.max_inflight);
                    }
                }
            }

            routes.push(RouteWindowSummary {
                route_id: route_id.clone(),
                requests_1h,
                requests_24h,
                inflight_current: self
                    .route_inflight
                    .get(route_id.as_str())
                    .copied()
                    .unwrap_or(0),
                inflight_peak_1h,
                inflight_peak_24h,
            });
        }

        routes.sort_by(|left, right| {
            right
                .requests_24h
                .cmp(&left.requests_24h)
                .then(right.inflight_current.cmp(&left.inflight_current))
                .then_with(|| left.route_id.cmp(&right.route_id))
        });

        let mut tokens: Vec<TokenWindowSummary> = self
            .token_buckets
            .iter()
            .map(|(token, buckets)| {
                let mut requests_1h = 0_u64;
                let mut requests_24h = 0_u64;
                for bucket in buckets {
                    if bucket.minute_epoch >= minute_24h {
                        requests_24h = requests_24h.saturating_add(bucket.requests);
                    }
                    if bucket.minute_epoch >= minute_1h {
                        requests_1h = requests_1h.saturating_add(bucket.requests);
                    }
                }
                TokenWindowSummary {
                    token: token.clone(),
                    requests_1h,
                    requests_24h,
                }
            })
            .collect();
        tokens.sort_by(|left, right| {
            right
                .requests_24h
                .cmp(&left.requests_24h)
                .then_with(|| left.token.cmp(&right.token))
        });

        let total_requests_1h = routes
            .iter()
            .fold(0_u64, |acc, route| acc.saturating_add(route.requests_1h));
        let total_requests_24h = routes
            .iter()
            .fold(0_u64, |acc, route| acc.saturating_add(route.requests_24h));
        ObservabilitySummary {
            generated_at_unix_ms: epoch_millis_now(),
            total_requests_1h,
            total_requests_24h,
            routes,
            tokens,
        }
    }
}

fn ensure_request_bucket(
    buckets: &mut VecDeque<RequestMinuteBucket>,
    minute_epoch: u64,
) -> Option<&mut RequestMinuteBucket> {
    if let Some(last) = buckets.back()
        && last.minute_epoch == minute_epoch
    {
        return buckets.back_mut();
    }
    buckets.push_back(RequestMinuteBucket {
        minute_epoch,
        requests: 0,
    });
    buckets.back_mut()
}

fn ensure_route_bucket(
    buckets: &mut VecDeque<RouteMinuteBucket>,
    minute_epoch: u64,
) -> Option<&mut RouteMinuteBucket> {
    if let Some(last) = buckets.back()
        && last.minute_epoch == minute_epoch
    {
        return buckets.back_mut();
    }
    buckets.push_back(RouteMinuteBucket {
        minute_epoch,
        requests: 0,
        max_inflight: 0,
    });
    buckets.back_mut()
}

fn prune_old_request_buckets(buckets: &mut VecDeque<RequestMinuteBucket>, cutoff: u64) {
    while buckets
        .front()
        .is_some_and(|bucket| bucket.minute_epoch < cutoff)
    {
        let _ = buckets.pop_front();
    }
}

fn prune_old_route_buckets(buckets: &mut VecDeque<RouteMinuteBucket>, cutoff: u64) {
    while buckets
        .front()
        .is_some_and(|bucket| bucket.minute_epoch < cutoff)
    {
        let _ = buckets.pop_front();
    }
}

fn normalize_metrics_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "/" {
        return "/".to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

fn metrics_sub_path(base_path: &str, suffix: &str) -> String {
    if base_path == "/" {
        return format!("/{suffix}");
    }
    format!("{base_path}/{suffix}")
}

fn epoch_minute_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 60)
        .unwrap_or(0)
}

fn epoch_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn token_label(token: &str) -> String {
    let token = token.trim();
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    let short_hash = hasher.finish() as u32;
    if token.is_empty() {
        return format!("***#{short_hash:08x}");
    }

    let prefix: String = token.chars().take(3).collect();
    let suffix: String = token
        .chars()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if token.chars().count() <= 5 {
        return format!("{prefix}***#{short_hash:08x}");
    }
    format!("{prefix}***{suffix}#{short_hash:08x}")
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

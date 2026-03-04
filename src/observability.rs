use crate::config::{
    LogFileConfig, LogFormat, LogRotation, LoggingConfig, MetricsSqliteConfig, ObservabilityConfig,
    TokenStatsConfig, TracingConfig,
};
use crate::config_storage::{ConfigStorage, ConfigValidationResult};
use crate::token_quota::{TokenQuotaManager, TokenQuotaChecker};
use crate::token_stats::TokenStatsCollector;
use crate::token_stats_storage::TokenStatsStorage;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use dashmap::DashMap;
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
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

pub const REQUEST_ID_HEADER: &str = "x-request-id";
const ONE_HOUR_MINUTES: u64 = 60;
const TWENTY_FOUR_HOURS_MINUTES: u64 = 24 * 60;
const ONE_WEEK_MINUTES: u64 = 7 * 24 * 60;
const ONE_MONTH_MINUTES: u64 = 30 * 24 * 60;
const FIVE_MINUTES: u64 = 5;

type FileLogWriter = (
    Option<tracing_appender::non_blocking::NonBlocking>,
    Option<tracing_appender::non_blocking::WorkerGuard>,
);

#[derive(Clone)]
pub struct ObservabilityRuntime {
    pub metrics: Option<Arc<GatewayMetrics>>,
    metrics_path: Option<String>,
    metrics_ui_path: Option<String>,
    metrics_summary_path: Option<String>,
    metrics_token: Option<String>,
    pub storage: Option<Arc<MetricsStorage>>,
    /// Token统计收集器
    pub token_stats: Option<Arc<TokenStatsCollector>>,
    /// Token配额管理器
    pub token_quota_manager: Option<Arc<TokenQuotaManager>>,
}

impl Default for ObservabilityRuntime {
    fn default() -> Self {
        Self {
            metrics: None,
            metrics_path: None,
            metrics_ui_path: None,
            metrics_summary_path: None,
            metrics_token: None,
            storage: None,
            token_stats: None,
            token_quota_manager: None,
        }
    }
}

impl ObservabilityRuntime {
    pub async fn from_config(
        config: Option<&ObservabilityConfig>,
        token_stats_config: Option<&TokenStatsConfig>,
        resolved_api_keys: Option<&[crate::config::ResolvedApiKey]>,
        config_storage: Option<Arc<ConfigStorage>>,
    ) -> Result<Self, String> {
        let mut runtime = Self::default();

        // Initialize metrics if configured
        if let Some(obs_config) = config {
            if obs_config.metrics.enabled {
                let metrics_path = normalize_metrics_path(obs_config.metrics.path.as_str());

                // Initialize SQLite storage if configured
                let storage = if let Some(sqlite_config) = &obs_config.metrics.sqlite {
                    let (storage, _handle) = MetricsStorage::new(sqlite_config.clone()).await?;
                    info!("Metrics SQLite storage initialized at: {}", sqlite_config.path);
                    Some(Arc::new(storage))
                } else {
                    None
                };

                let metrics = Arc::new(GatewayMetrics::new(storage.clone(), config_storage).await);

                runtime.metrics = Some(metrics);
                runtime.metrics_path = Some(metrics_path.clone());
                runtime.metrics_ui_path = Some(metrics_sub_path(metrics_path.as_str(), "ui"));
                runtime.metrics_summary_path = Some(metrics_sub_path(metrics_path.as_str(), "summary"));
                runtime.metrics_token = Some(obs_config.metrics.token.clone());
                runtime.storage = storage;
            }
        }

        // Initialize token stats and quota manager if configured
        if let Some(ts_config) = token_stats_config {
            if ts_config.enabled {
                // Create token quota manager
                let quota_manager = Arc::new(TokenQuotaManager::new());

                // Initialize quotas from API keys
                if let Some(keys) = resolved_api_keys {
                    quota_manager.init_from_resolved_keys(keys);
                }

                // Create token quota checker
                let _quota_checker = Arc::new(TokenQuotaChecker::new(quota_manager.clone()));

                // Create token stats storage if configured
                let token_storage = if let Some(sqlite_config) = &ts_config.sqlite {
                    let storage = TokenStatsStorage::new(
                        &sqlite_config.path,
                        sqlite_config.flush_interval_secs,
                        sqlite_config.batch_size,
                    )
                    .await
                    .map_err(|e| format!("Failed to create token stats storage: {}", e))?;
                    Some(Arc::new(storage))
                } else {
                    None
                };

                // Create token stats collector
                let token_stats = Arc::new(TokenStatsCollector::new(
                    token_storage,
                    Some(quota_manager.clone()),
                ));

                // Load historical stats from SQLite
                if let Err(e) = token_stats.load_historical_stats().await {
                    warn!("Failed to load historical token stats: {}", e);
                }

                runtime.token_stats = Some(token_stats);
                runtime.token_quota_manager = Some(quota_manager);

                info!("Token stats and quota management initialized");
            }
        }

        Ok(runtime)
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
    // Use DashMap for fine-grained concurrent access instead of Mutex<SummaryState>
    route_stats: DashMap<String, RouteStats>,
    route_token_stats: DashMap<String, RouteTokenStats>,
    ip_stats: DashMap<String, IPStats>,
    storage: Option<Arc<MetricsStorage>>,
    config_storage: Option<Arc<ConfigStorage>>,
}

impl GatewayMetrics {
    /// Synchronous constructor for Default
    fn new_sync(storage: Option<Arc<MetricsStorage>>, config_storage: Option<Arc<ConfigStorage>>) -> Self {
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
            route_stats: DashMap::new(),
            route_token_stats: DashMap::new(),
            ip_stats: DashMap::new(),
            storage,
            config_storage,
        }
    }

    /// Asynchronous constructor that loads historical data from SQLite
    pub async fn new(storage: Option<Arc<MetricsStorage>>, config_storage: Option<Arc<ConfigStorage>>) -> Self {
        // Load historical data first (before creating the instance)
        // to avoid holding MutexGuard across await points
        let historical_data = if let Some(storage) = &storage {
            match storage.load_historical_data().await {
                Ok(data) => Some(data),
                Err(e) => {
                    warn!("Failed to load historical metrics data: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let metrics = Self::new_sync(storage, config_storage);

        // Apply historical data if loaded successfully
        if let Some((route_buckets, route_token_buckets, ip_stats)) = historical_data {
            // Load route stats
            for (route_id, buckets) in route_buckets {
                let mut stats = RouteStats::default();
                stats.buckets = buckets;
                metrics.route_stats.insert(route_id, stats);
            }
            // Load token stats
            for (route_id, token_map) in route_token_buckets {
                let mut stats = RouteTokenStats::default();
                stats.token_buckets = token_map;
                metrics.route_token_stats.insert(route_id, stats);
            }
            // Load IP stats
            for (ip, stats) in ip_stats {
                metrics.ip_stats.insert(ip, stats);
            }
            info!("Historical metrics data loaded successfully");
        }

        metrics
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
        // Filter by config_storage if available
        if let Some(config_storage) = &self.config_storage {
            // Check if route_id is valid
            let is_route_valid = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(config_storage.validate_route(route_id))
            });
            if !is_route_valid {
                return; // Skip recording for invalid routes
            }

            // Check if API key is valid (if token_label is provided)
            if let Some(label) = token_label {
                let validation_result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(config_storage.validate_api_key(label))
                });
                if !matches!(validation_result, ConfigValidationResult::Valid { .. }) {
                    return; // Skip recording for invalid API keys
                }
            }
        }

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

        // Update route stats using DashMap for lock-free concurrent access
        let minute_epoch = epoch_minute_now();
        {
            let mut stats = self.route_stats.entry(route_id.to_string()).or_default();
            let current_inflight = stats.current_inflight;
            if let Some(bucket) = ensure_route_bucket(&mut stats.buckets, minute_epoch) {
                bucket.requests = bucket.requests.saturating_add(1);
                bucket.max_inflight = bucket.max_inflight.max(current_inflight);
            }
        }

        // Update token stats if token_label is provided
        if let Some(label) = token_label {
            let mut token_stats = self.route_token_stats.entry(route_id.to_string()).or_default();
            if let Some(bucket) = ensure_request_bucket(
                token_stats.token_buckets.entry(label.to_string()).or_default(),
                minute_epoch,
            ) {
                bucket.requests = bucket.requests.saturating_add(1);
            }
        }

        // Prune old data occasionally (simple probabilistic approach)
        if minute_epoch % 100 == 0 {
            self.prune_old_data(minute_epoch);
        }

        // Queue for SQLite persistence if enabled
        if let Some(storage) = &self.storage {
            let record = MetricsRecord {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                route_id: route_id.to_string(),
                method: method.as_str().to_string(),
                status_code: status.as_u16() as i32,
                outcome: outcome.to_string(),
                duration_ms: duration.as_millis() as i64,
                token_label: token_label.map(|s| s.to_string()),
                client_ip: None,
                request_path: None,
                upstream_host: None,
                upstream_result: None,
                upstream_duration_ms: None,
            };
            storage.queue_record(record);
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
        // Update inflight count using DashMap
        let mut stats = self.route_stats.entry(route_id.to_string()).or_default();
        stats.current_inflight = stats.current_inflight.saturating_add(1);
    }

    pub fn dec_inflight(&self, route_id: &str) {
        self.inflight_requests
            .get_or_create(&RouteLabels {
                route_id: route_id.to_string(),
            })
            .dec();
        // Update inflight count using DashMap
        let mut stats = self.route_stats.entry(route_id.to_string()).or_default();
        stats.current_inflight = stats.current_inflight.saturating_sub(1);
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

    pub fn observe_ip_request(
        &self,
        ip: &str,
        url: &str,
        token_label: Option<&str>,
    ) {
        // Update IP stats using DashMap
        let minute_epoch = epoch_minute_now();
        let mut stats = self.ip_stats.entry(ip.to_string()).or_default();

        // Find or create current minute bucket
        let bucket = if let Some(existing) = stats.buckets.iter_mut().find(|b| b.minute_epoch == minute_epoch) {
            existing
        } else {
            stats.buckets.push_back(IPMinuteBucket {
                minute_epoch,
                requests: 0,
                urls: HashMap::new(),
                tokens: HashMap::new(),
            });
            stats.buckets.back_mut().unwrap()
        };

        bucket.requests = bucket.requests.saturating_add(1);

        // Record URL
        let url_entry = bucket.urls.entry(url.to_string()).or_insert(0);
        *url_entry = url_entry.saturating_add(1);

        // Record token
        if let Some(label) = token_label {
            let token_entry = bucket.tokens.entry(label.to_string()).or_insert(0);
            *token_entry = token_entry.saturating_add(1);
        }

        // Queue IP record for SQLite persistence
        if let Some(storage) = &self.storage {
            let record = MetricsRecord {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                route_id: String::new(),
                method: String::new(),
                status_code: 0,
                outcome: String::new(),
                duration_ms: 0,
                token_label: token_label.map(|s| s.to_string()),
                client_ip: Some(ip.to_string()),
                request_path: Some(url.to_string()),
                upstream_host: None,
                upstream_result: None,
                upstream_duration_ms: None,
            };
            storage.queue_record(record);
        }
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        if let Ok(registry) = self.registry.read() {
            let _ = encode(&mut output, &registry);
        }
        output
    }

    pub fn snapshot_summary(&self) -> ObservabilitySummary {
        let now_minute = epoch_minute_now();
        let minute_1h = now_minute.saturating_sub(ONE_HOUR_MINUTES.saturating_sub(1));
        let minute_24h = now_minute.saturating_sub(TWENTY_FOUR_HOURS_MINUTES.saturating_sub(1));

        // Collect all route IDs from both route_stats and route_token_stats
        let mut route_ids: Vec<String> = self.route_stats.iter().map(|e| e.key().clone()).collect();
        route_ids.sort();
        route_ids.dedup();

        let mut routes = Vec::with_capacity(route_ids.len());
        for route_id in route_ids {
            let mut requests_1h = 0_u64;
            let mut requests_24h = 0_u64;
            let mut inflight_peak_1h = 0_u64;
            let mut inflight_peak_24h = 0_u64;

            if let Some(stats) = self.route_stats.get(&route_id) {
                for bucket in &stats.buckets {
                    if bucket.minute_epoch >= minute_24h {
                        requests_24h = requests_24h.saturating_add(bucket.requests);
                        inflight_peak_24h = inflight_peak_24h.max(bucket.max_inflight);
                    }
                    if bucket.minute_epoch >= minute_1h {
                        requests_1h = requests_1h.saturating_add(bucket.requests);
                        inflight_peak_1h = inflight_peak_1h.max(bucket.max_inflight);
                    }
                }

                routes.push(RouteWindowSummary {
                    route_id: route_id.clone(),
                    requests_1h,
                    requests_24h,
                    inflight_current: stats.current_inflight,
                    inflight_peak_1h,
                    inflight_peak_24h,
                });
            }
        }

        routes.sort_by(|left, right| {
            right
                .requests_24h
                .cmp(&left.requests_24h)
                .then(right.inflight_current.cmp(&left.inflight_current))
                .then_with(|| left.route_id.cmp(&right.route_id))
        });

        // Collect route-isolated token stats
        let mut tokens: Vec<TokenWindowSummary> = Vec::new();
        for entry in self.route_token_stats.iter() {
            let route_id = entry.key();
            for (token, buckets) in &entry.token_buckets {
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
                tokens.push(TokenWindowSummary {
                    token: token.clone(),
                    route_id: route_id.clone(),
                    requests_1h,
                    requests_24h,
                });
            }
        }
        tokens.sort_by(|left, right| {
            right
                .requests_24h
                .cmp(&left.requests_24h)
                .then_with(|| left.route_id.cmp(&right.route_id))
                .then_with(|| left.token.cmp(&right.token))
        });

        let total_requests_1h = routes
            .iter()
            .fold(0_u64, |acc, route| acc.saturating_add(route.requests_1h));
        let total_requests_24h = routes
            .iter()
            .fold(0_u64, |acc, route| acc.saturating_add(route.requests_24h));

        // Aggregate IP stats
        let ip_stats = self.snapshot_ip_stats(now_minute);

        ObservabilitySummary {
            generated_at_unix_ms: epoch_millis_now(),
            total_requests_1h,
            total_requests_24h,
            routes,
            tokens,
            ip_stats,
        }
    }

    /// Returns a filtered metrics summary containing only valid routes and API keys.
    /// If config_storage is not set, returns the full unfiltered summary.
    pub async fn snapshot_summary_filtered(&self) -> ObservabilitySummary {
        let mut summary = self.snapshot_summary();

        if let Some(config_storage) = &self.config_storage {
            // Get valid route IDs and API key IDs
            let valid_routes = config_storage.get_valid_route_ids().await;
            let valid_api_keys = config_storage.get_valid_api_key_ids().await;

            // Filter routes
            summary.routes.retain(|route| valid_routes.contains(&route.route_id));

            // Filter tokens (API keys)
            summary.tokens.retain(|token| valid_api_keys.contains(&token.token));

            // Recalculate totals after filtering
            summary.total_requests_1h = summary
                .routes
                .iter()
                .fold(0_u64, |acc, route| acc.saturating_add(route.requests_1h));
            summary.total_requests_24h = summary
                .routes
                .iter()
                .fold(0_u64, |acc, route| acc.saturating_add(route.requests_24h));
        }

        summary
    }

    fn snapshot_ip_stats(&self, now_minute: u64) -> IPStatsSummary {
        let minute_5m = now_minute.saturating_sub(FIVE_MINUTES.saturating_sub(1));
        let minute_1h = now_minute.saturating_sub(ONE_HOUR_MINUTES.saturating_sub(1));
        let minute_24h = now_minute.saturating_sub(TWENTY_FOUR_HOURS_MINUTES.saturating_sub(1));
        let minute_7d = now_minute.saturating_sub(ONE_WEEK_MINUTES.saturating_sub(1));
        let minute_30d = now_minute.saturating_sub(ONE_MONTH_MINUTES.saturating_sub(1));

        let mut ip_summaries: Vec<IPWindowSummary> = self
            .ip_stats
            .iter()
            .map(|entry| {
                let ip = entry.key();
                let stats = entry.value();
                let mut requests_5m = 0_u64;
                let mut requests_1h = 0_u64;
                let mut requests_24h = 0_u64;
                let mut requests_7d = 0_u64;
                let mut requests_30d = 0_u64;

                // URL and token aggregation (24h window)
                let mut url_counts: HashMap<String, u64> = HashMap::new();
                let mut token_counts: HashMap<String, u64> = HashMap::new();

                for bucket in &stats.buckets {
                    if bucket.minute_epoch >= minute_30d {
                        requests_30d = requests_30d.saturating_add(bucket.requests);
                        if bucket.minute_epoch >= minute_7d {
                            requests_7d = requests_7d.saturating_add(bucket.requests);
                            if bucket.minute_epoch >= minute_24h {
                                requests_24h = requests_24h.saturating_add(bucket.requests);
                                // Only aggregate URLs and tokens within 24h
                                for (url, count) in &bucket.urls {
                                    let entry = url_counts.entry(url.clone()).or_insert(0);
                                    *entry = entry.saturating_add(*count);
                                }
                                for (token, count) in &bucket.tokens {
                                    let entry = token_counts.entry(token.clone()).or_insert(0);
                                    *entry = entry.saturating_add(*count);
                                }
                                if bucket.minute_epoch >= minute_1h {
                                    requests_1h = requests_1h.saturating_add(bucket.requests);
                                    if bucket.minute_epoch >= minute_5m {
                                        requests_5m = requests_5m.saturating_add(bucket.requests);
                                    }
                                }
                            }
                        }
                    }
                }

                let mut urls: Vec<UrlSummary> = url_counts
                    .into_iter()
                    .map(|(url, count)| UrlSummary { url, count })
                    .collect();
                urls.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.url.cmp(&b.url)));
                urls.truncate(100);

                let mut tokens: Vec<IPTokenSummary> = token_counts
                    .into_iter()
                    .map(|(token, count)| IPTokenSummary { token, count })
                    .collect();
                tokens.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.token.cmp(&b.token)));
                tokens.truncate(100);

                IPWindowSummary {
                    ip: ip.clone(),
                    requests_5m,
                    requests_1h,
                    requests_24h,
                    requests_7d,
                    requests_30d,
                    urls,
                    tokens,
                }
            })
            .collect();

        ip_summaries.sort_by(|left, right| {
            right
                .requests_24h
                .cmp(&left.requests_24h)
                .then_with(|| left.ip.cmp(&right.ip))
        });

        let total_requests_5m = ip_summaries
            .iter()
            .fold(0_u64, |acc, ip| acc.saturating_add(ip.requests_5m));
        let total_requests_1h = ip_summaries
            .iter()
            .fold(0_u64, |acc, ip| acc.saturating_add(ip.requests_1h));
        let total_requests_24h = ip_summaries
            .iter()
            .fold(0_u64, |acc, ip| acc.saturating_add(ip.requests_24h));
        let total_requests_7d = ip_summaries
            .iter()
            .fold(0_u64, |acc, ip| acc.saturating_add(ip.requests_7d));
        let total_requests_30d = ip_summaries
            .iter()
            .fold(0_u64, |acc, ip| acc.saturating_add(ip.requests_30d));

        IPStatsSummary {
            total_requests_5m,
            total_requests_1h,
            total_requests_24h,
            total_requests_7d,
            total_requests_30d,
            ips: ip_summaries,
        }
    }

    /// Prune old data from all DashMaps
    fn prune_old_data(&self, now_minute: u64) {
        let cutoff_30d = now_minute.saturating_sub(ONE_MONTH_MINUTES);

        // Prune route stats
        for mut entry in self.route_stats.iter_mut() {
            entry.buckets.retain(|b| b.minute_epoch >= cutoff_30d);
        }

        // Prune token stats
        for mut entry in self.route_token_stats.iter_mut() {
            for buckets in entry.token_buckets.values_mut() {
                buckets.retain(|b| b.minute_epoch >= cutoff_30d);
            }
            // Remove empty token entries
            entry.token_buckets.retain(|_, buckets| !buckets.is_empty());
        }

        // Prune IP stats
        for mut entry in self.ip_stats.iter_mut() {
            entry.buckets.retain(|b| b.minute_epoch >= cutoff_30d);
        }
        self.ip_stats.retain(|_, stats| !stats.buckets.is_empty());
    }
}

impl Default for GatewayMetrics {
    fn default() -> Self {
        Self::new_sync(None, None)
    }
}

/// Use String for labels - prometheus-client doesn't support Arc<str> directly
/// The Strings are small and short-lived, so cloning is acceptable
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
    pub ip_stats: IPStatsSummary,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct IPStatsSummary {
    pub total_requests_5m: u64,
    pub total_requests_1h: u64,
    pub total_requests_24h: u64,
    pub total_requests_7d: u64,
    pub total_requests_30d: u64,
    pub ips: Vec<IPWindowSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IPWindowSummary {
    pub ip: String,
    pub requests_5m: u64,
    pub requests_1h: u64,
    pub requests_24h: u64,
    pub requests_7d: u64,
    pub requests_30d: u64,
    pub urls: Vec<UrlSummary>,
    pub tokens: Vec<IPTokenSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UrlSummary {
    pub url: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct IPTokenSummary {
    pub token: String,
    pub count: u64,
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
    pub route_id: String,
    pub requests_1h: u64,
    pub requests_24h: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestMinuteBucket {
    minute_epoch: u64,
    requests: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RouteMinuteBucket {
    minute_epoch: u64,
    requests: u64,
    max_inflight: u64,
}

// IP 监控数据结构
#[derive(Clone, Debug, Default)]
struct IPMinuteBucket {
    minute_epoch: u64,
    requests: u64,
    urls: HashMap<String, u64>,
    tokens: HashMap<String, u64>,
}

#[derive(Debug, Default)]
pub(crate) struct IPStats {
    buckets: VecDeque<IPMinuteBucket>,
}

/// Per-route statistics using DashMap for lock-free concurrent access
#[derive(Debug, Default)]
struct RouteStats {
    buckets: VecDeque<RouteMinuteBucket>,
    current_inflight: u64,
}

/// Per-route token statistics
#[derive(Debug, Default)]
struct RouteTokenStats {
    // token_label -> buckets
    token_buckets: HashMap<String, VecDeque<RequestMinuteBucket>>,
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
    token.trim().to_string()
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

/// Metrics record for batch insertion
#[derive(Debug, Clone)]
pub(crate) struct MetricsRecord {
    timestamp: i64,
    route_id: String,
    method: String,
    status_code: i32,
    outcome: String,
    duration_ms: i64,
    token_label: Option<String>,
    client_ip: Option<String>,
    request_path: Option<String>,
    upstream_host: Option<String>,
    upstream_result: Option<String>,
    upstream_duration_ms: Option<i64>,
}

/// SQLite-backed metrics storage
#[derive(Debug)]
pub struct MetricsStorage {
    pool: Pool<Sqlite>,
    _config: MetricsSqliteConfig,
    sender: mpsc::UnboundedSender<MetricsRecord>,
}

impl MetricsStorage {
    /// Initialize SQLite storage and create tables
    pub async fn new(config: MetricsSqliteConfig) -> Result<(Self, MetricsStorageHandle), String> {
        let db_path = Path::new(&config.path);

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("Failed to create metrics db directory: {}", e)
            })?;
        }

        let connection_string = format!("sqlite:{}", config.path);
        let options = SqliteConnectOptions::from_str(&connection_string)
            .map_err(|e| format!("Invalid SQLite connection string: {}", e))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| format!("Failed to connect to SQLite: {}", e))?;

        // Create tables
        Self::create_tables(&pool).await?;

        // Start background batch writer
        let (sender, receiver) = mpsc::unbounded_channel::<MetricsRecord>();
        let pool_clone = pool.clone();
        let flush_interval = Duration::from_secs(config.flush_interval_secs);
        let batch_size = config.batch_size;
        let retention_days = config.retention_days;

        let handle = tokio::spawn(async move {
            Self::batch_writer_task(
                pool_clone,
                receiver,
                flush_interval,
                batch_size,
                retention_days,
            )
            .await;
        });

        let storage = Self {
            pool,
            _config: config.clone(),
            sender,
        };

        let handle = MetricsStorageHandle { handle };

        Ok((storage, handle))
    }

    async fn create_tables(pool: &Pool<Sqlite>) -> Result<(), String> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS metrics_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                route_id TEXT NOT NULL,
                method TEXT NOT NULL,
                status_code INTEGER NOT NULL,
                outcome TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                token_label TEXT,
                client_ip TEXT,
                request_path TEXT,
                upstream_host TEXT,
                upstream_result TEXT,
                upstream_duration_ms INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_metrics_timestamp ON metrics_requests(timestamp);
            CREATE INDEX IF NOT EXISTS idx_metrics_route ON metrics_requests(route_id);
            CREATE INDEX IF NOT EXISTS idx_metrics_ip ON metrics_requests(client_ip);
            CREATE INDEX IF NOT EXISTS idx_metrics_token ON metrics_requests(token_label);
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create metrics tables: {}", e))?;

        // Create aggregated stats tables for faster queries
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS metrics_ip_stats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                window_start INTEGER NOT NULL,
                window_end INTEGER NOT NULL,
                window_type TEXT NOT NULL,
                client_ip TEXT NOT NULL,
                request_count INTEGER NOT NULL DEFAULT 0,
                error_count INTEGER NOT NULL DEFAULT 0,
                total_duration_ms INTEGER NOT NULL DEFAULT 0,
                avg_duration_ms INTEGER NOT NULL DEFAULT 0,
                min_duration_ms INTEGER NOT NULL DEFAULT 0,
                max_duration_ms INTEGER NOT NULL DEFAULT 0,
                routes TEXT, -- JSON array of routes
                tokens TEXT, -- JSON array of tokens
                UNIQUE(window_start, window_type, client_ip)
            );

            CREATE INDEX IF NOT EXISTS idx_ip_stats_window ON metrics_ip_stats(window_start, window_type);
            CREATE INDEX IF NOT EXISTS idx_ip_stats_ip ON metrics_ip_stats(client_ip);
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create IP stats table: {}", e))?;

        Ok(())
    }

    /// Queue a record for batch insertion
    pub(crate) fn queue_record(&self, record: MetricsRecord) {
        let _ = self.sender.send(record);
    }

    /// Load historical data from SQLite and return the data structures
    pub(crate) async fn load_historical_data(
        &self,
    ) -> Result<
        (
            HashMap<String, VecDeque<RouteMinuteBucket>>,
            HashMap<String, HashMap<String, VecDeque<RequestMinuteBucket>>>,
            HashMap<String, IPStats>,
        ),
        String,
    > {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Load data for the last 24 hours (in minutes)
        let twenty_four_hours_ago = now - (24 * 60 * 60);
        let one_month_ago = now - (30 * 24 * 60 * 60);

        let mut route_buckets: HashMap<String, VecDeque<RouteMinuteBucket>> = HashMap::new();
        let mut route_token_buckets: HashMap<String, HashMap<String, VecDeque<RequestMinuteBucket>>> =
            HashMap::new();
        let mut ip_stats: HashMap<String, IPStats> = HashMap::new();

        // Load route and token stats from metrics_requests table
        let rows = sqlx::query_as::<_, (String, String, i64, i64)>(
            r#"
            SELECT
                route_id,
                COALESCE(token_label, '') as token_label,
                (timestamp / 60) as minute_epoch,
                COUNT(*) as request_count
            FROM metrics_requests
            WHERE timestamp >= ?
            GROUP BY route_id, token_label, minute_epoch
            ORDER BY minute_epoch DESC
            "#,
        )
        .bind(twenty_four_hours_ago)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to load historical metrics: {}", e))?;

        for (route_id, token_label, minute_epoch, request_count) in rows {
            let minute_epoch = minute_epoch as u64;

            // Update route buckets
            let buckets = route_buckets.entry(route_id.clone()).or_default();
            if let Some(bucket) = ensure_route_bucket(buckets, minute_epoch) {
                bucket.requests = bucket.requests.saturating_add(request_count as u64);
            }

            // Update token buckets
            if !token_label.is_empty() {
                let route_tokens = route_token_buckets.entry(route_id).or_default();
                let token_buckets = route_tokens.entry(token_label).or_default();
                if let Some(bucket) = ensure_request_bucket(token_buckets, minute_epoch) {
                    bucket.requests = bucket.requests.saturating_add(request_count as u64);
                }
            }
        }

        // Load IP stats
        let ip_rows = sqlx::query_as::<_, (String, String, String, i64, i64)>(
            r#"
            SELECT
                client_ip,
                request_path,
                COALESCE(token_label, '') as token_label,
                (timestamp / 60) as minute_epoch,
                COUNT(*) as request_count
            FROM metrics_requests
            WHERE timestamp >= ? AND client_ip IS NOT NULL
            GROUP BY client_ip, request_path, token_label, minute_epoch
            ORDER BY minute_epoch DESC
            "#,
        )
        .bind(one_month_ago)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to load historical IP stats: {}", e))?;

        for (client_ip, request_path, token_label, minute_epoch, request_count) in ip_rows {
            let minute_epoch = minute_epoch as u64;

            let ip_stat = ip_stats.entry(client_ip).or_default();

            // Find or create the bucket for this minute
            let bucket = if let Some(existing) = ip_stat.buckets.iter_mut().find(|b| b.minute_epoch == minute_epoch) {
                existing
            } else {
                ip_stat.buckets.push_back(IPMinuteBucket {
                    minute_epoch,
                    requests: 0,
                    urls: HashMap::new(),
                    tokens: HashMap::new(),
                });
                ip_stat.buckets.back_mut().unwrap()
            };

            bucket.requests = bucket.requests.saturating_add(request_count as u64);

            if !request_path.is_empty() {
                let url_entry = bucket.urls.entry(request_path).or_insert(0);
                *url_entry = url_entry.saturating_add(request_count as u64);
            }

            if !token_label.is_empty() {
                let token_entry = bucket.tokens.entry(token_label).or_insert(0);
                *token_entry = token_entry.saturating_add(request_count as u64);
            }
        }

        let total_routes: usize = route_buckets.values().map(|b| b.len()).sum();
        let total_tokens: usize = route_token_buckets
            .values()
            .map(|m| m.values().map(|b| b.len()).sum::<usize>())
            .sum();
        info!(
            "Loaded historical metrics: {} route buckets, {} token buckets, {} IPs",
            total_routes,
            total_tokens,
            ip_stats.len()
        );

        Ok((route_buckets, route_token_buckets, ip_stats))
    }

    async fn batch_writer_task(
        pool: Pool<Sqlite>,
        mut receiver: mpsc::UnboundedReceiver<MetricsRecord>,
        flush_interval: Duration,
        batch_size: usize,
        retention_days: u32,
    ) {
        let mut batch = Vec::with_capacity(batch_size);
        let mut interval = interval(flush_interval);

        loop {
            tokio::select! {
                Some(record) = receiver.recv() => {
                    batch.push(record);
                    if batch.len() >= batch_size {
                        Self::flush_batch(&pool, &batch).await;
                        batch.clear();
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        Self::flush_batch(&pool, &batch).await;
                        batch.clear();
                    }
                    // Periodic cleanup of old records
                    Self::cleanup_old_records(&pool, retention_days).await;
                }
            }
        }
    }

    async fn flush_batch(pool: &Pool<Sqlite>, batch: &[MetricsRecord]) {
        if batch.is_empty() {
            return;
        }

        let mut query_builder = String::from(
            "INSERT INTO metrics_requests \
             (timestamp, route_id, method, status_code, outcome, duration_ms, \
              token_label, client_ip, request_path, upstream_host, upstream_result, upstream_duration_ms) \
             VALUES "
        );

        let mut _param_count = 0;
        for (i, _) in batch.iter().enumerate() {
            if i > 0 {
                query_builder.push_str(", ");
            }
            query_builder.push_str("(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
            _param_count += 12;
        }

        let mut query = sqlx::query(&query_builder);

        for record in batch {
            query = query
                .bind(record.timestamp)
                .bind(&record.route_id)
                .bind(&record.method)
                .bind(record.status_code)
                .bind(&record.outcome)
                .bind(record.duration_ms)
                .bind(record.token_label.as_deref())
                .bind(record.client_ip.as_deref())
                .bind(record.request_path.as_deref())
                .bind(record.upstream_host.as_deref())
                .bind(record.upstream_result.as_deref())
                .bind(record.upstream_duration_ms);
        }

        if let Err(e) = query.execute(pool).await {
            warn!("Failed to flush metrics batch: {}", e);
        }
    }

    async fn cleanup_old_records(pool: &Pool<Sqlite>, retention_days: u32) {
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            - (retention_days as i64 * 24 * 60 * 60);

        // Cleanup detailed request records
        if let Err(e) = sqlx::query("DELETE FROM metrics_requests WHERE timestamp < ?")
            .bind(cutoff)
            .execute(pool)
            .await
        {
            warn!("Failed to cleanup old metrics records: {}", e);
        }

        // Cleanup old aggregated stats (keep 30 days worth)
        let stats_cutoff = cutoff - (23 * 24 * 60 * 60); // Additional 23 days for stats
        if let Err(e) = sqlx::query("DELETE FROM metrics_ip_stats WHERE window_end < ?")
            .bind(stats_cutoff)
            .execute(pool)
            .await
        {
            warn!("Failed to cleanup old stats records: {}", e);
        }
    }

    /// Query IP statistics for a time window
    pub async fn query_ip_stats(
        &self,
        window_seconds: u64,
        ip_filter: Option<&str>,
        sort_by: &str,
        descending: bool,
        limit: usize,
    ) -> Result<Vec<IpStatsRow>, sqlx::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let window_start = now - window_seconds as i64;

        let order_clause = match sort_by {
            "requests" => "ORDER BY request_count",
            "errors" => "ORDER BY error_count",
            "latency_avg" => "ORDER BY avg_duration_ms",
            _ => "ORDER BY request_count",
        };

        let order_dir = if descending { "DESC" } else { "ASC" };

        let query_str = format!(
            r#"SELECT
                client_ip,
                SUM(request_count) as requests,
                SUM(error_count) as errors,
                AVG(avg_duration_ms) as latency_avg,
                routes,
                tokens
            FROM metrics_requests_view
            WHERE timestamp >= ?
                AND (?1 IS NULL OR client_ip LIKE ?)
            GROUP BY client_ip
            {} {}
            LIMIT ?"#,
            order_clause, order_dir
        );

        let ip_pattern = ip_filter.map(|f| format!("%{}%", f));

        let rows = sqlx::query_as::<_, IpStatsRow>(&query_str)
            .bind(window_start)
            .bind(ip_pattern)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    /// Aggregate and store IP stats for a time window
    pub async fn aggregate_ip_stats(&self, window_start: i64, window_end: i64, window_type: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO metrics_ip_stats
                (window_start, window_end, window_type, client_ip, request_count,
                 error_count, total_duration_ms, avg_duration_ms, min_duration_ms, max_duration_ms)
            SELECT
                ? as window_start,
                ? as window_end,
                ? as window_type,
                client_ip,
                COUNT(*) as request_count,
                SUM(CASE WHEN outcome != 'success' THEN 1 ELSE 0 END) as error_count,
                SUM(duration_ms) as total_duration_ms,
                AVG(duration_ms) as avg_duration_ms,
                MIN(duration_ms) as min_duration_ms,
                MAX(duration_ms) as max_duration_ms
            FROM metrics_requests
            WHERE timestamp >= ? AND timestamp < ? AND client_ip IS NOT NULL
            GROUP BY client_ip
            ON CONFLICT(window_start, window_type, client_ip) DO UPDATE SET
                request_count = excluded.request_count,
                error_count = excluded.error_count,
                total_duration_ms = excluded.total_duration_ms,
                avg_duration_ms = excluded.avg_duration_ms,
                min_duration_ms = excluded.min_duration_ms,
                max_duration_ms = excluded.max_duration_ms
            "#,
        )
        .bind(window_start)
        .bind(window_end)
        .bind(window_type)
        .bind(window_start)
        .bind(window_end)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

/// Row structure for IP stats query
#[derive(Debug, sqlx::FromRow)]
pub struct IpStatsRow {
    pub client_ip: String,
    pub requests: i64,
    pub errors: i64,
    pub latency_avg: i64,
    pub routes: Option<String>,
    pub tokens: Option<String>,
}

/// Handle to the background storage task
pub struct MetricsStorageHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl MetricsStorageHandle {
    pub fn abort(&self) {
        self.handle.abort();
    }
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

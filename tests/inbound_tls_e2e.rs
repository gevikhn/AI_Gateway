use ai_gw_lite::config::{
    AppConfig, GatewayAuthConfig, HeaderInjection, InboundTlsConfig, RouteConfig,
    TokenSourceConfig, UpstreamConfig,
};
use ai_gw_lite::server::run_server;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn https_listener_auto_generates_self_signed_cert() {
    let temp_dir = make_temp_dir();
    let cert_path = temp_dir.join("gateway-selfsigned.crt");
    let key_path = temp_dir.join("gateway-selfsigned.key");
    let listen_addr = unused_local_addr();

    let config = AppConfig {
        listen: listen_addr.to_string(),
        gateway_auth: GatewayAuthConfig {
            tokens: vec!["gw_token".to_string()],
            token_sources: vec![TokenSourceConfig::AuthorizationBearer],
        },
        routes: vec![RouteConfig {
            id: "openai".to_string(),
            prefix: "/openai".to_string(),
            upstream: UpstreamConfig {
                base_url: "https://api.openai.com".to_string(),
                strip_prefix: true,
                connect_timeout_ms: 1_000,
                request_timeout_ms: 1_000,
                inject_headers: vec![HeaderInjection {
                    name: "authorization".to_string(),
                    value: "Bearer test".to_string(),
                }],
                remove_headers: vec!["authorization".to_string()],
                forward_xff: false,
                proxy: None,
                upstream_key_max_inflight: None,
            },
        }],
        inbound_tls: Some(InboundTlsConfig {
            cert_path: None,
            key_path: None,
            self_signed_cert_path: cert_path.to_string_lossy().to_string(),
            self_signed_key_path: key_path.to_string_lossy().to_string(),
        }),
        cors: None,
        rate_limit: None,
        concurrency: None,
        observability: None,
    };

    let server_handle = tokio::spawn(async move { run_server(Arc::new(config)).await });

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("https client should build");
    let healthz_url = format!("https://{listen_addr}/healthz");
    let response = wait_until_ready(&client, &healthz_url).await;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(cert_path.exists(), "self-signed cert should be generated");
    assert!(key_path.exists(), "self-signed key should be generated");

    server_handle.abort();
    let _ = server_handle.await;
    let _ = std::fs::remove_dir_all(temp_dir);
}

async fn wait_until_ready(client: &reqwest::Client, url: &str) -> reqwest::Response {
    let mut last_error = None;

    for _ in 0..40 {
        match client.get(url).send().await {
            Ok(response) => return response,
            Err(err) => {
                last_error = Some(err);
                sleep(Duration::from_millis(50)).await;
            }
        }
    }

    panic!(
        "https server did not become ready: {}",
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "unknown error".to_string())
    );
}

fn unused_local_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind should succeed");
    let addr = listener
        .local_addr()
        .expect("local addr should be available");
    drop(listener);
    addr
}

fn make_temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "ai-gw-lite-inbound-tls-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

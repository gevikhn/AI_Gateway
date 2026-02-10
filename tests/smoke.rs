use ai_gw_lite::config::AppConfig;
use ai_gw_lite::proxy::{build_upstream_url_for_route, match_route};

#[test]
fn dev_config_loads_and_has_route() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("config").join("dev.yaml");
    let config = AppConfig::load_from_file(path).expect("dev config should load");

    assert_eq!(config.listen, "127.0.0.1:8080");
    assert_eq!(config.routes.len(), 1);
}

#[test]
fn route_matching_and_rewrite_work() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("config").join("dev.yaml");
    let config = AppConfig::load_from_file(path).expect("dev config should load");

    let route = match_route("/openai/v1/models", &config.routes).expect("route should match");
    let upstream_url =
        build_upstream_url_for_route(route, "/openai/v1/models", Some("limit=1")).unwrap();

    assert_eq!(upstream_url, "https://api.openai.com/v1/models?limit=1");
}

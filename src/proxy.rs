use crate::config::{HeaderInjection, RouteConfig, UpstreamConfig};
use http::header::HOST;
use http::{HeaderMap, HeaderName, HeaderValue};
use std::fmt;

pub const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub const FORWARDED_IP_HEADERS: [&str; 5] = [
    "x-forwarded-for",
    "forwarded",
    "cf-connecting-ip",
    "true-client-ip",
    "x-real-ip",
];

#[derive(Debug)]
pub enum ProxyError {
    InvalidHeaderName(String),
    InvalidHeaderValue(String),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeaderName(name) => write!(f, "invalid header name `{name}`"),
            Self::InvalidHeaderValue(value) => write!(f, "invalid header value `{value}`"),
        }
    }
}

impl std::error::Error for ProxyError {}

pub fn match_route<'a>(path: &str, routes: &'a [RouteConfig]) -> Option<&'a RouteConfig> {
    routes
        .iter()
        .filter(|route| path_matches_prefix(path, &route.prefix))
        .max_by_key(|route| route.prefix.len())
}

pub fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if !path.starts_with('/') || !prefix.starts_with('/') {
        return false;
    }

    if prefix == "/" {
        return true;
    }

    if !path.starts_with(prefix) {
        return false;
    }

    if path.len() == prefix.len() {
        return true;
    }

    path.as_bytes().get(prefix.len()) == Some(&b'/')
}

pub fn rewrite_path(path: &str, prefix: &str, strip_prefix: bool) -> String {
    if !strip_prefix {
        return normalize_path(path);
    }

    if prefix == "/" {
        return normalize_path(path);
    }

    if !path_matches_prefix(path, prefix) {
        return normalize_path(path);
    }

    let rest = &path[prefix.len()..];
    if rest.is_empty() {
        "/".to_string()
    } else {
        normalize_path(rest)
    }
}

pub fn build_upstream_url(base_url: &str, rest_path: &str, query: Option<&str>) -> String {
    let mut url = base_url.trim_end_matches('/').to_string();
    let rest_path = normalize_path(rest_path);

    if rest_path == "/" {
        url.push('/');
    } else {
        url.push_str(&rest_path);
    }

    if let Some(query) = query.filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(query);
    }

    url
}

pub fn build_upstream_url_for_route(
    route: &RouteConfig,
    request_path: &str,
    query: Option<&str>,
) -> Option<String> {
    if !path_matches_prefix(request_path, &route.prefix) {
        return None;
    }

    let rest = rewrite_path(request_path, &route.prefix, route.upstream.strip_prefix);
    Some(build_upstream_url(&route.upstream.base_url, &rest, query))
}

pub fn prepare_upstream_headers(
    inbound: &HeaderMap,
    upstream: &UpstreamConfig,
) -> Result<HeaderMap, ProxyError> {
    let mut outbound = inbound.clone();

    for name in HOP_BY_HOP_HEADERS {
        remove_header_case_insensitive(&mut outbound, name);
    }

    for name in &upstream.remove_headers {
        remove_header_case_insensitive(&mut outbound, name);
    }

    if !upstream.forward_xff {
        for name in FORWARDED_IP_HEADERS {
            remove_header_case_insensitive(&mut outbound, name);
        }
    }

    for header in &upstream.inject_headers {
        upsert_header(&mut outbound, header)?;
    }

    outbound.remove(HOST);
    Ok(outbound)
}

pub fn sanitize_response_headers(upstream_headers: &HeaderMap) -> HeaderMap {
    let mut headers = upstream_headers.clone();
    for name in HOP_BY_HOP_HEADERS {
        remove_header_case_insensitive(&mut headers, name);
    }
    headers
}

fn upsert_header(headers: &mut HeaderMap, injection: &HeaderInjection) -> Result<(), ProxyError> {
    let name = HeaderName::from_bytes(injection.name.as_bytes())
        .map_err(|_| ProxyError::InvalidHeaderName(injection.name.clone()))?;
    let value = HeaderValue::from_str(&injection.value)
        .map_err(|_| ProxyError::InvalidHeaderValue(injection.value.clone()))?;

    headers.insert(name, value);
    Ok(())
}

fn remove_header_case_insensitive(headers: &mut HeaderMap, header_name: &str) {
    if let Ok(name) = HeaderName::from_bytes(header_name.as_bytes()) {
        headers.remove(name);
    }
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }

    if path.starts_with('/') {
        return path.to_string();
    }

    format!("/{path}")
}

#[cfg(test)]
mod tests {
    use super::{
        build_upstream_url_for_route, match_route, prepare_upstream_headers, rewrite_path,
        sanitize_response_headers,
    };
    use crate::config::{HeaderInjection, RouteConfig, UpstreamConfig};
    use http::{HeaderMap, HeaderValue};

    #[test]
    fn match_longest_prefix() {
        let routes = vec![
            RouteConfig {
                id: "root".to_string(),
                prefix: "/openai".to_string(),
                upstream: minimal_upstream(),
            },
            RouteConfig {
                id: "nested".to_string(),
                prefix: "/openai/v1".to_string(),
                upstream: minimal_upstream(),
            },
        ];

        let route = match_route("/openai/v1/models", &routes).expect("route should match");
        assert_eq!(route.id, "nested");
    }

    #[test]
    fn prevent_prefix_boundary_false_match() {
        let route = RouteConfig {
            id: "openai".to_string(),
            prefix: "/openai".to_string(),
            upstream: minimal_upstream(),
        };

        assert!(build_upstream_url_for_route(&route, "/openai/v1/models", None).is_some());
        assert!(build_upstream_url_for_route(&route, "/openai2/v1/models", None).is_none());
    }

    #[test]
    fn rewrite_path_root_when_exact_prefix() {
        let path = rewrite_path("/openai", "/openai", true);
        assert_eq!(path, "/");
    }

    #[test]
    fn remove_hop_and_override_injected_headers() {
        let mut inbound = HeaderMap::new();
        inbound.insert("connection", HeaderValue::from_static("keep-alive"));
        inbound.insert(
            "authorization",
            HeaderValue::from_static("Bearer from-client"),
        );
        inbound.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));

        let upstream = UpstreamConfig {
            base_url: "https://api.openai.com".to_string(),
            strip_prefix: true,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            inject_headers: vec![HeaderInjection {
                name: "authorization".to_string(),
                value: "Bearer injected".to_string(),
            }],
            remove_headers: vec!["authorization".to_string()],
            forward_xff: false,
        };

        let outbound = prepare_upstream_headers(&inbound, &upstream).expect("headers are valid");
        assert!(!outbound.contains_key("connection"));
        assert!(!outbound.contains_key("x-forwarded-for"));
        assert_eq!(
            outbound.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer injected")
        );
    }

    #[test]
    fn remove_hop_by_hop_from_response_headers() {
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert("connection", HeaderValue::from_static("keep-alive"));
        upstream_headers.insert("upgrade", HeaderValue::from_static("websocket"));
        upstream_headers.insert("x-upstream-ok", HeaderValue::from_static("1"));

        let sanitized = sanitize_response_headers(&upstream_headers);
        assert!(!sanitized.contains_key("connection"));
        assert!(!sanitized.contains_key("upgrade"));
        assert_eq!(
            sanitized.get("x-upstream-ok").and_then(|v| v.to_str().ok()),
            Some("1")
        );
    }

    fn minimal_upstream() -> UpstreamConfig {
        UpstreamConfig {
            base_url: "https://api.openai.com".to_string(),
            strip_prefix: true,
            connect_timeout_ms: 10_000,
            request_timeout_ms: 60_000,
            inject_headers: Vec::new(),
            remove_headers: Vec::new(),
            forward_xff: false,
        }
    }
}

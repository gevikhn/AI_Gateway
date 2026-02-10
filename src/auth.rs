use crate::config::{GatewayAuthConfig, TokenSourceConfig};
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderName};

pub fn extract_token(headers: &HeaderMap, token_sources: &[TokenSourceConfig]) -> Option<String> {
    for source in token_sources {
        match source {
            TokenSourceConfig::AuthorizationBearer => {
                if let Some(value) = headers.get(AUTHORIZATION)
                    && let Ok(text) = value.to_str()
                    && let Some(token) = parse_bearer_token(text)
                {
                    return Some(token.to_string());
                }
            }
            TokenSourceConfig::Header { name } => {
                if let Ok(header_name) = HeaderName::from_bytes(name.as_bytes())
                    && let Some(value) = headers.get(header_name)
                    && let Ok(text) = value.to_str()
                    && !text.trim().is_empty()
                {
                    return Some(text.trim().to_string());
                }
            }
        }
    }

    None
}

pub fn is_authorized(headers: &HeaderMap, gateway_auth: &GatewayAuthConfig) -> bool {
    let Some(token) = extract_token(headers, &gateway_auth.token_sources) else {
        return false;
    };

    gateway_auth.tokens.iter().any(|allowed| allowed == &token)
}

fn parse_bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.trim().split_once(' ')?;

    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    Some(token)
}

#[cfg(test)]
mod tests {
    use super::{extract_token, is_authorized};
    use crate::config::{GatewayAuthConfig, TokenSourceConfig};
    use http::header::AUTHORIZATION;
    use http::{HeaderMap, HeaderValue};

    #[test]
    fn extract_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer gw_token"));

        let token = extract_token(&headers, &[TokenSourceConfig::AuthorizationBearer]);
        assert_eq!(token.as_deref(), Some("gw_token"));
    }

    #[test]
    fn fallback_to_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-gw-token", HeaderValue::from_static("fallback_token"));

        let token = extract_token(
            &headers,
            &[
                TokenSourceConfig::AuthorizationBearer,
                TokenSourceConfig::Header {
                    name: "x-gw-token".to_string(),
                },
            ],
        );

        assert_eq!(token.as_deref(), Some("fallback_token"));
    }

    #[test]
    fn authorize_with_allowlist_token() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer gw_token"));

        let auth = GatewayAuthConfig {
            tokens: vec!["gw_token".to_string()],
            token_sources: vec![TokenSourceConfig::AuthorizationBearer],
        };

        assert!(is_authorized(&headers, &auth));
    }
}

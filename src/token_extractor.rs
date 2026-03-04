use axum::body::Bytes;
use serde::Deserialize;

/// Token使用信息
#[derive(Debug, Clone, Copy)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// OpenAI格式的usage字段
#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

/// Claude格式的usage字段
#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

pub struct TokenExtractor;

impl TokenExtractor {
    /// 从非流式响应体中提取token使用信息
    pub fn extract_from_body(body: &Bytes) -> Option<TokenUsage> {
        // 如果body为空，直接返回
        if body.is_empty() {
            tracing::warn!("Response body is empty, cannot extract token usage");
            return None;
        }

        tracing::info!("Extracting tokens from body of {} bytes", body.len());

        // 尝试解析为JSON
        let json_value: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => {
                let preview = String::from_utf8_lossy(&body[..body.len().min(100)]);
                tracing::warn!("Failed to parse response body as JSON: {}, preview: {}", e, preview);
                return None;
            }
        };

        // 检查是否有usage字段
        let usage = match json_value.get("usage") {
            Some(u) => u,
            None => {
                tracing::warn!("No 'usage' field found in response body, keys: {:?}", json_value.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                return None;
            }
        };

        tracing::info!("Found usage field: {:?}", usage);

        // 尝试OpenAI格式
        if let Ok(openai_usage) = serde_json::from_value::<OpenAiUsage>(usage.clone()) {
            tracing::info!(
                "Extracted OpenAI format tokens: prompt={}, completion={}",
                openai_usage.prompt_tokens,
                openai_usage.completion_tokens
            );
            return Some(TokenUsage {
                input_tokens: openai_usage.prompt_tokens,
                output_tokens: openai_usage.completion_tokens,
                total_tokens: openai_usage.total_tokens,
            });
        }

        // 尝试Claude格式
        if let Ok(claude_usage) = serde_json::from_value::<ClaudeUsage>(usage.clone()) {
            tracing::info!(
                "Extracted Claude format tokens: input={}, output={}",
                claude_usage.input_tokens,
                claude_usage.output_tokens
            );
            return Some(TokenUsage {
                input_tokens: claude_usage.input_tokens,
                output_tokens: claude_usage.output_tokens,
                total_tokens: claude_usage.input_tokens + claude_usage.output_tokens,
            });
        }

        // 尝试通用格式（直接提取字段）
        let input_tokens = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let output_tokens = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let total_tokens = usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(input_tokens + output_tokens);

        if input_tokens > 0 || output_tokens > 0 {
            Some(TokenUsage {
                input_tokens,
                output_tokens,
                total_tokens,
            })
        } else {
            None
        }
    }

    /// 从累积的SSE chunks中提取token使用信息
    /// SSE流的最后一条消息通常包含usage信息
    pub fn extract_from_sse_body(full_body: &Bytes) -> Option<TokenUsage> {
        let text = String::from_utf8_lossy(full_body);

        tracing::info!("Extracting from SSE body of {} bytes", full_body.len());

        // 按行分割，收集所有 data: 行
        let mut data_lines: Vec<&str> = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim();
            // 支持两种格式：
            // 1. OpenAI: "data: {...}"
            // 2. Kimi: "data:{...}" (无空格)
            if (trimmed.starts_with("data: ") || trimmed.starts_with("data:")) && !trimmed.contains("[DONE]") {
                data_lines.push(trimmed);
            }
        }

        tracing::info!("Found {} data lines in SSE body", data_lines.len());

        if data_lines.is_empty() {
            tracing::warn!("No data: line found in SSE body");
            return None;
        }

        // 先尝试从所有 data 行中找到包含 usage 字段的那一行
        // 这对于 Claude/Kimi API 很重要，因为 usage 可能在 message_delta 事件中
        let mut usage_line: Option<&str> = None;
        for line in &data_lines {
            if let Some(json_str) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str.trim()) {
                    if json.get("usage").is_some() {
                        usage_line = Some(line);
                        tracing::info!("Found data line with usage field");
                        break;
                    }
                }
            }
        }

        // 如果没找到包含 usage 的行，使用最后一行（OpenAI 格式）
        let data_line = usage_line.or_else(|| data_lines.last().copied()).unwrap();
        tracing::info!("Using data line: {}", data_line.chars().take(100).collect::<String>());

        // 支持 "data: " (带空格) 和 "data:" (无空格) 两种前缀
        let json_str = if let Some(s) = data_line.strip_prefix("data: ") {
            s.trim()
        } else if let Some(s) = data_line.strip_prefix("data:") {
            s.trim()
        } else {
            tracing::warn!("Failed to strip 'data:' prefix");
            return None;
        };

        tracing::info!("JSON string to parse: {}", json_str.chars().take(200).collect::<String>());

        // 解析JSON
        let json_value: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse SSE JSON: {}", e);
                return None;
            }
        };

        // 查找usage字段（可能在不同位置）
        // OpenAI: 在根对象的usage字段
        // Claude: 也在根对象的usage字段
        tracing::info!("Looking for usage field in JSON, keys: {:?}", json_value.as_object().map(|o| o.keys().collect::<Vec<_>>()));

        if let Some(usage) = json_value.get("usage") {
            tracing::info!("Found usage field: {:?}", usage);
            // 尝试OpenAI格式
            if let Ok(openai_usage) = serde_json::from_value::<OpenAiUsage>(usage.clone()) {
                if openai_usage.prompt_tokens > 0 || openai_usage.completion_tokens > 0 {
                    tracing::info!("Extracted OpenAI format from SSE: prompt={}, completion={}", openai_usage.prompt_tokens, openai_usage.completion_tokens);
                    return Some(TokenUsage {
                        input_tokens: openai_usage.prompt_tokens,
                        output_tokens: openai_usage.completion_tokens,
                        total_tokens: openai_usage.total_tokens,
                    });
                }
            }

            // 尝试Claude格式
            if let Ok(claude_usage) = serde_json::from_value::<ClaudeUsage>(usage.clone()) {
                if claude_usage.input_tokens > 0 || claude_usage.output_tokens > 0 {
                    tracing::info!("Extracted Claude format from SSE: input={}, output={}", claude_usage.input_tokens, claude_usage.output_tokens);
                    return Some(TokenUsage {
                        input_tokens: claude_usage.input_tokens,
                        output_tokens: claude_usage.output_tokens,
                        total_tokens: claude_usage.input_tokens + claude_usage.output_tokens,
                    });
                }
            }
        } else {
            tracing::warn!("No 'usage' field found in SSE JSON");
        }

        // 有些SSE流可能将usage放在不同的嵌套位置
        // 尝试在message或delta中查找
        if let Some(message) = json_value.get("message") {
            if let Some(usage) = message.get("usage") {
                if let Ok(claude_usage) = serde_json::from_value::<ClaudeUsage>(usage.clone()) {
                    if claude_usage.input_tokens > 0 || claude_usage.output_tokens > 0 {
                        return Some(TokenUsage {
                            input_tokens: claude_usage.input_tokens,
                            output_tokens: claude_usage.output_tokens,
                            total_tokens: claude_usage.input_tokens + claude_usage.output_tokens,
                        });
                    }
                }
            }
        }

        None
    }

    /// 从流式SSE chunk中提取token使用信息
    /// 注意：大多数SSE流只在最后一条消息包含usage
    pub fn extract_from_sse_chunk(chunk: &Bytes) -> Option<TokenUsage> {
        let text = String::from_utf8_lossy(chunk);

        for line in text.lines() {
            let trimmed = line.trim();
            // 支持 "data: " (带空格) 和 "data:" (无空格) 两种格式
            if (trimmed.starts_with("data: ") || trimmed.starts_with("data:")) && !trimmed.contains("[DONE]") {
                let json_str = if let Some(s) = trimmed.strip_prefix("data: ") {
                    s.trim()
                } else {
                    trimmed.strip_prefix("data:")?.trim()
                };
                let json_value: serde_json::Value = serde_json::from_str(json_str).ok()?;

                // 查找usage字段
                if let Some(usage) = json_value.get("usage") {
                    // 尝试OpenAI格式
                    if let Ok(openai_usage) = serde_json::from_value::<OpenAiUsage>(usage.clone()) {
                        if openai_usage.prompt_tokens > 0 || openai_usage.completion_tokens > 0 {
                            return Some(TokenUsage {
                                input_tokens: openai_usage.prompt_tokens,
                                output_tokens: openai_usage.completion_tokens,
                                total_tokens: openai_usage.total_tokens,
                            });
                        }
                    }

                    // 尝试Claude格式
                    if let Ok(claude_usage) = serde_json::from_value::<ClaudeUsage>(usage.clone()) {
                        if claude_usage.input_tokens > 0 || claude_usage.output_tokens > 0 {
                            return Some(TokenUsage {
                                input_tokens: claude_usage.input_tokens,
                                output_tokens: claude_usage.output_tokens,
                                total_tokens: claude_usage.input_tokens + claude_usage.output_tokens,
                            });
                        }
                    }
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_openai_format() {
        let json = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1677652288,
            "choices": [],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        }"#;

        let body = Bytes::from(json);
        let usage = TokenExtractor::extract_from_body(&body).unwrap();

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn test_extract_claude_format() {
        let json = r#"{
            "id": "msg_01XgYJDaXz5f5vD",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": "claude-3-opus-20240229",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 200,
                "output_tokens": 100
            }
        }"#;

        let body = Bytes::from(json);
        let usage = TokenExtractor::extract_from_body(&body).unwrap();

        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 100);
        assert_eq!(usage.total_tokens, 300);
    }

    #[test]
    fn test_extract_from_sse_openai() {
        // OpenAI SSE格式：最后一条消息包含usage
        let sse = r#"
data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1677652288,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1677652288,"model":"gpt-3.5-turbo","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}

data: [DONE]
"#;

        let body = Bytes::from(sse);
        let usage = TokenExtractor::extract_from_sse_body(&body).unwrap();

        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn test_no_usage_field() {
        let json = r#"{"id": "test", "choices": []}"#;
        let body = Bytes::from(json);
        let usage = TokenExtractor::extract_from_body(&body);
        assert!(usage.is_none());
    }

    #[test]
    fn test_invalid_json() {
        let body = Bytes::from("not valid json");
        let usage = TokenExtractor::extract_from_body(&body);
        assert!(usage.is_none());
    }
}

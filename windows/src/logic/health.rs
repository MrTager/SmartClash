use super::model::{Health, ProbeResult};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub const DELAY_TARGET: &str = "https://cp.cloudflare.com/generate_204";

/// 与 macOS 版一致：只用字母数字保留，其余全部百分号编码。
pub fn percent_encode_alphanumeric(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

pub fn delay_path(name: &str, timeout_ms: u64) -> String {
    format!(
        "/proxies/{}/delay?url={}&timeout={}&expected=204",
        percent_encode_alphanumeric(name),
        percent_encode_alphanumeric(DELAY_TARGET),
        timeout_ms
    )
}

pub struct HealthPolicy;

impl HealthPolicy {
    pub fn classify(code: i64, body: &str, elapsed: i64) -> ProbeResult {
        let lower = body.to_lowercase();
        let blocked_markers = [
            "unsupported_country",
            "unsupported country",
            "region is not supported",
            "country is not supported",
            "country, region, or territory not supported",
            "not available in your country",
            "not supported in your region",
        ];
        let parsed: Option<Value> = serde_json::from_str(body).ok();

        let health = if blocked_markers.iter().any(|m| lower.contains(m)) {
            Health::Blocked
        } else if code == 429 {
            Health::Limited
        } else if (200..300).contains(&code) {
            match &parsed {
                Some(v)
                    if v.get("object").and_then(|o| o.as_str()) == Some("list")
                        && v.get("data").map(|d| d.is_array()).unwrap_or(false) =>
                {
                    Health::Healthy
                }
                _ => Health::Unknown,
            }
        } else if code == 401 {
            // 没有 API Key 时，只有结构化的鉴权错误才能证明 API 端点可达。
            let message = parsed
                .as_ref()
                .and_then(|v| v.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_lowercase();
            if message.contains("api key") || message.contains("authentication") {
                Health::Healthy
            } else {
                Health::Unknown
            }
        } else if code == 0 || code >= 500 {
            Health::Offline
        } else {
            Health::Unknown
        };

        ProbeResult::new(
            health,
            elapsed,
            format!("HTTP {} · {}", code, health.label()),
        )
    }

    pub fn should_failover(failures: u32, last_switch: u64, now: u64, threshold: u32) -> bool {
        failures >= threshold && now.saturating_sub(last_switch) >= 30_000
    }
}

pub struct InternetPolicy;

impl InternetPolicy {
    pub fn classify(code: i64, elapsed: i64) -> ProbeResult {
        ProbeResult::new(
            if code == 204 { Health::Healthy } else { Health::Offline },
            elapsed,
            if code == 204 {
                "HTTPS 出网正常".to_string()
            } else {
                format!("HTTPS 出网未通过（HTTP {code}）")
            },
        )
    }
}

pub struct StandbyPolicy;

impl StandbyPolicy {
    pub fn usable(result: &ProbeResult, checked: u64, now: u64, ttl_ms: u64) -> bool {
        result.health == Health::Healthy && now >= checked && now - checked <= ttl_ms
    }
}

#[cfg(test)]
mod tests {
    use super::HealthPolicy;
    use crate::logic::model::Health;

    #[test]
    fn openai_region_error_is_blocked_even_without_error_code() {
        let body = r#"{"error":{"message":"Country, region, or territory not supported"}}"#;
        let result = HealthPolicy::classify(403, body, 120);
        assert_eq!(result.health, Health::Blocked);
    }

    #[test]
    fn openai_region_error_with_code_is_blocked() {
        let body = r#"{"error":{"message":"Country, region, or territory not supported","code":"unsupported_country"}}"#;
        let result = HealthPolicy::classify(403, body, 120);
        assert_eq!(result.health, Health::Blocked);
    }

    #[test]
    fn missing_api_key_still_proves_endpoint_reachable() {
        let body = r#"{"error":{"message":"You didn't provide an API key","type":"invalid_request_error"}}"#;
        let result = HealthPolicy::classify(401, body, 90);
        assert_eq!(result.health, Health::Healthy);
    }

    #[test]
    fn rate_limit_is_not_a_failure() {
        let result = HealthPolicy::classify(429, r#"{"error":{"message":"rate limit"}}"#, 50);
        assert_eq!(result.health, Health::Limited);
    }
}

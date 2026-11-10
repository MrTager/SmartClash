use super::model::{ProxyNode, Source};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const MAX_BYTES: usize = 10_000_000;
const MAX_NODES: usize = 5000;

const SUPPORTED: [&str; 12] = [
    "ss",
    "ssr",
    "vmess",
    "vless",
    "trojan",
    "hysteria",
    "hysteria2",
    "tuic",
    "socks5",
    "http",
    "snell",
    "anytls",
];

pub struct ParseOutcome {
    pub nodes: Vec<ProxyNode>,
    pub skipped: usize,
}

/// 只导入节点定义；远程规则、控制器设置与脚本全部丢弃。
pub fn parse(text: &str, source: &Source) -> Result<Vec<ProxyNode>, String> {
    parse_detailed(text, source).map(|outcome| outcome.nodes)
}

/// 与 `parse` 相同，但保留被跳过节点的数量。
/// 单个节点不受支持或格式异常时只跳过该节点，不再拒绝整份订阅。
pub fn parse_detailed(text: &str, source: &Source) -> Result<ParseOutcome, String> {
    if text.len() > MAX_BYTES {
        return Err("订阅超过 10 MB".into());
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|_| "订阅 YAML 语法无效，已隐藏可能包含凭据的解析详情")?;
    let json: Value = serde_json::to_value(&yaml).map_err(|_| "订阅格式无法转换为节点列表")?;
    let doc = json
        .as_object()
        .ok_or("需要含 proxies 的 Clash YAML/JSON 订阅（最多 5000 个节点）；不支持 Base64 URI 列表或仅有 provider 的配置")?;
    let list = doc
        .get("proxies")
        .and_then(|v| v.as_array())
        .ok_or("需要含 proxies 的 Clash YAML/JSON 订阅（最多 5000 个节点）；不支持 Base64 URI 列表或仅有 provider 的配置")?;
    if list.is_empty() || list.len() > MAX_NODES {
        return Err("需要含 proxies 的 Clash YAML/JSON 订阅（最多 5000 个节点）；不支持 Base64 URI 列表或仅有 provider 的配置".into());
    }

    let mut nodes = Vec::with_capacity(list.len());
    let mut skipped = 0usize;
    for item in list {
        let Some(input) = item.as_object() else {
            skipped += 1;
            continue;
        };
        let raw = match normalize_transport(input) {
            Ok(raw) => raw,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let node_type = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let server = raw.get("server").and_then(|v| v.as_str()).unwrap_or("");
        let port = raw.get("port").and_then(value_as_u64).unwrap_or(0);
        let original = raw.get("name").and_then(|v| v.as_str()).unwrap_or("");

        if !SUPPORTED.contains(&node_type)
            || server.is_empty()
            || !(1..=65535).contains(&port)
            || original.is_empty()
        {
            skipped += 1;
            continue;
        }

        let mut checked = raw.clone();
        if node_type == "ss" && raw.get("plugin").and_then(|v| v.as_str()) == Some("obfs") {
            let allowed: [&str; 2] = ["mode", "host"];
            let options_ok = raw
                .get("plugin-opts")
                .and_then(|v| v.as_object())
                .map(|options| {
                    options.keys().all(|k| allowed.contains(&k.as_str()))
                        && matches!(
                            options.get("mode").and_then(|m| m.as_str()).unwrap_or(""),
                            "http" | "tls"
                        )
                        && options.values().all(|v| v.is_string())
                })
                .unwrap_or(false);
            if options_ok {
                // Mihomo 内建该传输实现，不执行外部插件。
                checked.remove("plugin");
            }
        }
        if validate(&Value::Object(checked.clone())).is_err() {
            skipped += 1;
            continue;
        }

        let mut identity = raw.clone();
        identity.remove("name");
        let canonical = match serde_json::to_string(&Value::Object(identity.clone())) {
            Ok(canonical) => canonical,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let fingerprint = hex_sha256(canonical.as_bytes());

        let truncated: String = original.chars().take(70).collect();
        let name = format!("{} · {}", truncated, &fingerprint[..12]);
        let mut payload = raw.clone();
        payload.insert("name".into(), Value::String(name.clone()));

        nodes.push(ProxyNode {
            id: fingerprint,
            exit_name: name.clone(),
            name,
            source_id: source.id.clone(),
            payload,
        });
    }
    if nodes.is_empty() {
        return Err(format!(
            "订阅中没有可用节点（已跳过 {skipped} 个不支持或格式异常的节点）"
        ));
    }
    Ok(ParseOutcome { nodes, skipped })
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn normalize_transport(input: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if !input.contains_key("ws-path") && !input.contains_key("ws-headers") {
        return Ok(input.clone());
    }
    if input.get("type").and_then(|v| v.as_str()) != Some("vmess")
        || input.get("network").and_then(|v| v.as_str()) != Some("ws")
    {
        return Err("旧式 WebSocket 参数仅适用于 VMess WebSocket 节点".into());
    }
    let mut raw = input.clone();
    let mut options = raw
        .get("ws-opts")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(path) = raw.remove("ws-path") {
        if !path.is_string() {
            return Err("WebSocket 路径格式无效".into());
        }
        options.entry("path").or_insert(path);
    }
    if let Some(headers) = raw.remove("ws-headers") {
        if !headers.is_object()
            || headers
                .as_object()
                .map(|h| h.values().any(|v| !v.is_string()))
                .unwrap_or(true)
        {
            return Err("WebSocket 请求头格式无效".into());
        }
        options.entry("headers").or_insert(headers);
    }
    raw.insert("ws-opts".into(), Value::Object(options));
    Ok(raw)
}

fn validate(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(dict) => {
            for (key, item) in dict {
                let k = key.to_lowercase();
                let forbidden = [
                    "dialer-proxy",
                    "certificate",
                    "private-key",
                    "script",
                    "plugin",
                    "provider",
                ];
                if (k.contains("path") && k != "path") || forbidden.contains(&k.as_str()) {
                    return Err("不支持包含本地文件、插件或节点链引用的订阅".into());
                }
                // WebSocket / HTTP 传输的 path 是 URL 路径，不是本地文件引用。
                validate(item)?;
            }
        }
        Value::Array(list) => {
            for item in list {
                validate(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn merge(batches: Vec<Vec<ProxyNode>>) -> Vec<ProxyNode> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for node in batches.into_iter().flatten() {
        if seen.insert(node.id.clone()) {
            result.push(node);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            id: "test".into(),
            name: "test".into(),
            url: String::new(),
        }
    }

    #[test]
    fn skips_invalid_nodes_instead_of_rejecting_whole_source() {
        let text = r#"
proxies:
  - name: good
    type: ss
    server: example.com
    port: 443
    cipher: aes-128-gcm
    password: secret
  - name: bad
    type: unsupported-type
    server: example.com
    port: 443
"#;
        let outcome = parse_detailed(text, &source()).expect("应保留有效节点");
        assert_eq!(outcome.nodes.len(), 1);
        assert_eq!(outcome.skipped, 1);
    }
}

use super::model::{ProxyNode, TrafficMode, CONTROLLER_PORT, PROBE_PORT, PROXY_PORT};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// SmartClash 官方国内直连规则集（mihomo rule-provider）。
/// 仓库：https://github.com/MrTager/SmartClash-Rules
const RULES_REPO_BASE: &str =
    "https://raw.githubusercontent.com/MrTager/SmartClash-Rules/main/rules";
/// 规则文件本地缓存相对内核工作目录（-d）的位置。
const RULES_CACHE_DIR: &str = "./rules";
/// 规则集自动更新间隔：7 天（秒）。
const RULES_INTERVAL: u64 = 60 * 60 * 24 * 7;

/// 国内域名/IP 直连规则集：通过 mihomo rule-provider 订阅官方规则仓库，
/// 下载与更新走 SMART 组（可用节点），本地缓存于内核目录 rules/ 下。
fn china_rule_providers() -> Value {
    json!({
        "cn-domains": {
            "type": "http",
            "behavior": "domain",
            "format": "yaml",
            "url": format!("{RULES_REPO_BASE}/direct.yaml"),
            "path": format!("{RULES_CACHE_DIR}/cn-domains.yaml"),
            "interval": RULES_INTERVAL,
            "proxy": "SMART"
        },
        "cn-custom": {
            "type": "http",
            "behavior": "domain",
            "format": "yaml",
            "url": format!("{RULES_REPO_BASE}/custom-direct.yaml"),
            "path": format!("{RULES_CACHE_DIR}/cn-custom.yaml"),
            "interval": RULES_INTERVAL,
            "proxy": "SMART"
        },
        "cn-ips": {
            "type": "http",
            "behavior": "ipcidr",
            "format": "yaml",
            "url": format!("{RULES_REPO_BASE}/cncidr.yaml"),
            "path": format!("{RULES_CACHE_DIR}/cn-ips.yaml"),
            "interval": RULES_INTERVAL,
            "proxy": "SMART"
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub fn make(
    nodes: &[ProxyNode],
    secret: &str,
    tun: bool,
    routes: Option<&[ProxyNode]>,
    extra_proxies: &[ProxyNode],
    outbound_interface: Option<&str>,
    preferred_name: Option<&str>,
    traffic_mode: TrafficMode,
) -> Result<Vec<u8>, String> {
    if nodes.is_empty() && traffic_mode.uses_proxy() {
        return Err("请先添加并更新订阅".into());
    }
    let route_nodes = routes.unwrap_or(nodes);
    let mut names: Vec<String> = route_nodes.iter().map(|n| n.name.clone()).collect();
    if names.is_empty() && !traffic_mode.uses_proxy() {
        // 直连模式：没有订阅节点也能启动内核，出口固定为 DIRECT。
        names.push("DIRECT".into());
    }
    if names.is_empty() {
        return Err("没有可用链路".into());
    }
    if let Some(preferred) = preferred_name {
        if let Some(index) = names.iter().position(|n| n == preferred) {
            let name = names.remove(index);
            names.insert(0, name);
        }
    }

    let mut probe_names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names.iter().chain(nodes.iter().map(|n| &n.name)) {
        if seen.insert(name.clone()) {
            probe_names.push(name.clone());
        }
    }

    let proxy_source: &[ProxyNode] = if extra_proxies.is_empty() {
        route_nodes
    } else {
        extra_proxies
    };
    let mut proxies: BTreeMap<String, Value> = BTreeMap::new();
    for node in nodes.iter().chain(proxy_source.iter()) {
        proxies.insert(
            node.name.clone(),
            Value::Object(node.payload.clone()),
        );
    }
    let proxies: Vec<Value> = proxies.into_values().collect();

    let mut obj: Map<String, Value> = Map::new();
    obj.insert("mixed-port".into(), json!(PROXY_PORT));
    obj.insert("allow-lan".into(), json!(false));
    obj.insert("bind-address".into(), json!("127.0.0.1"));
    obj.insert("mode".into(), json!(traffic_mode.raw()));
    obj.insert("log-level".into(), json!("warning"));
    obj.insert("ipv6".into(), json!(false));
    obj.insert(
        "external-controller".into(),
        json!(format!("127.0.0.1:{CONTROLLER_PORT}")),
    );
    obj.insert("secret".into(), json!(secret));
    obj.insert("profile".into(), json!({ "store-selected": false }));
    obj.insert("find-process-mode".into(), json!("off"));
    obj.insert("proxies".into(), Value::Array(proxies));
    obj.insert(
        "proxy-groups".into(),
        json!([
            { "name": "SMART", "type": "select", "proxies": names },
            { "name": "PROBE", "type": "select", "proxies": probe_names },
            // 全局模式由 GLOBAL 组决定出口，保持与 SMART 同步。
            { "name": "GLOBAL", "type": "select", "proxies": ["SMART"] }
        ]),
    );
    // 独立探测监听端口，绝不改变用户正在使用的路由组。
    obj.insert(
        "listeners".into(),
        json!([{
            "name": "openai-probe",
            "type": "http",
            "port": PROBE_PORT,
            "listen": "127.0.0.1",
            "proxy": "PROBE"
        }]),
    );
    obj.insert(
        "tun".into(),
        json!({
            "enable": tun,
            "stack": "gvisor",
            "auto-route": true,
            "auto-detect-interface": outbound_interface.is_none(),
            "dns-hijack": ["any:53"]
        }),
    );
    obj.insert(
        "dns".into(),
        json!({
            "enable": true,
            "listen": "127.0.0.1:11053",
            "ipv6": false,
            "enhanced-mode": "fake-ip",
            "fake-ip-range": "198.18.0.1/16",
            "fake-ip-filter": ["*.lan", "*.local", "localhost"],
            "default-nameserver": ["223.5.5.5", "119.29.29.29"],
            "proxy-server-nameserver": ["223.5.5.5", "tcp://223.5.5.5", "119.29.29.29"],
            "nameserver": ["https://dns.alidns.com/dns-query", "https://cloudflare-dns.com/dns-query"]
        }),
    );
    obj.insert("rule-providers".into(), china_rule_providers());
    obj.insert(
        "rules".into(),
        json!([
            "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
            "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
            "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
            // 国内域名/IP 直连，避免国内站点绕道境外节点。
            "RULE-SET,cn-domains,DIRECT",
            "RULE-SET,cn-custom,DIRECT",
            "RULE-SET,cn-ips,DIRECT",
            "MATCH,SMART"
        ]),
    );
    if let Some(interface) = outbound_interface {
        obj.insert("interface-name".into(), json!(interface));
    }

    serde_json::to_vec_pretty(&Value::Object(obj)).map_err(|e| format!("生成配置失败：{e}"))
}

/// JSON 是 YAML 1.2 的合法子集，Clash/Mihomo 客户端可直接读取；导出内容不含本地控制密钥。
pub fn export(nodes: &[ProxyNode]) -> Result<Vec<u8>, String> {
    let obj = json!({
        "mixed-port": 7890,
        "allow-lan": false,
        "mode": "rule",
        "proxies": nodes.iter().map(|n| Value::Object(n.payload.clone())).collect::<Vec<_>>(),
        "proxy-groups": [{
            "name": "聚合节点",
            "type": "select",
            "proxies": nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
        }],
        "rules": ["MATCH,聚合节点"]
    });
    serde_json::to_vec_pretty(&obj).map_err(|e| format!("导出失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::model::Source;
    use crate::logic::subscription::parse;
    use std::process::Command;

    #[test]
    fn generated_config_passes_mihomo_check() {
        let yaml = r#"
proxies:
  - name: TestSS
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: secret
"#;
        let source = Source {
            id: "test".into(),
            name: "test".into(),
            url: "https://example.com/sub".into(),
        };
        let nodes = parse(yaml, &source).expect("parse sample subscription");
        assert_eq!(nodes.len(), 1);
        let config = make(
            &nodes,
            "secret",
            false,
            None,
            &[],
            None,
            None,
            TrafficMode::Rule,
        )
        .expect("build config");

        check_with_core(&config, "rule");
        let value: Value = serde_json::from_slice(&config).expect("config is json");
        assert_eq!(value["mode"], "rule");
        let groups = value["proxy-groups"].as_array().expect("proxy-groups");
        assert!(groups.iter().any(|g| g["name"] == "GLOBAL" && g["proxies"][0] == "SMART"));

        // 国内直连规则集：订阅 SmartClash-Rules 仓库并置于 MATCH 之前。
        let providers = value["rule-providers"].as_object().expect("rule-providers");
        for name in ["cn-domains", "cn-custom", "cn-ips"] {
            let provider = providers.get(name).unwrap_or_else(|| panic!("provider {name}"));
            let filename = match name {
                "cn-domains" => "direct.yaml",
                "cn-custom" => "custom-direct.yaml",
                "cn-ips" => "cncidr.yaml",
                _ => unreachable!(),
            };
            assert_eq!(provider["url"], format!("https://raw.githubusercontent.com/MrTager/SmartClash-Rules/main/rules/{filename}"));
            assert_eq!(provider["proxy"], "SMART");
        }
        let rules = value["rules"].as_array().expect("rules");
        let match_index = rules
            .iter()
            .position(|r| r == "MATCH,SMART")
            .expect("MATCH rule");
        for rule in ["RULE-SET,cn-domains,DIRECT", "RULE-SET,cn-custom,DIRECT", "RULE-SET,cn-ips,DIRECT"] {
            let index = rules.iter().position(|r| r == rule).unwrap_or_else(|| panic!("rule {rule}"));
            assert!(index < match_index, "{rule} must precede MATCH");
        }
    }

    #[test]
    fn every_traffic_mode_passes_mihomo_check() {
        let yaml = r#"
proxies:
  - name: TestSS
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: secret
"#;
        let source = Source {
            id: "test".into(),
            name: "test".into(),
            url: "https://example.com/sub".into(),
        };
        let nodes = parse(yaml, &source).expect("parse sample subscription");
        for mode in TrafficMode::ALL {
            let config = make(&nodes, "secret", false, None, &[], None, None, mode)
                .unwrap_or_else(|e| panic!("build {mode:?} config: {e}"));
            check_with_core(&config, mode.raw());
            let value: Value = serde_json::from_slice(&config).expect("config is json");
            assert_eq!(value["mode"], mode.raw());
        }
    }

    /// 直连模式必须能在没有任何订阅节点时启动内核。
    #[test]
    fn direct_mode_without_nodes_passes_mihomo_check() {
        let config = make(&[], "secret", false, None, &[], None, None, TrafficMode::Direct)
            .expect("build node-less direct config");
        check_with_core(&config, "direct");
        let value: Value = serde_json::from_slice(&config).expect("config is json");
        assert_eq!(value["mode"], "direct");
        let groups = value["proxy-groups"].as_array().expect("proxy-groups");
        let smart = groups
            .iter()
            .find(|g| g["name"] == "SMART")
            .expect("SMART group");
        assert_eq!(smart["proxies"][0], "DIRECT");
        assert!(
            make(&[], "secret", false, None, &[], None, None, TrafficMode::Global).is_err(),
            "global mode still requires at least one node"
        );
    }

    fn check_with_core(config: &[u8], label: &str) {
        // 同一进程内多个测试会并行调用这里，label 可能重复（如 direct），
        // 因此再加一个自增序号，避免共用临时目录互相删目录。
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "smartclash-config-test-{}-{label}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.json");
        std::fs::write(&path, config).expect("write config");

        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/mihomo.exe");
        let output = Command::new(core)
            .args(["-t", "-d", dir.to_str().unwrap(), "-f", path.to_str().unwrap()])
            .output()
            .expect("run mihomo -t");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "mihomo rejected {label} config: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

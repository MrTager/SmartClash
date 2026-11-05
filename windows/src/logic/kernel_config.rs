//! sing-box / Xray 配置生成。Mihomo 配置仍由 `configuration.rs` 负责。

use super::configuration::BuiltConfig;
use super::convert;
use super::kernel::{KernelKind, XRAY_API_PORT, XRAY_SOCKS_PORT};
use super::model::{ProxyNode, TrafficMode, PROBE_PORT, proxy_port};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// 规则集下载镜像。raw.githubusercontent.com 在部分网络下不可达，
/// 而 sing-box 首次启动时规则集下载失败会直接退出，因此默认走国内可用的加速镜像。
const SINGBOX_RULESET_MIRROR: &str = "https://ghfast.top/";

fn unique_supported<'a>(kind: KernelKind, list: &'a [ProxyNode]) -> (Vec<&'a ProxyNode>, usize) {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    let mut skipped = 0usize;
    for node in list {
        if !seen.insert(node.name.clone()) {
            continue;
        }
        if kind.supports_node(node) {
            kept.push(node);
        } else {
            skipped += 1;
        }
    }
    (kept, skipped)
}

fn warning_for(kind: KernelKind, nodes: &[ProxyNode]) -> Option<String> {
    let skipped = nodes.iter().filter(|node| !kind.supports_node(node)).count();
    if skipped == 0 {
        None
    } else {
        Some(format!(
            "当前内核（{}）不支持 {skipped} 个节点协议，已从配置中跳过",
            kind.title()
        ))
    }
}

pub fn build_singbox(
    nodes: &[ProxyNode],
    secret: &str,
    tun: bool,
    routes: Option<&[ProxyNode]>,
    extra_proxies: &[ProxyNode],
    outbound_interface: Option<&str>,
    preferred_name: Option<&str>,
    traffic_mode: TrafficMode,
) -> Result<BuiltConfig, String> {
    if !traffic_mode.uses_proxy() && nodes.is_empty() {
        // 直连模式允许没有订阅。
    }
    let route_nodes = routes.unwrap_or(nodes);
    let (mut route_kept, _) = unique_supported(KernelKind::SingBox, route_nodes);
    if let Some(preferred) = preferred_name {
        if let Some(index) = route_kept.iter().position(|node| node.name == preferred) {
            let node = route_kept.remove(index);
            route_kept.insert(0, node);
        }
    }
    if route_kept.is_empty() && traffic_mode.uses_proxy() {
        return Err("当前内核不支持订阅中的节点协议，请更换订阅或内核".into());
    }

    let mut warnings = Vec::new();
    if let Some(warning) = warning_for(KernelKind::SingBox, nodes) {
        warnings.push(warning);
    }

    // 节点出站：订阅节点 + 链路定义节点。
    let mut outbounds: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    for node in nodes.iter().chain(extra_proxies.iter()) {
        if !seen.insert(node.name.clone()) {
            continue;
        }
        if !KernelKind::SingBox.supports_node(node) {
            continue;
        }
        match convert::to_singbox(node) {
            Ok(value) => outbounds.push(value),
            Err(error) => warnings.push(format!("{}：{error}", node.short_name())),
        }
    }

    let names: Vec<String> = route_kept.iter().map(|node| node.name.clone()).collect();
    let mut probe_names: Vec<String> = names.clone();
    for node in nodes.iter().chain(extra_proxies.iter()) {
        if KernelKind::SingBox.supports_node(node) && !probe_names.contains(&node.name) {
            probe_names.push(node.name.clone());
        }
    }

    let smart: Vec<String> = if names.is_empty() {
        vec!["direct".into()]
    } else {
        names.clone()
    };
    let probe: Vec<String> = if probe_names.is_empty() {
        vec!["direct".into()]
    } else {
        probe_names
    };
    outbounds.push(json!({ "type": "selector", "tag": "SMART", "outbounds": smart }));
    outbounds.push(json!({ "type": "selector", "tag": "PROBE", "outbounds": probe }));
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));
    outbounds.push(json!({ "type": "block", "tag": "block" }));

    let mut inbounds = vec![
        json!({
            "type": "mixed",
            "tag": "mixed-in",
            "listen": "127.0.0.1",
            "listen_port": proxy_port(),
            "sniff": true
        }),
        json!({
            "type": "http",
            "tag": "probe-in",
            "listen": "127.0.0.1",
            "listen_port": PROBE_PORT
        }),
    ];
    if tun {
        inbounds.push(json!({
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30"],
            "auto_route": true,
            "strict_route": false,
            "stack": "gvisor",
            "sniff": true
        }));
    }

    let rule_mode = traffic_mode == TrafficMode::Rule;
    let mut rules = vec![json!({ "inbound": ["probe-in"], "outbound": "PROBE" })];
    if rule_mode {
        rules.push(json!({ "ip_is_private": true, "outbound": "direct" }));
        rules.push(json!({ "rule_set": ["cn-domains"], "outbound": "direct" }));
        rules.push(json!({ "rule_set": ["cn-ips"], "outbound": "direct" }));
    }
    let final_outbound = if traffic_mode.uses_proxy() {
        "SMART"
    } else {
        "direct"
    };

    let mut route = Map::new();
    route.insert(
        "auto_detect_interface".into(),
        json!(outbound_interface.is_none()),
    );
    if let Some(interface) = outbound_interface {
        route.insert("default_interface".into(), json!(interface));
    }
    route.insert("default_domain_resolver".into(), json!("dns-direct"));
    route.insert("rules".into(), Value::Array(rules));
    route.insert("final".into(), json!(final_outbound));
    if rule_mode {
        // 官方 sing-box 规则集：由内核按小时自行刷新。首次启动必须下载成功，
        // 下载失败会直接 FATAL，因此走 GitHub 镜像并直连，不依赖当前节点是否可用。
        route.insert(
            "rule_set".into(),
            json!([
                {
                    "type": "remote",
                    "tag": "cn-domains",
                    "format": "binary",
                    "url": format!("{SINGBOX_RULESET_MIRROR}https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs"),
                    "download_detour": "direct",
                    "update_interval": "1h"
                },
                {
                    "type": "remote",
                    "tag": "cn-ips",
                    "format": "binary",
                    "url": format!("{SINGBOX_RULESET_MIRROR}https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs"),
                    "download_detour": "direct",
                    "update_interval": "1h"
                }
            ]),
        );
    }

    let mut dns = Map::new();
    dns.insert(
        "servers".into(),
        json!([
            { "tag": "dns-proxy", "address": "https://1.1.1.1/dns-query", "detour": if traffic_mode.uses_proxy() { "SMART" } else { "direct" } },
            { "tag": "dns-direct", "address": "223.5.5.5", "detour": "direct" }
        ]),
    );
    let mut dns_rules = vec![json!({ "ip_is_private": true, "server": "dns-direct" })];
    if rule_mode {
        dns_rules.push(json!({ "rule_set": ["cn-domains"], "server": "dns-direct" }));
    }
    dns.insert("rules".into(), Value::Array(dns_rules));
    dns.insert(
        "final".into(),
        json!(if traffic_mode.uses_proxy() {
            "dns-proxy"
        } else {
            "dns-direct"
        }),
    );
    dns.insert("strategy".into(), json!("ipv4_only"));

    let config = json!({
        "log": { "level": "warning", "timestamp": true },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": Value::Object(route),
        "dns": Value::Object(dns),
        "experimental": {
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", super::model::CONTROLLER_PORT),
                "secret": secret,
                "default_mode": "rule"
            }
        }
    });
    let data = serde_json::to_vec_pretty(&config).map_err(|e| format!("生成 sing-box 配置失败：{e}"))?;
    Ok(BuiltConfig { data, warnings })
}

pub fn build_xray(
    nodes: &[ProxyNode],
    secret: &str,
    tun: bool,
    routes: Option<&[ProxyNode]>,
    extra_proxies: &[ProxyNode],
    outbound_interface: Option<&str>,
    preferred_name: Option<&str>,
    traffic_mode: TrafficMode,
) -> Result<BuiltConfig, String> {
    if tun {
        return Err("Xray 内核不支持 TUN，请改用 Mihomo 或 sing-box".into());
    }
    let route_nodes = routes.unwrap_or(nodes);
    let (mut route_kept, _) = unique_supported(KernelKind::Xray, route_nodes);
    if let Some(preferred) = preferred_name {
        if let Some(index) = route_kept.iter().position(|node| node.name == preferred) {
            let node = route_kept.remove(index);
            route_kept.insert(0, node);
        }
    }
    if route_kept.is_empty() && traffic_mode.uses_proxy() {
        return Err("当前内核不支持订阅中的节点协议，请更换订阅或内核".into());
    }
    let mut warnings = Vec::new();
    if let Some(warning) = warning_for(KernelKind::Xray, nodes) {
        warnings.push(warning);
    }

    let mut outbounds: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    for node in nodes.iter().chain(extra_proxies.iter()) {
        if !seen.insert(node.name.clone()) {
            continue;
        }
        if !KernelKind::Xray.supports_node(node) {
            continue;
        }
        match convert::to_xray(node) {
            Ok(mut value) => {
                if let Some(interface) = outbound_interface {
                    apply_xray_interface(&mut value, interface);
                }
                outbounds.push(value);
            }
            Err(error) => warnings.push(format!("{}：{error}", node.short_name())),
        }
    }
    outbounds.push(json!({ "tag": "direct", "protocol": "freedom", "settings": {} }));
    outbounds.push(json!({ "tag": "block", "protocol": "blackhole", "settings": {} }));

    let selected = route_kept.first().map(|node| convert::xray_tag(&node.name));
    let final_tag = if traffic_mode.uses_proxy() {
        selected.clone().unwrap_or_else(|| "direct".into())
    } else {
        "direct".into()
    };

    let mut rules = vec![
        json!({ "type": "field", "inboundTag": ["api"], "outboundTag": "api" }),
    ];
    if traffic_mode == TrafficMode::Rule {
        rules.push(json!({
            "type": "field",
            "ip": ["geoip:private"],
            "outboundTag": "direct"
        }));
        rules.push(json!({
            "type": "field",
            "domain": ["geosite:cn"],
            "outboundTag": "direct"
        }));
        rules.push(json!({
            "type": "field",
            "ip": ["geoip:cn"],
            "outboundTag": "direct"
        }));
    }
    rules.push(json!({
        "type": "field",
        "network": "tcp,udp",
        "outboundTag": final_tag
    }));

    let config = json!({
        "log": { "loglevel": "warning" },
        "stats": {},
        "api": { "tag": "api", "services": ["StatsService"] },
        "policy": {
            "system": {
                "statsInboundUplink": true,
                "statsInboundDownlink": true,
                "statsOutboundUplink": true,
                "statsOutboundDownlink": true
            }
        },
        "dns": {
            "servers": ["223.5.5.5", "1.1.1.1"],
            "queryStrategy": "UseIPv4"
        },
        "inbounds": [
            {
                "tag": "http-in",
                "listen": "127.0.0.1",
                "port": proxy_port(),
                "protocol": "http",
                "settings": {},
                "sniffing": { "enabled": true, "destOverride": ["http", "tls", "quic"] }
            },
            {
                "tag": "socks-in",
                "listen": "127.0.0.1",
                "port": XRAY_SOCKS_PORT,
                "protocol": "socks",
                "settings": { "udp": true },
                "sniffing": { "enabled": true, "destOverride": ["http", "tls", "quic"] }
            },
            {
                "tag": "api",
                "listen": "127.0.0.1",
                "port": XRAY_API_PORT,
                "protocol": "dokodemo-door",
                "settings": { "address": "127.0.0.1" }
            }
        ],
        "outbounds": outbounds,
        "routing": {
            "domainStrategy": "IPIfNonMatch",
            "rules": rules
        }
    });
    let _ = secret; // Xray 不使用 Clash 控制密钥。
    let data = serde_json::to_vec_pretty(&config).map_err(|e| format!("生成 Xray 配置失败：{e}"))?;
    Ok(BuiltConfig { data, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::model::Source;
    use crate::logic::subscription::parse;
    use std::path::Path;
    use std::process::Command;

    fn sample_nodes() -> Vec<ProxyNode> {
        let yaml = r#"
proxies:
  - name: TestSS
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: secret
  - name: TestVMess
    type: vmess
    server: 5.6.7.8
    port: 443
    uuid: 00000000-0000-0000-0000-000000000000
    alterId: 0
    cipher: auto
    tls: true
    servername: example.com
"#;
        let source = Source {
            id: "test".into(),
            name: "test".into(),
            url: "https://example.com/sub".into(),
        };
        parse(yaml, &source).expect("parse sample subscription")
    }

    #[test]
    fn singbox_configs_pass_real_check() {
        let nodes = sample_nodes();
        for mode in TrafficMode::ALL {
            let built = build_singbox(
                &nodes,
                "secret",
                false,
                None,
                &[],
                None,
                Some("TestSS"),
                mode,
            )
            .unwrap_or_else(|error| panic!("build {mode:?}: {error}"));
            check_with_core(KernelKind::SingBox, &built.data, mode.raw());
        }
    }

    #[test]
    fn xray_configs_pass_real_check() {
        let nodes = sample_nodes();
        for mode in TrafficMode::ALL {
            let built = build_xray(
                &nodes,
                "secret",
                false,
                None,
                &[],
                None,
                Some("TestSS"),
                mode,
            )
            .unwrap_or_else(|error| panic!("build {mode:?}: {error}"));
            check_with_core(KernelKind::Xray, &built.data, mode.raw());
        }
    }

    #[test]
    fn unsupported_nodes_are_reported_not_configured() {
        let yaml = r#"
proxies:
  - name: TestHY2
    type: hysteria2
    server: 1.2.3.4
    port: 443
    password: secret
"#;
        let source = Source {
            id: "test".into(),
            name: "test".into(),
            url: "https://example.com/sub".into(),
        };
        let nodes = parse(yaml, &source).expect("parse sample subscription");
        let built = build_xray(&nodes, "secret", false, None, &[], None, None, TrafficMode::Direct)
            .expect("direct-mode config should still build");
        assert!(!built.warnings.is_empty());
        assert!(
            build_xray(&nodes, "secret", false, None, &[], None, None, TrafficMode::Global).is_err(),
            "proxy mode without supported nodes must fail clearly"
        );
    }

    #[test]
    fn singbox_starts_and_serves_clash_api() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(KernelKind::SingBox.binary_name());
        if !exe.exists() {
            eprintln!("跳过 sing-box 启动测试：缺少 {}", exe.display());
            return;
        }
        let nodes = sample_nodes();
        let built =
            build_singbox(&nodes, "secret", false, None, &[], None, None, TrafficMode::Rule)
                .expect("build config");
        let dir = test_dir("singbox-run");
        let path = dir.join("config.json");
        std::fs::write(&path, &built.data).expect("write config");
        let mut child = Command::new(&exe)
            .args([
                "run",
                "-c",
                path.to_str().unwrap(),
                "-D",
                dir.to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(
                std::fs::File::create(dir.join("stderr.log")).expect("create log"),
            ))
            .spawn()
            .expect("start sing-box");
        let ready = wait_port(9090);
        let log = std::fs::read_to_string(dir.join("stderr.log")).unwrap_or_default();
        let output = Command::new("curl.exe")
            .args([
                "-s",
                "--max-time",
                "3",
                "-H",
                "Authorization: Bearer secret",
                "http://127.0.0.1:9090/version",
            ])
            .output();
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ready, "sing-box 未监听 Clash API 端口 9090：{log}");
        let output = output.expect("call sing-box Clash API");
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("version"),
            "sing-box Clash API 未返回版本：{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn xray_starts_and_serves_stats_api() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(KernelKind::Xray.binary_name());
        if !exe.exists() {
            eprintln!("跳过 Xray 启动测试：缺少 {}", exe.display());
            return;
        }
        let nodes = sample_nodes();
        let built =
            build_xray(&nodes, "secret", false, None, &[], None, None, TrafficMode::Rule)
                .expect("build config");
        let dir = test_dir("xray-run");
        let path = dir.join("config.json");
        std::fs::write(&path, &built.data).expect("write config");
        let mut child = Command::new(&exe)
            .args(["run", "-c", path.to_str().unwrap()])
            .env(
                "XRAY_LOCATION_ASSET",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("start xray");
        let ready = wait_port(crate::logic::kernel::XRAY_API_PORT);
        let output = Command::new(&exe)
            .args([
                "api",
                "statsquery",
                &format!("--server=127.0.0.1:{}", crate::logic::kernel::XRAY_API_PORT),
                "-pattern",
                "",
            ])
            .output();
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ready, "Xray 未监听 API 端口");
        let output = output.expect("call xray stats api");
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("stat"),
            "Xray Stats API 调用失败：{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn test_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "smartclash-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn wait_port(port: u16) -> bool {
        use std::net::{SocketAddr, TcpStream};
        use std::time::Duration;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        for _ in 0..50 {
            if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    fn check_with_core(kind: KernelKind, data: &[u8], label: &str) {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(kind.binary_name());
        if !exe.exists() {
            eprintln!("跳过 {} 校验：缺少 {}", kind.title(), exe.display());
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "smartclash-{}-test-{}-{}",
            kind.binary_name(),
            std::process::id(),
            label
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.json");
        std::fs::write(&path, data).expect("write config");
        let mut cmd = Command::new(&exe);
        match kind {
            KernelKind::Mihomo => {
                cmd.args(["-t", "-d", dir.to_str().unwrap(), "-f", path.to_str().unwrap()]);
            }
            KernelKind::SingBox => {
                cmd.args([
                    "check",
                    "-c",
                    path.to_str().unwrap(),
                    "-D",
                    dir.to_str().unwrap(),
                ]);
            }
            KernelKind::Xray => {
                cmd.args(["run", "-test", "-c", path.to_str().unwrap()]);
                cmd.env(
                    "XRAY_LOCATION_ASSET",
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"),
                );
            }
        }
        let output = cmd.output().expect("run kernel check");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "{} rejected {label} config: {}{}",
            kind.title(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// 让 Xray 出站走指定网卡：写入 streamSettings.sockopt.interface。
fn apply_xray_interface(outbound: &mut Value, interface: &str) {
    let Some(object) = outbound.as_object_mut() else {
        return;
    };
    let stream = object
        .entry("streamSettings")
        .or_insert_with(|| json!({}));
    let Some(stream) = stream.as_object_mut() else {
        return;
    };
    let sockopt = stream.entry("sockopt").or_insert_with(|| json!({}));
    if let Some(sockopt) = sockopt.as_object_mut() {
        sockopt.insert("interface".into(), json!(interface));
    }
}

/// 单节点临时探测配置：只监听一个本地 HTTP 端口，出口固定为该节点。
pub fn probe_config(kind: KernelKind, node: &ProxyNode, port: u16) -> Result<Vec<u8>, String> {
    match kind {
        KernelKind::Mihomo => {
            let mut obj = Map::new();
            obj.insert("mixed-port".into(), json!(port));
            obj.insert("allow-lan".into(), json!(false));
            obj.insert("bind-address".into(), json!("127.0.0.1"));
            obj.insert("mode".into(), json!("rule"));
            obj.insert("log-level".into(), json!("warning"));
            obj.insert(
                "proxies".into(),
                json!([Value::Object(node.payload.clone())]),
            );
            obj.insert(
                "rules".into(),
                json!([format!("MATCH,{}", node.name)]),
            );
            serde_json::to_vec_pretty(&Value::Object(obj))
                .map_err(|e| format!("生成探测配置失败：{e}"))
        }
        KernelKind::SingBox => {
            let outbound = convert::to_singbox(node)?;
            let config = json!({
                "log": { "level": "warning" },
                "inbounds": [{
                    "type": "http",
                    "tag": "probe-in",
                    "listen": "127.0.0.1",
                    "listen_port": port
                }],
                "outbounds": [outbound],
                "route": { "final": node.name }
            });
            serde_json::to_vec_pretty(&config).map_err(|e| format!("生成探测配置失败：{e}"))
        }
        KernelKind::Xray => {
            let outbound = convert::to_xray(node)?;
            let config = json!({
                "log": { "loglevel": "warning" },
                "inbounds": [{
                    "tag": "probe-in",
                    "listen": "127.0.0.1",
                    "port": port,
                    "protocol": "http",
                    "settings": {}
                }],
                "outbounds": [outbound],
                "routing": {
                    "domainStrategy": "IPIfNonMatch",
                    "rules": [{
                        "type": "field",
                        "inboundTag": ["probe-in"],
                        "outboundTag": convert::xray_tag(&node.name)
                    }]
                }
            });
            serde_json::to_vec_pretty(&config).map_err(|e| format!("生成探测配置失败：{e}"))
        }
    }
}

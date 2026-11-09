//! Clash 节点协议 → sing-box / Xray 配置片段转换。
//!
//! 订阅解析阶段保留的是 Clash 字段；这里只做协议与传输层的字段映射，
//! 不做任何网络请求。无法可靠映射的节点会被明确标记为不支持，而不是生成错误配置。

use crate::logic::kernel::KernelKind;
use crate::logic::model::ProxyNode;
use serde_json::{json, Map, Value};

type Fields = Map<String, Value>;

fn s<'a>(p: &'a Fields, key: &str) -> Option<&'a str> {
    p.get(key).and_then(|v| v.as_str()).filter(|v| !v.is_empty())
}

fn num(p: &Fields, key: &str) -> Option<u64> {
    p.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
            .or_else(|| v.as_str().and_then(|v| v.parse().ok()))
    })
}

fn flag(p: &Fields, key: &str) -> Option<bool> {
    p.get(key).and_then(|v| v.as_bool())
}

fn object<'a>(p: &'a Fields, key: &str) -> Option<&'a Fields> {
    p.get(key).and_then(|v| v.as_object())
}

fn string_list(value: Option<&Value>) -> Option<Vec<Value>> {
    let list = value?.as_array()?;
    let mut out = Vec::new();
    for item in list {
        if let Some(text) = item.as_str() {
            out.push(Value::String(text.to_string()));
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn first_host(p: &Fields, opts: Option<&Fields>) -> Option<String> {
    if let Some(host) = s(p, "servername").or_else(|| s(p, "sni")) {
        return Some(host.to_string());
    }
    let opts = opts?;
    if let Some(value) = opts.get("headers") {
        if let Some(map) = value.as_object() {
            for key in ["Host", "host"] {
                if let Some(host) = map.get(key).and_then(|v| v.as_str()) {
                    if !host.is_empty() {
                        return Some(host.to_string());
                    }
                }
            }
        }
    }
    if let Some(value) = opts.get("host") {
        if let Some(host) = value.as_str() {
            return Some(host.to_string());
        }
        if let Some(host) = value.as_array().and_then(|v| v.first()).and_then(|v| v.as_str()) {
            return Some(host.to_string());
        }
    }
    None
}

fn network(p: &Fields) -> &str {
    s(p, "network").unwrap_or("tcp")
}

fn protocol(p: &Fields) -> &str {
    s(p, "type").unwrap_or("")
}

fn transport_error(p: &Fields) -> Option<String> {
    match protocol(p) {
        "vmess" | "vless" | "trojan" => match network(p) {
            "tcp" | "ws" | "grpc" | "h2" | "httpupgrade" => None,
            other => Some(format!("不支持的传输方式 {other}")),
        },
        _ => None,
    }
}

/// 返回该节点在当前内核下无法使用的原因。`None` 表示可以转换。
pub fn support_error(kind: KernelKind, p: &Fields) -> Option<String> {
    let ty = protocol(p);
    if s(p, "server").is_none() || num(p, "port").is_none() {
        return Some("缺少服务器地址或端口".into());
    }
    if kind == KernelKind::Mihomo {
        return None;
    }
    if let Some(error) = transport_error(p) {
        return Some(error);
    }
    match kind {
        KernelKind::SingBox => match ty {
            "ss" => {
                if p.contains_key("plugin") {
                    Some("sing-box 不支持 Clash 内建的 SS 插件".into())
                } else {
                    None
                }
            }
            "vmess" | "vless" | "trojan" | "hysteria" | "hysteria2" | "tuic" | "anytls"
            | "http" => None,
            "socks5" => {
                if flag(p, "tls").unwrap_or(false) {
                    Some("sing-box 的 SOCKS5 出站不支持 TLS".into())
                } else {
                    None
                }
            }
            "ssr" => Some("sing-box 不支持 ShadowsocksR".into()),
            "snell" => Some("sing-box 不支持 Snell".into()),
            other => Some(format!("sing-box 不支持协议 {other}")),
        },
        KernelKind::Xray => match ty {
            "ss" => {
                if p.contains_key("plugin") {
                    Some("Xray 不支持 Clash 内建的 SS 插件".into())
                } else {
                    None
                }
            }
            "vmess" | "vless" | "trojan" | "socks5" | "http" => None,
            "hysteria" | "hysteria2" | "tuic" | "anytls" => {
                Some(format!("Xray 不支持 {ty} 协议"))
            }
            "ssr" => Some("Xray 不支持 ShadowsocksR".into()),
            "snell" => Some("Xray 不支持 Snell".into()),
            other => Some(format!("Xray 不支持协议 {other}")),
        },
        KernelKind::Mihomo => None,
    }
}

// ---------------------------------------------------------------- sing-box

fn singbox_tls(p: &Fields, force: bool) -> Option<Value> {
    let reality = object(p, "reality-opts");
    let enabled = force || flag(p, "tls").unwrap_or(false) || reality.is_some();
    if !enabled {
        return None;
    }
    let mut tls = Map::new();
    tls.insert("enabled".into(), json!(true));
    if let Some(name) = s(p, "servername").or_else(|| s(p, "sni")) {
        tls.insert("server_name".into(), json!(name));
    }
    if flag(p, "skip-cert-verify").unwrap_or(false) {
        tls.insert("insecure".into(), json!(true));
    }
    if let Some(alpn) = string_list(p.get("alpn")) {
        tls.insert("alpn".into(), Value::Array(alpn));
    }
    let fingerprint = s(p, "client-fingerprint").or_else(|| s(p, "fingerprint"));
    if let Some(reality) = reality {
        let mut value = Map::new();
        value.insert("enabled".into(), json!(true));
        if let Some(key) = s(reality, "public-key") {
            value.insert("public_key".into(), json!(key));
        }
        if let Some(short) = reality.get("short-id").and_then(|v| v.as_str()) {
            value.insert("short_id".into(), json!(short));
        }
        tls.insert("reality".into(), Value::Object(value));
        tls.insert(
            "utls".into(),
            json!({ "enabled": true, "fingerprint": fingerprint.unwrap_or("chrome") }),
        );
    } else if let Some(fingerprint) = fingerprint {
        tls.insert(
            "utls".into(),
            json!({ "enabled": true, "fingerprint": fingerprint }),
        );
    }
    Some(Value::Object(tls))
}

fn singbox_transport(p: &Fields) -> Result<Option<Value>, String> {
    let net = network(p);
    let value = match net {
        "tcp" => return Ok(None),
        "ws" => {
            let opts = object(p, "ws-opts");
            let mut ws = Map::new();
            ws.insert("type".into(), json!("ws"));
            let path = opts
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            ws.insert("path".into(), json!(path));
            if let Some(headers) = opts.and_then(|o| o.get("headers")).and_then(|v| v.as_object()) {
                let mut map = Map::new();
                for (key, value) in headers {
                    if let Some(text) = value.as_str() {
                        map.insert(key.clone(), json!(text));
                    }
                }
                if !map.is_empty() {
                    ws.insert("headers".into(), Value::Object(map));
                }
            }
            Value::Object(ws)
        }
        "grpc" => {
            let service = object(p, "grpc-opts")
                .and_then(|o| o.get("grpc-service-name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            json!({ "type": "grpc", "service_name": service })
        }
        "h2" => {
            let opts = object(p, "h2-opts");
            let mut http = Map::new();
            http.insert("type".into(), json!("http"));
            let path = opts
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            http.insert("path".into(), json!(path));
            if let Some(host) = opts
                .and_then(|o| o.get("host"))
                .and_then(|v| v.as_array())
            {
                if !host.is_empty() {
                    http.insert("host".into(), Value::Array(host.clone()));
                }
            } else if let Some(host) = first_host(p, opts) {
                http.insert("host".into(), json!([host]));
            }
            Value::Object(http)
        }
        "httpupgrade" => {
            let opts = object(p, "http-upgrade-opts");
            let mut upgrade = Map::new();
            upgrade.insert("type".into(), json!("httpupgrade"));
            let path = opts
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            upgrade.insert("path".into(), json!(path));
            if let Some(host) = first_host(p, opts) {
                upgrade.insert("host".into(), json!(host));
            }
            Value::Object(upgrade)
        }
        other => return Err(format!("不支持的传输方式 {other}")),
    };
    Ok(Some(value))
}

fn mbps(p: &Fields, key: &str) -> Option<u64> {
    num(p, key).filter(|value| *value > 0)
}

pub fn to_singbox(node: &ProxyNode) -> Result<Value, String> {
    let p = &node.payload;
    if let Some(error) = support_error(KernelKind::SingBox, p) {
        return Err(error);
    }
    let ty = protocol(p);
    let server = s(p, "server").unwrap_or("");
    let port = num(p, "port").unwrap_or(0);
    let mut out = Map::new();
    out.insert("tag".into(), json!(node.name));
    out.insert("server".into(), json!(server));
    out.insert("server_port".into(), json!(port));

    match ty {
        "ss" => {
            out.insert("type".into(), json!("shadowsocks"));
            out.insert("method".into(), json!(s(p, "cipher").unwrap_or("aes-128-gcm")));
            out.insert("password".into(), json!(s(p, "password").unwrap_or("")));
            if flag(p, "udp-over-tcp").unwrap_or(false) {
                out.insert("udp_over_tcp".into(), json!(true));
            }
        }
        "vmess" => {
            out.insert("type".into(), json!("vmess"));
            out.insert("uuid".into(), json!(s(p, "uuid").unwrap_or("")));
            out.insert("security".into(), json!(s(p, "cipher").unwrap_or("auto")));
            out.insert("alter_id".into(), json!(num(p, "alterId").unwrap_or(0)));
            if let Some(tls) = singbox_tls(p, false) {
                out.insert("tls".into(), tls);
            }
            if let Some(transport) = singbox_transport(p)? {
                out.insert("transport".into(), transport);
            }
        }
        "vless" => {
            out.insert("type".into(), json!("vless"));
            out.insert("uuid".into(), json!(s(p, "uuid").unwrap_or("")));
            if let Some(flow) = s(p, "flow") {
                out.insert("flow".into(), json!(flow));
            }
            if let Some(tls) = singbox_tls(p, false) {
                out.insert("tls".into(), tls);
            }
            if let Some(transport) = singbox_transport(p)? {
                out.insert("transport".into(), transport);
            }
        }
        "trojan" => {
            out.insert("type".into(), json!("trojan"));
            out.insert("password".into(), json!(s(p, "password").unwrap_or("")));
            if let Some(tls) = singbox_tls(p, true) {
                out.insert("tls".into(), tls);
            }
            if let Some(transport) = singbox_transport(p)? {
                out.insert("transport".into(), transport);
            }
        }
        "hysteria" => {
            out.insert("type".into(), json!("hysteria"));
            if let Some(auth) = s(p, "auth-str").or_else(|| s(p, "auth_str")) {
                out.insert("auth_str".into(), json!(auth));
            }
            if let Some(value) = mbps(p, "up") {
                out.insert("up_mbps".into(), json!(value));
            }
            if let Some(value) = mbps(p, "down") {
                out.insert("down_mbps".into(), json!(value));
            }
            if let Some(obfs) = s(p, "obfs") {
                out.insert("obfs".into(), json!(obfs));
            }
            if let Some(tls) = singbox_tls(p, true) {
                out.insert("tls".into(), tls);
            }
        }
        "hysteria2" => {
            out.insert("type".into(), json!("hysteria2"));
            out.insert("password".into(), json!(s(p, "password").unwrap_or("")));
            if let Some(value) = mbps(p, "up") {
                out.insert("up_mbps".into(), json!(value));
            }
            if let Some(value) = mbps(p, "down") {
                out.insert("down_mbps".into(), json!(value));
            }
            if let Some(obfs) = s(p, "obfs") {
                out.insert("obfs".into(), json!({
                    "type": obfs,
                    "password": s(p, "obfs-password").unwrap_or("")
                }));
            }
            if let Some(tls) = singbox_tls(p, true) {
                out.insert("tls".into(), tls);
            }
        }
        "tuic" => {
            out.insert("type".into(), json!("tuic"));
            out.insert("uuid".into(), json!(s(p, "uuid").unwrap_or("")));
            out.insert("password".into(), json!(s(p, "password").unwrap_or("")));
            if let Some(value) = s(p, "congestion-controller") {
                out.insert("congestion_control".into(), json!(value));
            }
            if let Some(value) = s(p, "udp-relay-mode") {
                out.insert("udp_relay_mode".into(), json!(value));
            }
            if flag(p, "reduce-rtt").unwrap_or(false) {
                out.insert("zero_rtt_handshake".into(), json!(true));
            }
            if let Some(tls) = singbox_tls(p, true) {
                out.insert("tls".into(), tls);
            }
        }
        "anytls" => {
            out.insert("type".into(), json!("anytls"));
            out.insert("password".into(), json!(s(p, "password").unwrap_or("")));
            if let Some(tls) = singbox_tls(p, true) {
                out.insert("tls".into(), tls);
            }
        }
        "http" => {
            out.insert("type".into(), json!("http"));
            if let Some(user) = s(p, "username") {
                out.insert("username".into(), json!(user));
            }
            if let Some(pass) = s(p, "password") {
                out.insert("password".into(), json!(pass));
            }
            if let Some(tls) = singbox_tls(p, false) {
                out.insert("tls".into(), tls);
            }
        }
        "socks5" => {
            out.insert("type".into(), json!("socks"));
            out.insert("version".into(), json!("5"));
            if let Some(user) = s(p, "username") {
                out.insert("username".into(), json!(user));
            }
            if let Some(pass) = s(p, "password") {
                out.insert("password".into(), json!(pass));
            }
        }
        other => return Err(format!("sing-box 不支持协议 {other}")),
    }

    if let Some(dialer) = s(p, "dialer-proxy") {
        out.insert("detour".into(), json!(dialer));
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- Xray

pub fn xray_tag(name: &str) -> String {
    format!("n:{name}")
}

fn xray_reality(p: &Fields) -> Option<Value> {
    let reality = object(p, "reality-opts")?;
    let mut value = Map::new();
    if let Some(name) = s(p, "servername").or_else(|| s(p, "sni")) {
        value.insert("serverName".into(), json!(name));
    }
    if let Some(key) = s(reality, "public-key") {
        value.insert("publicKey".into(), json!(key));
    }
    if let Some(short) = reality.get("short-id").and_then(|v| v.as_str()) {
        value.insert("shortId".into(), json!(short));
    }
    if let Some(fingerprint) = s(p, "client-fingerprint").or_else(|| s(p, "fingerprint")) {
        value.insert("fingerprint".into(), json!(fingerprint));
    }
    if let Some(spider) = reality.get("spider-x").and_then(|v| v.as_str()) {
        value.insert("spiderX".into(), json!(spider));
    }
    Some(Value::Object(value))
}

fn xray_stream(p: &Fields, force_tls: bool) -> Result<Value, String> {
    let net = network(p);
    let reality = xray_reality(p);
    let tls_enabled = force_tls || flag(p, "tls").unwrap_or(false) || reality.is_some();
    let mut stream = Map::new();
    stream.insert(
        "network".into(),
        json!(match net {
            "tcp" => "tcp",
            "ws" => "ws",
            "grpc" => "grpc",
            "h2" => "h2",
            "httpupgrade" => "httpupgrade",
            other => return Err(format!("不支持的传输方式 {other}")),
        }),
    );
    stream.insert(
        "security".into(),
        json!(if reality.is_some() {
            "reality"
        } else if tls_enabled {
            "tls"
        } else {
            "none"
        }),
    );
    if let Some(reality) = reality {
        stream.insert("realitySettings".into(), reality);
    } else if tls_enabled {
        let mut tls = Map::new();
        if let Some(name) = s(p, "servername").or_else(|| s(p, "sni")) {
            tls.insert("serverName".into(), json!(name));
        }
        if flag(p, "skip-cert-verify").unwrap_or(false) {
            tls.insert("allowInsecure".into(), json!(true));
        }
        if let Some(alpn) = string_list(p.get("alpn")) {
            tls.insert("alpn".into(), Value::Array(alpn));
        }
        if let Some(fingerprint) = s(p, "client-fingerprint").or_else(|| s(p, "fingerprint")) {
            tls.insert("fingerprint".into(), json!(fingerprint));
        }
        stream.insert("tlsSettings".into(), Value::Object(tls));
    }
    match net {
        "ws" => {
            let opts = object(p, "ws-opts");
            let mut ws = Map::new();
            let path = opts
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            ws.insert("path".into(), json!(path));
            if let Some(host) = first_host(p, opts) {
                ws.insert("headers".into(), json!({ "Host": host }));
            }
            stream.insert("wsSettings".into(), Value::Object(ws));
        }
        "grpc" => {
            let service = object(p, "grpc-opts")
                .and_then(|o| o.get("grpc-service-name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            stream.insert("grpcSettings".into(), json!({ "serviceName": service }));
        }
        "h2" => {
            let opts = object(p, "h2-opts");
            let mut h2 = Map::new();
            let path = opts
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            h2.insert("path".into(), json!(path));
            if let Some(list) = opts
                .and_then(|o| o.get("host"))
                .and_then(|v| v.as_array())
            {
                h2.insert("host".into(), Value::Array(list.clone()));
            } else if let Some(host) = first_host(p, opts) {
                h2.insert("host".into(), json!([host]));
            }
            stream.insert("h2Settings".into(), Value::Object(h2));
        }
        "httpupgrade" => {
            let opts = object(p, "http-upgrade-opts");
            let mut upgrade = Map::new();
            let path = opts
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            upgrade.insert("path".into(), json!(path));
            if let Some(host) = first_host(p, opts) {
                upgrade.insert("host".into(), json!(host));
            }
            stream.insert("httpupgradeSettings".into(), Value::Object(upgrade));
        }
        _ => {}
    }
    if let Some(dialer) = s(p, "dialer-proxy") {
        stream.insert(
            "sockopt".into(),
            json!({ "dialerProxy": xray_tag(dialer) }),
        );
    }
    Ok(Value::Object(stream))
}

pub fn to_xray(node: &ProxyNode) -> Result<Value, String> {
    let p = &node.payload;
    if let Some(error) = support_error(KernelKind::Xray, p) {
        return Err(error);
    }
    let ty = protocol(p);
    let server = s(p, "server").unwrap_or("");
    let port = num(p, "port").unwrap_or(0);
    let mut out = Map::new();
    out.insert("tag".into(), json!(xray_tag(&node.name)));

    let force_tls = matches!(ty, "trojan");
    match ty {
        "ss" => {
            out.insert("protocol".into(), json!("shadowsocks"));
            out.insert(
                "settings".into(),
                json!({ "servers": [{
                    "address": server,
                    "port": port,
                    "method": s(p, "cipher").unwrap_or("aes-128-gcm"),
                    "password": s(p, "password").unwrap_or("")
                }]}),
            );
        }
        "vmess" => {
            out.insert("protocol".into(), json!("vmess"));
            out.insert(
                "settings".into(),
                json!({ "vnext": [{
                    "address": server,
                    "port": port,
                    "users": [{
                        "id": s(p, "uuid").unwrap_or(""),
                        "alterId": num(p, "alterId").unwrap_or(0),
                        "security": s(p, "cipher").unwrap_or("auto"),
                        "level": 0
                    }]
                }]}),
            );
        }
        "vless" => {
            out.insert("protocol".into(), json!("vless"));
            let mut user = Map::new();
            user.insert("id".into(), json!(s(p, "uuid").unwrap_or("")));
            user.insert("encryption".into(), json!("none"));
            if let Some(flow) = s(p, "flow") {
                user.insert("flow".into(), json!(flow));
            }
            user.insert("level".into(), json!(0));
            out.insert(
                "settings".into(),
                json!({ "vnext": [{
                    "address": server,
                    "port": port,
                    "users": [Value::Object(user)]
                }]}),
            );
        }
        "trojan" => {
            out.insert("protocol".into(), json!("trojan"));
            out.insert(
                "settings".into(),
                json!({ "servers": [{
                    "address": server,
                    "port": port,
                    "password": s(p, "password").unwrap_or(""),
                    "level": 0
                }]}),
            );
        }
        "socks5" => {
            out.insert("protocol".into(), json!("socks"));
            let mut server_obj = Map::new();
            server_obj.insert("address".into(), json!(server));
            server_obj.insert("port".into(), json!(port));
            if let Some(user) = s(p, "username") {
                server_obj.insert(
                    "users".into(),
                    json!([{ "user": user, "pass": s(p, "password").unwrap_or("") }]),
                );
            }
            out.insert("settings".into(), json!({ "servers": [Value::Object(server_obj)] }));
        }
        "http" => {
            out.insert("protocol".into(), json!("http"));
            let mut server_obj = Map::new();
            server_obj.insert("address".into(), json!(server));
            server_obj.insert("port".into(), json!(port));
            if let Some(user) = s(p, "username") {
                server_obj.insert(
                    "users".into(),
                    json!([{ "user": user, "pass": s(p, "password").unwrap_or("") }]),
                );
            }
            out.insert("settings".into(), json!({ "servers": [Value::Object(server_obj)] }));
        }
        other => return Err(format!("Xray 不支持协议 {other}")),
    }
    out.insert("streamSettings".into(), xray_stream(p, force_tls)?);
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn node(payload: Value) -> ProxyNode {
        ProxyNode {
            id: "id".into(),
            name: "测试节点 · abc123456789".into(),
            exit_name: "测试节点 · abc123456789".into(),
            source_id: "source".into(),
            payload: payload.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn converts_common_protocols() {
        let vmess = node(json!({
            "name": "测试节点 · abc123456789", "type": "vmess", "server": "a.example",
            "port": 443, "uuid": "6f2b0b4e-1f0b-4c1e-9b1a-8a2b3c4d5e6f", "alterId": 0,
            "cipher": "auto", "tls": true, "servername": "a.example", "network": "ws",
            "ws-opts": {"path": "/ws", "headers": {"Host": "a.example"}}
        }));
        let converted = to_singbox(&vmess).unwrap();
        assert_eq!(converted["type"], "vmess");
        assert_eq!(converted["transport"]["type"], "ws");
        assert_eq!(converted["tls"]["server_name"], "a.example");
        let converted = to_xray(&vmess).unwrap();
        assert_eq!(converted["protocol"], "vmess");
        assert_eq!(converted["streamSettings"]["network"], "ws");

        let trojan = node(json!({
            "name": "测试节点 · abc123456789", "type": "trojan", "server": "t.example",
            "port": 443, "password": "p", "sni": "t.example", "network": "grpc",
            "grpc-opts": {"grpc-service-name": "svc"}
        }));
        assert!(to_singbox(&trojan).is_ok());
        assert!(to_xray(&trojan).is_ok());

        let hy2 = node(json!({
            "name": "测试节点 · abc123456789", "type": "hysteria2", "server": "h.example",
            "port": 443, "password": "p", "sni": "h.example"
        }));
        assert!(to_singbox(&hy2).is_ok());
        assert!(to_xray(&hy2).is_err());
    }

    #[test]
    fn unsupported_protocols_are_reported() {
        let ssr = node(json!({
            "name": "测试节点 · abc123456789", "type": "ssr", "server": "s.example", "port": 443
        }));
        assert!(support_error(KernelKind::SingBox, &ssr.payload).is_some());
        assert!(support_error(KernelKind::Xray, &ssr.payload).is_some());
        assert!(support_error(KernelKind::Mihomo, &ssr.payload).is_none());

        let bad_network = node(json!({
            "name": "测试节点 · abc123456789", "type": "vmess", "server": "s.example",
            "port": 443, "uuid": "u", "network": "kcp"
        }));
        assert!(support_error(KernelKind::SingBox, &bad_network.payload).is_some());
        assert!(support_error(KernelKind::Xray, &bad_network.payload).is_some());
    }

    #[test]
    fn all_declared_protocols_are_deterministic() {
        // 防止协议分支被意外删掉：每个协议至少在 Mihomo 之外给出明确支持/不支持结论。
        let protocols = [
            "ss", "ssr", "vmess", "vless", "trojan", "hysteria", "hysteria2", "tuic", "socks5",
            "http", "snell", "anytls",
        ];
        let seen: BTreeSet<&str> = protocols.into_iter().collect();
        for protocol in seen {
            let value = node(json!({
                "name": "测试节点 · abc123456789", "type": protocol, "server": "s.example", "port": 443
            }));
            assert!(convert_decision(KernelKind::SingBox, &value.payload));
            assert!(convert_decision(KernelKind::Xray, &value.payload));
        }
    }

    fn convert_decision(kind: KernelKind, payload: &Fields) -> bool {
        support_error(kind, payload).is_some() || to_singbox_for(kind, payload).is_ok()
    }

    fn to_singbox_for(kind: KernelKind, payload: &Fields) -> Result<(), String> {
        let node = ProxyNode {
            id: "id".into(),
            name: "测试节点 · abc123456789".into(),
            exit_name: "测试节点 · abc123456789".into(),
            source_id: "source".into(),
            payload: payload.clone(),
        };
        match kind {
            KernelKind::SingBox => to_singbox(&node).map(|_| ()),
            KernelKind::Xray => to_xray(&node).map(|_| ()),
            KernelKind::Mihomo => Ok(()),
        }
    }
}

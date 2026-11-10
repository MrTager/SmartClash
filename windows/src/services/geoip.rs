//! IP 地理定位：与 macOS 版 GeoIPService 对齐。
//! 只把已验证的公网 IP 发给 ipwho.is；结果缓存 24 小时。

use crate::logic::model::GeoLocation;
use crate::services::{command, paths};
use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Mutex;

const CACHE_TTL_MS: u64 = 86_400_000;
const MAX_CACHE: usize = 2000;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Entry {
    location: GeoLocation,
    checked: u64,
}

struct Cache {
    map: HashMap<String, Entry>,
    dirty: bool,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn now_ms() -> u64 {
    crate::logic::health::now_ms()
}

fn cache_path() -> std::path::PathBuf {
    paths::data_root().join("geoip-cache.json")
}

fn with_cache<R>(f: impl FnOnce(&mut Cache) -> R) -> R {
    let mut guard = CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(|| {
        let map = std::fs::read(cache_path())
            .ok()
            .and_then(|data| serde_json::from_slice::<HashMap<String, Entry>>(&data).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, entry)| {
                now_ms().saturating_sub(entry.checked) < CACHE_TTL_MS && entry.location.is_valid()
            })
            .collect();
        Cache { map, dirty: false }
    });
    f(cache)
}

fn persist(cache: &mut Cache) {
    if !cache.dirty {
        return;
    }
    if cache.map.len() > MAX_CACHE {
        let mut entries: Vec<(String, Entry)> = cache.map.drain().collect();
        entries.sort_by(|a, b| b.1.checked.cmp(&a.1.checked));
        entries.truncate(MAX_CACHE / 2);
        cache.map = entries.into_iter().collect();
    }
    if let Ok(data) = serde_json::to_vec(&cache.map) {
        let _ = paths::write_private(&cache_path(), &data);
        cache.dirty = false;
    }
}

/// 是否公网 IP（不把内网 / 回环 / 链路本地地址发给外部服务）。
pub fn is_public_ip(text: &str) -> bool {
    let Ok(ip) = text.parse::<IpAddr>() else {
        return false;
    };
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 100.64.0.0/10（运营商 CGNAT）与 198.18.0.0/15（基准测试段，
            // mihomo fake-ip 默认池 198.18.0.1/16 落在这里）都不是真实公网地址。
            let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
            let fake_ip = o[0] == 198 && (o[1] & 0xfe) == 18;
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || cgnat
                || fake_ip
                || o[0] >= 224)
        }
        IpAddr::V6(v6) => {
            let head = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (head & 0xffc0) == 0xfe80
                || (head & 0xfe00) == 0xfc00)
        }
    }
}

fn looks_like_domain(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.ends_with(".local")
        && host != "localhost"
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !host.parse::<IpAddr>().is_ok()
}

/// TUN 开启时系统 DNS 被 mihomo 劫持并返回 fake-ip（198.18.x.x），
/// 需要走 DoH 拿真实解析结果。用 --resolve 钉住阿里 DoH 的 IP，
/// 避免"解析 DoH 服务器域名"本身再被劫持。
fn doh_query(host: &str, kind: u8) -> Option<String> {
    let url = format!("https://dns.alidns.com/resolve?name={host}&type={kind}");
    let output = command::run(
        &paths::curl_binary(),
        &[
            "-s".into(),
            "--noproxy".into(),
            "*".into(),
            "--max-time".into(),
            "4".into(),
            "--resolve".into(),
            "dns.alidns.com:443:223.5.5.5".into(),
            "-H".into(),
            "accept: application/dns-json".into(),
            url,
        ],
    )
    .ok()?;
    if output.code != 0 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&output.text).ok()?;
    let answers = value.get("Answer")?.as_array()?;
    answers
        .iter()
        .filter(|answer| answer.get("type").and_then(|v| v.as_u64()) == Some(kind as u64))
        .filter_map(|answer| answer.get("data").and_then(|v| v.as_str()))
        .find(|data| is_public_ip(data))
        .map(|data| data.to_string())
}

fn doh_resolve(host: &str) -> Option<String> {
    // 优先 A 记录，没有公网 IPv4 再查 AAAA。
    doh_query(host, 1).or_else(|| doh_query(host, 28))
}

/// 与 macOS 版 publicAddress(for:) 一致：域名先解析 DNS，取第一个公网地址。
/// 系统解析拿不到公网地址（例如 TUN fake-ip 劫持）时回退 DoH。
pub fn public_address(host: &str) -> Option<String> {
    if is_public_ip(host) {
        return Some(host.to_string());
    }
    if !looks_like_domain(host) {
        return None;
    }
    if let Ok(addresses) = (host, 443).to_socket_addrs() {
        let mut first_v6 = None;
        for address in addresses {
            let ip = address.ip().to_string();
            if is_public_ip(&ip) {
                if ip.contains(':') {
                    first_v6.get_or_insert(ip);
                } else {
                    return Some(ip);
                }
            }
        }
        if let Some(v6) = first_v6 {
            return Some(v6);
        }
    }
    doh_resolve(host)
}

fn query(ip: &str) -> Option<GeoLocation> {
    let url = format!("https://ipwho.is/{ip}");
    // 直接直连查询，不经过本机代理；超时 5 秒。
    let output = command::run(
        &paths::curl_binary(),
        &[
            "-s".into(),
            "-S".into(),
            "--noproxy".into(),
            "*".into(),
            "--max-time".into(),
            "5".into(),
            url,
        ],
    )
    .ok()?;
    if output.code != 0 || output.text.len() >= 65_536 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&output.text).ok()?;
    if value.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let latitude = value.get("latitude").and_then(|v| v.as_f64())?;
    let longitude = value.get("longitude").and_then(|v| v.as_f64())?;
    let country = value
        .get("country")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;
    let location = GeoLocation {
        latitude,
        longitude,
        country,
        city: value
            .get("city")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };
    location.is_valid().then_some(location)
}

/// 查询一个 host（IP 或域名）的位置；缓存 24 小时。
pub fn location(host: &str) -> Option<GeoLocation> {
    let ip = public_address(host)?;
    if let Some(entry) = with_cache(|cache| cache.map.get(&ip).cloned()) {
        if now_ms().saturating_sub(entry.checked) < CACHE_TTL_MS {
            return Some(entry.location);
        }
    }
    let location = query(&ip)?;
    with_cache(|cache| {
        cache.map.insert(
            ip.clone(),
            Entry {
                location: location.clone(),
                checked: now_ms(),
            },
        );
        cache.dirty = true;
        persist(cache);
    });
    Some(location)
}

/// 退出时把缓存落盘（后台线程结束时调用）。
pub fn flush() {
    with_cache(|cache| persist(cache));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ip_filtering() {
        // 真实公网地址保留。
        assert!(is_public_ip("114.37.232.43"));
        assert!(is_public_ip("1.1.1.1"));
        // 私网 / 回环 / 链路本地排除。
        assert!(!is_public_ip("192.168.1.1"));
        assert!(!is_public_ip("10.0.0.1"));
        assert!(!is_public_ip("127.0.0.1"));
        assert!(!is_public_ip("169.254.1.1"));
        // mihomo fake-ip 池（198.18.0.0/15）与 CGNAT（100.64.0.0/10）排除。
        assert!(!is_public_ip("198.18.0.76"));
        assert!(!is_public_ip("198.19.255.255"));
        assert!(!is_public_ip("100.64.0.1"));
        assert!(!is_public_ip("100.127.255.254"));
        // 相邻段不误伤。
        assert!(is_public_ip("198.20.0.1"));
        assert!(is_public_ip("100.128.0.1"));
        // 域名与非法输入。
        assert!(!is_public_ip("example.com"));
        assert!(!is_public_ip(""));
    }
}

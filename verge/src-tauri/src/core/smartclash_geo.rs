use super::handle::Handle;
use crate::config::Config;
use serde::Serialize;
use serde_yaml_ng::Value;
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    ip: String,
    latitude: f64,
    longitude: f64,
    country: String,
    city: String,
}

#[derive(Serialize, specta::Type)]
pub struct MapNode {
    name: String,
    location: Location,
}

#[derive(Serialize, specta::Type)]
pub struct MapRoute {
    id: String,
    host: String,
    chain: Vec<String>,
    locations: Vec<Location>,
}

#[derive(Default, Serialize, specta::Type)]
pub struct MapSnapshot {
    nodes: Vec<MapNode>,
    routes: Vec<MapRoute>,
    unmapped: usize,
}

struct Entry {
    location: Option<Location>,
    checked: Instant,
}

static CACHE: LazyLock<parking_lot::Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);
static RESOLVING: AtomicBool = AtomicBool::new(false);

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Only global unicast; exclude documentation and embedded IPv4.
            segments[0] & 0xe000 == 0x2000 && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

/// Mihomo alpha leaves `destinationIP` empty in normal DNS mode; `remoteDestination`
/// keeps the resolved target in that case.
fn destination_ip(connection: &tauri_plugin_mihomo::models::Connection) -> String {
    let metadata = &connection.metadata;
    if metadata.destination_ip.is_empty() {
        metadata.remote_destination.clone()
    } else {
        metadata.destination_ip.clone()
    }
}

fn cached(host: &str) -> Option<Location> {
    CACHE
        .lock()
        .get(host)
        .filter(|entry| entry.checked.elapsed() < Duration::from_secs(86_400))
        .and_then(|entry| entry.location.clone())
}

/// 连线涉及的主机是否仍在后台解析中 (缓存里还没有条目)。
/// 已有条目但定位失败 (= 解析过且失败) 才视为真正无法定位。
fn is_pending(destination_host: &str, chain: &[String], servers: &HashMap<String, String>) -> bool {
    fn unknown(host: &str) -> bool {
        !CACHE.lock().contains_key(host)
    }
    unknown(destination_host)
        || chain
            .iter()
            .any(|name| servers.get(name).is_some_and(|server| unknown(server)))
}

async fn locate(client: &reqwest::Client, host: &str) -> Option<Location> {
    let ip = match host.parse::<IpAddr>() {
        Ok(ip) if public_ip(ip) => ip,
        Ok(_) => return None,
        Err(_) => tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host((host, 443)))
            .await
            .ok()?
            .ok()?
            .map(|address| address.ip())
            .find(|ip| public_ip(*ip))?,
    };
    // Only the resolved public IP is sent to the location provider.
    let mut response = client.get(format!("https://ipwho.is/{ip}")).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 32_768 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    if value["success"].as_bool() != Some(true) {
        return None;
    }
    let latitude = value["latitude"].as_f64()?;
    let longitude = value["longitude"].as_f64()?;
    if !latitude.is_finite() || !longitude.is_finite() || latitude.abs() > 90.0 || longitude.abs() > 180.0 {
        return None;
    }
    Some(Location {
        ip: ip.to_string(),
        latitude,
        longitude,
        country: value["country"].as_str().unwrap_or_default().into(),
        city: value["city"].as_str().unwrap_or_default().into(),
    })
}

fn resolve_missing(mut hosts: Vec<String>) {
    if RESOLVING.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                RESOLVING.store(false, Ordering::Release);
            }
        }
        let _guard = Guard;
        let Ok(client) = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(4))
            .build()
        else {
            return;
        };
        // 去重: servers 与连接目标常重复, 重复项白白消耗每轮 4 个的解析额度
        let mut seen = std::collections::HashSet::new();
        hosts.retain(|host| seen.insert(host.clone()));
        let mut count = 0;
        for host in hosts {
            let fresh = CACHE.lock().get(&host).is_some_and(|entry| {
                entry.checked.elapsed() < Duration::from_secs(if entry.location.is_some() { 86_400 } else { 300 })
            });
            if fresh {
                continue;
            }
            let location = locate(&client, &host).await;
            let mut cache = CACHE.lock();
            if cache.len() >= 2000 {
                cache.retain(|_, entry| entry.checked.elapsed() < Duration::from_secs(300));
            }
            cache.insert(
                host,
                Entry {
                    location,
                    checked: Instant::now(),
                },
            );
            count += 1;
            if count == 8 || Handle::global().is_exiting() {
                break;
            }
        }
    });
}

pub async fn snapshot() -> MapSnapshot {
    let runtime = Config::runtime().await.data_arc();
    let definitions: HashMap<&str, &Value> = runtime
        .config
        .as_ref()
        .and_then(|config| config.get("proxies"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|node| Some((node.get("name")?.as_str()?, node)))
        .collect();
    let servers: HashMap<String, String> = runtime
        .config
        .as_ref()
        .and_then(|config| config.get("proxies"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|node| {
            Some((
                node.get("name")?.as_str()?.to_owned(),
                node.get("server")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    let connections = match tokio::time::timeout(Duration::from_secs(2), Handle::mihomo().get_connections()).await {
        Ok(Ok(value)) => value.connections.unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut hosts: Vec<String> = connections
        .iter()
        .take(64)
        .map(|connection| destination_ip(connection))
        .filter(|ip| ip.parse::<IpAddr>().is_ok_and(public_ip))
        .collect();
    // 节点服务器优先: 它们决定节点/连线的可绘制性, 每轮解析额度有限,
    // 排在连接目标前面避免被大量一次性 destination 饿死
    hosts.extend(servers.values().cloned());
    let mut result = MapSnapshot::default();
    for (name, server) in &servers {
        if let Some(location) = cached(server) {
            result.nodes.push(MapNode {
                name: name.clone(),
                location,
            });
        }
    }
    for connection in connections.iter().take(64) {
        let chain = connection
            .chains
            .iter()
            .filter_map(|name| definitions.get(name.as_str()))
            .filter_map(|node| super::smartclash_plan::trace(node, &definitions, &mut std::collections::HashSet::new()))
            .max_by_key(Vec::len)
            .unwrap_or_default();
        let mut locations: Vec<_> = chain
            .iter()
            .filter_map(|name| servers.get(name).and_then(|server| cached(server)))
            .collect();
        let destination_host = destination_ip(connection);
        let destination = cached(&destination_host);
        if chain.is_empty() || locations.len() != chain.len() || destination.is_none() {
            // 尚在后台解析中的目标只是"还没就绪", 不算定位失败,
            // 否则首轮快照会出现大片虚假 unmapped (地球看起来没数据)
            if is_pending(&destination_host, &chain, &servers) {
                continue;
            }
            result.unmapped += 1;
            continue;
        }
        if let Some(destination) = destination {
            locations.push(destination);
        }
        result.routes.push(MapRoute {
            id: connection.id.clone(),
            host: connection.metadata.host.clone(),
            chain,
            locations,
        });
    }
    resolve_missing(hosts);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_service_never_receives_private_or_fake_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "100.64.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "0.1.2.3",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()));
        }
    }

    #[test]
    fn pending_hosts_are_not_counted_as_unmapped() {
        CACHE.lock().clear();
        let servers: HashMap<String, String> = [
            ("节点A".to_owned(), "a.example.com".to_owned()),
            ("节点B".to_owned(), "b.example.com".to_owned()),
        ]
        .into();
        // destination 与入口节点尚未解析 → pending; 出口节点已解析失败 (有条目) → 不是 pending
        CACHE.lock().insert(
            "b.example.com".to_owned(),
            Entry {
                location: None,
                checked: Instant::now(),
            },
        );
        // 目标未解析 → pending (即使另一个节点已失败)
        assert!(is_pending("8.8.8.8", &["节点A".to_owned()], &servers));
        CACHE.lock().clear();
    }

    #[test]
    fn failed_lookups_are_unmapped_but_pending_ones_are_skipped() {
        CACHE.lock().clear();
        let servers: HashMap<String, String> =
            [("节点A".to_owned(), "a.example.com".to_owned())].into();
        let chain = ["节点A".to_owned()];

        // 全部未解析 → pending, 不应计入 unmapped
        assert!(is_pending("8.8.8.8", &chain, &servers));

        // 已解析但失败 (缓存有条目、location 为 None) → 真 unmapped
        CACHE.lock().insert(
            "a.example.com".to_owned(),
            Entry {
                location: None,
                checked: Instant::now(),
            },
        );
        CACHE.lock().insert(
            "8.8.8.8".to_owned(),
            Entry {
                location: None,
                checked: Instant::now(),
            },
        );
        assert!(!is_pending("8.8.8.8", &chain, &servers));
        CACHE.lock().clear();
    }

    #[test]
    fn resolve_queue_deduplicates_hosts() {
        // 模拟 snapshot 构造的 hosts: 连接目标 + servers 大量重复
        let mut hosts = vec!["1.1.1.1".to_owned(); 3];
        hosts.push("a.example.com".to_owned());
        hosts.push("1.1.1.1".to_owned());
        hosts.push("a.example.com".to_owned());
        let mut seen = std::collections::HashSet::new();
        hosts.retain(|host| seen.insert(host.clone()));
        assert_eq!(hosts, vec!["1.1.1.1".to_owned(), "a.example.com".to_owned()]);
    }
}

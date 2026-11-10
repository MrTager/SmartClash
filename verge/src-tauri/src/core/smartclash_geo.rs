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

fn cached(host: &str) -> Option<Location> {
    CACHE
        .lock()
        .get(host)
        .filter(|entry| entry.checked.elapsed() < Duration::from_secs(86_400))
        .and_then(|entry| entry.location.clone())
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

fn resolve_missing(hosts: Vec<String>) {
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
            if count == 4 || Handle::global().is_exiting() {
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
        .map(|connection| connection.metadata.destination_ip.clone())
        .filter(|ip| ip.parse::<IpAddr>().is_ok_and(public_ip))
        .collect();
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
        let destination = cached(&connection.metadata.destination_ip);
        if chain.is_empty() || locations.len() != chain.len() || destination.is_none() {
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
}

use super::model::{ChainOptions, ProxyNode};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct ChainBuilder;

impl ChainBuilder {
    pub fn routes(nodes: &[ProxyNode], options: &ChainOptions) -> Result<Vec<ProxyNode>, String> {
        if !options.enabled {
            return Ok(nodes.to_vec());
        }
        let entries: Vec<&ProxyNode> = nodes
            .iter()
            .filter(|n| options.entry_ids.contains(&n.id))
            .collect();
        let exits: Vec<&ProxyNode> = nodes
            .iter()
            .filter(|n| options.exit_ids.contains(&n.id))
            .collect();
        if entries.is_empty() || exits.is_empty() {
            return Err("链式模式需要至少一个入口和一个出口节点，请重新选择已失效的节点".into());
        }
        if entries.len() * exits.len() > 128 {
            return Err("入口 × 出口最多 128 个组合，请减少候选节点".into());
        }
        let tcp = [
            "ss", "vmess", "vless", "trojan", "socks5", "http", "snell", "anytls",
        ];
        let all_tcp = entries
            .iter()
            .chain(exits.iter())
            .all(|n| tcp.contains(&n.payload.get("type").and_then(|v| v.as_str()).unwrap_or("")));
        if !all_tcp {
            return Err("链式模式暂支持 SS、VMess、VLESS、Trojan、SOCKS5、HTTP、Snell、AnyTLS；请勿选择 UDP 专用或 SSR 节点".into());
        }

        let mut result = Vec::new();
        for entry in &entries {
            for exit in &exits {
                if exit.id == entry.id {
                    continue;
                }
                let id = format!("chain-{}-{}", entry.id, exit.id);
                let name = format!("链路 · {} → {}", entry.name, exit.name);
                let mut payload = exit.payload.clone();
                payload.insert("name".into(), Value::String(name.clone()));
                // 出口经由原始入口连接，绝不引用生成的链路或策略组。
                payload.insert("dialer-proxy".into(), Value::String(entry.name.clone()));
                result.push(ProxyNode {
                    id,
                    name,
                    source_id: exit.source_id.clone(),
                    exit_name: exit.exit_name.clone(),
                    payload,
                });
            }
        }
        if result.is_empty() {
            return Err("入口和出口不能是同一个节点".into());
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainHop {
    pub id: String,
    pub name: String,
    pub source_id: String,
    pub protocol_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainTrace {
    pub hops: Vec<ChainHop>,
    pub complete: bool,
}

impl ChainTrace {
    pub fn resolve(selected: &str, raw_nodes: &[ProxyNode], definitions: &[ProxyNode]) -> ChainTrace {
        let mut index: HashMap<String, ProxyNode> = HashMap::new();
        for node in raw_nodes.iter().chain(definitions.iter()) {
            index.insert(node.name.clone(), node.clone());
        }
        let mut originals: HashMap<String, ProxyNode> = HashMap::new();
        for node in raw_nodes {
            originals.insert(node.id.clone(), node.clone());
        }

        let mut cursor = if selected.is_empty() {
            None
        } else {
            Some(selected.to_string())
        };
        let mut visited = std::collections::HashSet::new();
        let mut result: Vec<ChainHop> = Vec::new();

        while let Some(name) = cursor.clone() {
            if name.is_empty() {
                break;
            }
            if !visited.insert(name.clone()) || result.len() >= 16 {
                return ChainTrace {
                    hops: result.into_iter().rev().collect(),
                    complete: false,
                };
            }
            let Some(node) = index.get(&name) else {
                return ChainTrace {
                    hops: result.into_iter().rev().collect(),
                    complete: false,
                };
            };
            let mut identity = node.payload.clone();
            identity.remove("name");
            identity.remove("dialer-proxy");
            let canonical = serde_json::to_string(&Value::Object(identity)).unwrap_or_default();
            let fingerprint = hex_sha256(canonical.as_bytes());
            let original = originals.get(&fingerprint).unwrap_or(node);
            result.push(ChainHop {
                id: node.id.clone(),
                name: original.name.clone(),
                source_id: original.source_id.clone(),
                protocol_name: node.protocol_name(),
            });
            cursor = node
                .payload
                .get("dialer-proxy")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }

        let complete = !result.is_empty();
        ChainTrace {
            hops: result.into_iter().rev().collect(),
            complete,
        }
    }
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

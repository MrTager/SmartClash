use super::model::ProxyNode;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct RoutePlan {
    pub candidates: Vec<ProxyNode>,
    pub definitions: Vec<ProxyNode>,
}

pub struct DynamicRouter;

impl DynamicRouter {
    /// 搜索预算是工作量上限，不是固定的两跳/三跳拓扑；每一轮都会探索新路径。
    pub fn plan(
        nodes: &[ProxyNode],
        round: u32,
        budget: usize,
        allow_chains: bool,
    ) -> Result<RoutePlan, String> {
        if nodes.is_empty() || budget == 0 {
            return Err("没有可探索的节点".into());
        }
        if !allow_chains {
            let count = budget.min(nodes.len());
            let offset = ((round as usize % nodes.len()) * count) % nodes.len();
            let candidates = (0..count)
                .map(|index| nodes[(offset + index) % nodes.len()].clone())
                .collect();
            return Ok(RoutePlan {
                candidates,
                definitions: Vec::new(),
            });
        }

        let tcp = [
            "ss", "vmess", "vless", "trojan", "socks5", "http", "snell", "anytls",
        ];
        let pool: Vec<&ProxyNode> = nodes
            .iter()
            .filter(|n| tcp.contains(&n.payload.get("type").and_then(|v| v.as_str()).unwrap_or("")))
            .collect();

        let mut candidates: Vec<ProxyNode> = Vec::new();
        let mut definitions: HashMap<String, ProxyNode> = HashMap::new();
        let mut seen = std::collections::HashSet::new();

        for slot in 0..budget {
            let depth = if pool.len() < 2 {
                1
            } else {
                1 + (round as usize + slot) % pool.len().min(budget)
            };
            if depth == 1 {
                let node = &nodes[(round as usize * budget + slot) % nodes.len()];
                if seen.insert(node.id.clone()) {
                    candidates.push(node.clone());
                }
                continue;
            }

            let start = (round as usize * budget + slot) % pool.len();
            let reverse = (round as usize / pool.len().max(1)) % 2 == 1;
            let mut previous = String::new();
            let mut path_ids: Vec<String> = Vec::new();
            let mut last: Option<ProxyNode> = None;

            for index in 0..depth {
                let offset = if reverse {
                    (pool.len() - index) % pool.len()
                } else {
                    index
                };
                let node = pool[(start + offset) % pool.len()];
                path_ids.push(node.id.clone());
                if index == 0 {
                    previous = node.name.clone();
                    continue;
                }
                let digest = hex_sha256(path_ids.join("/").as_bytes());
                let name = format!("AUTO-{}", &digest[..24]);
                let mut payload = node.payload.clone();
                payload.insert("name".into(), Value::String(name.clone()));
                payload.insert("dialer-proxy".into(), Value::String(previous.clone()));
                let item = ProxyNode {
                    id: format!("auto-{digest}"),
                    name: name.clone(),
                    source_id: node.source_id.clone(),
                    exit_name: node.exit_name.clone(),
                    payload,
                };
                definitions.insert(item.id.clone(), item.clone());
                previous = name;
                last = Some(item);
            }
            if let Some(node) = last {
                if seen.insert(node.id.clone()) {
                    candidates.push(node);
                }
            }
        }

        let mut definitions: Vec<ProxyNode> = definitions.into_values().collect();
        definitions.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(RoutePlan {
            candidates,
            definitions,
        })
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

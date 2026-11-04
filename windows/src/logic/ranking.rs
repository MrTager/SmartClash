use super::model::{LinkMeasurements, NodeLatency, NodeSortOrder, ProxyNode, RoutingMode};
use std::collections::{HashMap, HashSet};

pub struct NodeRanking;

impl NodeRanking {
    pub fn sorted(
        nodes: &[ProxyNode],
        latencies: &HashMap<String, NodeLatency>,
        order: NodeSortOrder,
    ) -> Vec<ProxyNode> {
        let mut list = nodes.to_vec();
        if order == NodeSortOrder::Original {
            return list;
        }
        list.sort_by(|a, b| {
            if order == NodeSortOrder::Latency {
                let left = latencies.get(&a.id);
                let right = latencies.get(&b.id);
                let l = rank(left);
                let r = rank(right);
                if l != r {
                    return l.cmp(&r);
                }
                if l == 0 {
                    let lm = left.and_then(|v| v.milliseconds).unwrap_or(i64::MAX);
                    let rm = right.and_then(|v| v.milliseconds).unwrap_or(i64::MAX);
                    if lm != rm {
                        return lm.cmp(&rm);
                    }
                }
            }
            let name_cmp = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            if name_cmp == std::cmp::Ordering::Equal {
                a.id.cmp(&b.id)
            } else {
                name_cmp
            }
        });
        list
    }

    /// 新鲜的可用节点优先，然后按来源轮转，避免一次性耗尽单一订阅。
    pub fn connection_candidates(
        nodes: &[ProxyNode],
        latencies: &HashMap<String, NodeLatency>,
        now: u64,
    ) -> Vec<ProxyNode> {
        let known: Vec<ProxyNode> = Self::sorted(
            &nodes
                .iter()
                .filter(|n| {
                    latencies
                        .get(&n.id)
                        .map(|l| l.reachable() && l.is_fresh(now, 300_000))
                        .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>(),
            latencies,
            NodeSortOrder::Latency,
        );
        let mut seen: HashSet<String> = known.iter().map(|n| n.id.clone()).collect();
        let mut source_order: Vec<String> = Vec::new();
        let mut pools: HashMap<String, Vec<ProxyNode>> = HashMap::new();
        for node in nodes {
            if seen.contains(&node.id) {
                continue;
            }
            if !pools.contains_key(&node.source_id) {
                source_order.push(node.source_id.clone());
            }
            pools
                .entry(node.source_id.clone())
                .or_default()
                .push(node.clone());
        }
        let mut result = known;
        let rounds = pools.values().map(|v| v.len()).max().unwrap_or(0);
        for index in 0..rounds {
            for source in &source_order {
                if let Some(pool) = pools.get(source) {
                    if index < pool.len() && seen.insert(pool[index].id.clone()) {
                        result.push(pool[index].clone());
                    }
                }
            }
        }
        result
    }
}

fn rank(latency: Option<&NodeLatency>) -> u8 {
    match latency {
        Some(l) if l.reachable() => 0,
        None => 1,
        _ => 2,
    }
}

pub struct ModeRanking;

impl ModeRanking {
    pub fn best(
        nodes: &[ProxyNode],
        mode: RoutingMode,
        latencies: &HashMap<String, NodeLatency>,
        measurements: &HashMap<String, LinkMeasurements>,
    ) -> Vec<ProxyNode> {
        let ordered = NodeRanking::sorted(nodes, latencies, NodeSortOrder::Latency);
        match mode {
            RoutingMode::LowLatency => ordered,
            RoutingMode::HighSpeed => {
                let mut list: Vec<ProxyNode> = ordered
                    .into_iter()
                    .filter(|n| {
                        measurements
                            .get(&n.id)
                            .and_then(|m| m.throughput.as_ref())
                            .map(|t| t.bytes_per_second > 0.0)
                            .unwrap_or(false)
                    })
                    .collect();
                list.sort_by(|a, b| {
                    let left = measurements[&a.id].throughput.as_ref().unwrap().bytes_per_second;
                    let right = measurements[&b.id].throughput.as_ref().unwrap().bytes_per_second;
                    if left == right {
                        a.id.cmp(&b.id)
                    } else {
                        right.partial_cmp(&left).unwrap_or(std::cmp::Ordering::Equal)
                    }
                });
                list
            }
            RoutingMode::Ai => ordered
                .into_iter()
                .filter(|n| {
                    measurements
                        .get(&n.id)
                        .and_then(|m| m.openai_reachable)
                        .unwrap_or(false)
                })
                .collect(),
        }
    }
}

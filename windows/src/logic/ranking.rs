use super::model::{LinkMeasurements, NodeLatency, NodeSortOrder, ProxyNode, RoutingMode};
use std::collections::{HashMap, HashSet};

pub struct NodeRanking;

impl NodeRanking {
    /// Labels are hints, not proof of exit IP location or OpenAI support.
    pub fn is_hong_kong(node: &ProxyNode) -> bool {
        let label = node.exit_name.to_lowercase();
        if label.contains("香港") || label.contains("\u{1f1ed}\u{1f1f0}") {
            return true;
        }
        let words: Vec<&str> = label
            .split(|ch: char| !ch.is_alphabetic())
            .filter(|s| !s.is_empty())
            .collect();
        words
            .iter()
            .any(|word| matches!(*word, "hk" | "hkg" | "hongkong"))
            || words.windows(2).any(|pair| pair == ["hong", "kong"])
    }

    /// Stable partition; retain all fallback exits and metric/source order within each tier.
    pub fn prefer_non_hong_kong(nodes: &[ProxyNode]) -> Vec<ProxyNode> {
        nodes
            .iter()
            .filter(|node| !Self::is_hong_kong(node))
            .chain(nodes.iter().filter(|node| Self::is_hong_kong(node)))
            .cloned()
            .collect()
    }

    /// Complete both passes of the preferred tier before considering fallback exits.
    pub fn probe_passes(quick: &[ProxyNode], candidates: &[ProxyNode]) -> Vec<Vec<ProxyNode>> {
        [false, true]
            .into_iter()
            .flat_map(|fallback| {
                [quick, candidates].into_iter().map(move |source| {
                    source
                        .iter()
                        .filter(|node| Self::is_hong_kong(node) == fallback)
                        .cloned()
                        .collect()
                })
            })
            .collect()
    }

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
        Self::prefer_non_hong_kong(&result)
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
        let ranked = match mode {
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
                    let left = measurements[&a.id]
                        .throughput
                        .as_ref()
                        .unwrap()
                        .bytes_per_second;
                    let right = measurements[&b.id]
                        .throughput
                        .as_ref()
                        .unwrap()
                        .bytes_per_second;
                    if left == right {
                        a.id.cmp(&b.id)
                    } else {
                        right
                            .partial_cmp(&left)
                            .unwrap_or(std::cmp::Ordering::Equal)
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
        };
        NodeRanking::prefer_non_hong_kong(&ranked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::chain::ChainBuilder;
    use crate::logic::dynamic_router::DynamicRouter;
    use crate::logic::model::{ChainOptions, ThroughputSample, TrafficMode};
    use serde_json::json;

    fn node(id: &str, label: &str, source: &str) -> ProxyNode {
        ProxyNode {
            id: id.into(),
            name: label.into(),
            exit_name: label.into(),
            source_id: source.into(),
            payload: json!({"name": label, "type": "ss", "server": format!("{id}.example.com"),
                "port": 443, "cipher": "aes-128-gcm", "password": "fixture"})
            .as_object()
            .unwrap()
            .clone(),
        }
    }

    fn ids(nodes: &[ProxyNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn recognizes_common_hong_kong_labels_and_suffixes() {
        for label in [
            "中国香港 01",
            "HK01",
            "[hK] 02",
            "HKG-03",
            "Hong Kong 04",
            "Hong-Kong",
            "HongKong",
            "\u{1f1ed}\u{1f1f0} 05",
            "专线 · HK 06",
        ] {
            assert!(NodeRanking::is_hong_kong(&node("n", label, "s")), "{label}");
        }
    }

    #[test]
    fn does_not_match_hk_inside_other_words_or_fingerprint() {
        for label in [
            "Tokyo",
            "Singapore",
            "US 01",
            "SHK-01",
            "HKT 02",
            "HKEX",
            "日本 · abcd12345678",
        ] {
            assert!(
                !NodeRanking::is_hong_kong(&node("n", label, "s")),
                "{label}"
            );
        }
    }

    #[test]
    fn preference_is_stable_and_keeps_fallback_nodes() {
        let nodes = vec![
            node("hk1", "HK01", "a"),
            node("jp", "JP01", "a"),
            node("hk2", "中国香港02", "b"),
            node("sg", "SG01", "b"),
        ];
        let ranked = NodeRanking::prefer_non_hong_kong(&nodes);
        assert_eq!(ids(&ranked), ["jp", "sg", "hk1", "hk2"]);
        assert_eq!(NodeRanking::prefer_non_hong_kong(&nodes[..1]).len(), 1);
        assert!(NodeRanking::prefer_non_hong_kong(&[]).is_empty());
    }

    #[test]
    fn connection_preference_overrides_faster_fresh_hong_kong_result() {
        let nodes = vec![node("hk", "HK01", "a"), node("jp", "JP01", "a")];
        let latencies = HashMap::from([(
            "hk".into(),
            NodeLatency {
                milliseconds: Some(5),
                checked: 1000,
                failure: None,
            },
        )]);
        assert_eq!(
            ids(&NodeRanking::connection_candidates(
                &nodes, &latencies, 1000
            )),
            ["jp", "hk"]
        );
        // The node list's explicit latency sort is not changed by automatic routing preferences.
        assert_eq!(
            ids(&NodeRanking::sorted(
                &nodes,
                &latencies,
                NodeSortOrder::Latency
            )),
            ["hk", "jp"]
        );
    }

    #[test]
    fn connection_preference_preserves_source_rotation_within_tiers() {
        let nodes = vec![
            node("hk1", "HK01", "a"),
            node("jp", "JP01", "a"),
            node("us", "US01", "a"),
            node("hk2", "HK02", "b"),
            node("sg", "SG01", "b"),
        ];
        assert_eq!(
            ids(&NodeRanking::connection_candidates(
                &nodes,
                &HashMap::new(),
                1000
            )),
            ["jp", "sg", "us", "hk1", "hk2"]
        );
    }

    #[test]
    fn quick_hong_kong_pass_cannot_preempt_full_preferred_pass() {
        let hk = node("hk", "HK01", "a");
        let jp = node("jp", "JP01", "a");
        let sg = node("sg", "SG01", "b");
        let passes = NodeRanking::probe_passes(&[jp.clone(), hk.clone()], &[hk, jp, sg]);
        assert_eq!(
            passes.iter().map(|pass| ids(pass)).collect::<Vec<_>>(),
            vec![vec!["jp"], vec!["jp", "sg"], vec!["hk"], vec!["hk"]]
        );
        assert!(passes.iter().all(
            |pass| pass.iter().all(|node| NodeRanking::is_hong_kong(node))
                || pass.iter().all(|node| !NodeRanking::is_hong_kong(node))
        ));
    }

    #[test]
    fn all_routing_modes_prefer_other_regions_but_require_measurements() {
        let nodes = vec![node("hk", "HK01", "a"), node("jp", "JP01", "a")];
        let latencies = HashMap::from([
            ("hk".into(), NodeLatency::new(Some(5), None)),
            ("jp".into(), NodeLatency::new(Some(90), None)),
        ]);
        let measurements = HashMap::from([
            (
                "hk".into(),
                LinkMeasurements {
                    throughput: Some(ThroughputSample {
                        bytes_per_second: 900.0,
                        downloaded_bytes: 1_048_576,
                        checked: 1000,
                    }),
                    openai_reachable: Some(true),
                    openai_checked: Some(1000),
                },
            ),
            (
                "jp".into(),
                LinkMeasurements {
                    throughput: Some(ThroughputSample {
                        bytes_per_second: 100.0,
                        downloaded_bytes: 1_048_576,
                        checked: 1000,
                    }),
                    openai_reachable: Some(true),
                    openai_checked: Some(1000),
                },
            ),
        ]);
        for mode in RoutingMode::ALL {
            assert_eq!(
                ids(&ModeRanking::best(&nodes, mode, &latencies, &measurements)),
                ["jp", "hk"]
            );
        }
        assert!(ModeRanking::best(&nodes, RoutingMode::Ai, &latencies, &HashMap::new()).is_empty());
        assert!(
            ModeRanking::best(&nodes, RoutingMode::HighSpeed, &latencies, &HashMap::new())
                .is_empty()
        );
    }

    #[test]
    fn verified_hong_kong_exit_remains_an_ai_fallback() {
        let nodes = vec![node("jp", "JP01", "a"), node("hk", "HK01", "a")];
        let measurements = HashMap::from([
            (
                "jp".into(),
                LinkMeasurements {
                    openai_reachable: Some(false),
                    ..Default::default()
                },
            ),
            (
                "hk".into(),
                LinkMeasurements {
                    openai_reachable: Some(true),
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(
            ids(&ModeRanking::best(
                &nodes,
                RoutingMode::Ai,
                &HashMap::new(),
                &measurements
            )),
            ["hk"]
        );
    }

    #[test]
    fn explicit_chain_preference_uses_exit_not_entry() {
        let nodes = vec![node("hk", "HK01", "a"), node("jp", "JP01", "a")];
        let chain = |entry: &str, exit: &str| ChainOptions {
            enabled: true,
            entry_ids: [entry.into()].into(),
            exit_ids: [exit.into()].into(),
        };
        let jp_exit = ChainBuilder::routes(&nodes, &chain("hk", "jp")).unwrap();
        let hk_exit = ChainBuilder::routes(&nodes, &chain("jp", "hk")).unwrap();
        assert!(!NodeRanking::is_hong_kong(&jp_exit[0]));
        assert!(NodeRanking::is_hong_kong(&hk_exit[0]));
    }

    #[test]
    fn anonymous_auto_chain_keeps_original_exit_region() {
        let nodes = vec![node("hk", "HK01", "a"), node("jp", "JP01", "a")];
        let hk_plan = DynamicRouter::plan(&nodes, 0, 2, true).unwrap();
        let hk_exit = hk_plan
            .candidates
            .iter()
            .find(|n| n.name.starts_with("AUTO-"))
            .unwrap();
        assert!(NodeRanking::is_hong_kong(hk_exit));
        let jp_plan = DynamicRouter::plan(&nodes, 1, 2, true).unwrap();
        let jp_exit = jp_plan
            .candidates
            .iter()
            .find(|n| n.name.starts_with("AUTO-"))
            .unwrap();
        assert!(!NodeRanking::is_hong_kong(jp_exit));
    }

    #[test]
    fn manual_preferred_hong_kong_node_is_not_overridden_or_leaked_to_payload() {
        let nodes = vec![node("hk", "HK01", "a"), node("jp", "JP01", "a")];
        let data = crate::logic::configuration::make(
            &nodes,
            "fixture",
            false,
            None,
            &[],
            None,
            Some("HK01"),
            TrafficMode::Rule,
        )
        .unwrap();
        let config: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(config["proxy-groups"][0]["proxies"][0], "HK01");
        assert!(config["proxies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|proxy| proxy.get("exit_name").is_none() && proxy.get("exitName").is_none()));
    }
}

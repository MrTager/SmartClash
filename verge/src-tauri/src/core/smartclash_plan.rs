use crate::config::smartclash::{Mode, Settings};
use anyhow::{Result, bail, ensure};
use serde_yaml_ng::{Mapping, Value};
use std::collections::{HashMap, HashSet};

pub const GROUP: &str = "SMARTCLASH";
const PREFIX: &str = "SMARTCLASH::";
const TCP_TYPES: &[&str] = &["ss", "vmess", "vless", "trojan", "socks5", "http", "snell", "anytls"];

#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub chain: Vec<String>,
    pub hong_kong: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub candidates: Vec<Candidate>,
    pub definitions: Vec<Value>,
    pub providers: Vec<String>,
}

pub fn is_hong_kong(label: &str) -> bool {
    let label = label.to_lowercase();
    let words: Vec<_> = label
        .split(|ch: char| !ch.is_alphabetic())
        .filter(|s| !s.is_empty())
        .collect();
    label.contains("香港")
        || label.contains("\u{1f1ed}\u{1f1f0}")
        || words.iter().any(|word| matches!(*word, "hk" | "hkg" | "hongkong"))
        || words.windows(2).any(|pair| pair == ["hong", "kong"])
}

fn name(value: &Value) -> Option<&str> {
    value.get("name").and_then(Value::as_str)
}

pub(crate) fn trace(node: &Value, nodes: &HashMap<&str, &Value>, seen: &mut HashSet<String>) -> Option<Vec<String>> {
    let name = name(node)?;
    if !seen.insert(name.to_owned()) || seen.len() > 16 {
        return None;
    }
    let mut chain = match node.get("dialer-proxy").and_then(Value::as_str) {
        Some("DIRECT") | None => Vec::new(),
        Some(previous) => trace(nodes.get(previous)?, nodes, seen)?,
    };
    chain.push(name.to_owned());
    Some(chain)
}

pub fn build(config: &Mapping, settings: &Settings) -> Result<Plan> {
    settings.validate()?;
    if !settings.enabled {
        return Ok(Plan::default());
    }
    let definitions = config
        .get("proxies")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let mut nodes = HashMap::new();
    for node in &definitions {
        if let Some(name) = name(node) {
            ensure!(
                name != GROUP && !name.starts_with(PREFIX),
                "订阅占用了 SmartClash 保留名称：{name}"
            );
            ensure!(nodes.insert(name, node).is_none(), "订阅节点名称重复：{name}");
        }
    }
    for group in config
        .get("proxy-groups")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
    {
        if let Some(name) = name(group) {
            ensure!(
                name != GROUP && !name.starts_with(PREFIX),
                "订阅策略组占用了 SmartClash 保留名称：{name}"
            );
        }
    }
    let providers = config
        .get("proxy-providers")
        .and_then(Value::as_mapping)
        .into_iter()
        .flat_map(|providers| providers.keys())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut plan = Plan {
        candidates: Vec::new(),
        definitions: definitions.clone(),
        providers,
    };
    if settings.chain_enabled {
        for node_name in settings.entry_nodes.iter().chain(&settings.exit_nodes) {
            let node = nodes
                .get(node_name.as_str())
                .ok_or_else(|| anyhow::anyhow!("链路节点不存在或仅来自 provider：{node_name}"))?;
            ensure!(
                TCP_TYPES.contains(&node.get("type").and_then(Value::as_str).unwrap_or("")),
                "链路仅支持显式 TCP 节点：{node_name}"
            );
            ensure!(
                node.get("dialer-proxy").is_none(),
                "显式链路不能叠加已有 dialer-proxy：{node_name}"
            );
        }
        for (i, entry) in settings.entry_nodes.iter().enumerate() {
            for (j, exit) in settings.exit_nodes.iter().enumerate() {
                if entry == exit {
                    continue;
                }
                let name = format!("{PREFIX}{i}:{j} · {entry} → {exit}");
                let mut clone = nodes[exit.as_str()]
                    .as_mapping()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("出口不是节点映射"))?;
                clone.insert("name".into(), name.clone().into());
                clone.insert("dialer-proxy".into(), entry.clone().into());
                clone.insert("udp".into(), false.into());
                plan.definitions.push(clone.into());
                plan.candidates.push(Candidate {
                    name,
                    chain: vec![entry.clone(), exit.clone()],
                    hong_kong: is_hong_kong(exit),
                });
            }
        }
        ensure!(!plan.candidates.is_empty(), "入口和出口不能相同");
        plan.providers.clear();
    } else {
        for node in &definitions {
            if matches!(
                node.get("type").and_then(Value::as_str),
                None | Some("direct" | "reject" | "pass" | "dns")
            ) {
                continue;
            }
            let Some(chain) = trace(node, &nodes, &mut HashSet::new()) else {
                continue;
            };
            let exit = chain.last().ok_or_else(|| anyhow::anyhow!("链路缺少末跳"))?;
            plan.candidates.push(Candidate {
                name: name(node).unwrap_or_default().to_owned(),
                hong_kong: is_hong_kong(exit),
                chain,
            });
        }
    }
    if settings.explore_multihop && !settings.chain_enabled {
        let pool: Vec<_> = definitions
            .iter()
            .filter(|node| {
                TCP_TYPES.contains(&node.get("type").and_then(Value::as_str).unwrap_or(""))
                    && node.get("dialer-proxy").is_none()
            })
            .collect();
        let mut generated = 0;
        for offset in 1..pool.len().min(4) {
            for (index, entry) in pool.iter().enumerate() {
                if generated == 48 {
                    break;
                }
                let exit = pool[(index + offset) % pool.len()];
                let entry_name = name(entry).unwrap_or_default();
                let exit_name = name(exit).unwrap_or_default();
                let route_name = format!("{PREFIX}auto:{offset}:{index} · {entry_name} → {exit_name}");
                let mut route = exit
                    .as_mapping()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("探索出口不是节点映射"))?;
                route.insert("name".into(), route_name.clone().into());
                route.insert("dialer-proxy".into(), entry_name.into());
                route.insert("udp".into(), false.into());
                plan.definitions.push(route.into());
                plan.candidates.push(Candidate {
                    name: route_name,
                    chain: vec![entry_name.into(), exit_name.into()],
                    hong_kong: is_hong_kong(exit_name),
                });
                generated += 1;
            }
            if generated == 48 {
                break;
            }
        }
    }
    if settings.prefer_non_hong_kong {
        // Exit labels are preference hints; measurements remain the eligibility gate.
        plan.candidates.sort_by_key(|candidate| candidate.hong_kong);
    }
    ensure!(
        !plan.candidates.is_empty() || !plan.providers.is_empty(),
        "没有可用节点或 provider；未注入空 SmartClash 组"
    );
    Ok(plan)
}

pub fn inject(config: &mut Mapping, plan: &Plan, selected: Option<&str>) -> Result<()> {
    if plan.candidates.is_empty() && plan.providers.is_empty() {
        return Ok(());
    }
    let original_count = config.get("proxies").and_then(Value::as_sequence).map_or(0, Vec::len);
    let mut proxies = config
        .get("proxies")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    proxies.extend(plan.definitions.iter().skip(original_count).cloned());
    let mut names: Vec<Value> = plan
        .candidates
        .iter()
        .map(|candidate| Value::from(candidate.name.clone()))
        .collect();
    if let Some(index) = names.iter().position(|name| name.as_str() == selected) {
        let preferred = names.remove(index);
        names.insert(0, preferred);
    }
    let mut group = Mapping::new();
    group.insert("name".into(), GROUP.into());
    group.insert("type".into(), "select".into());
    group.insert("proxies".into(), names.into());
    if !plan.providers.is_empty() {
        group.insert(
            "use".into(),
            plan.providers
                .iter()
                .cloned()
                .map(Value::from)
                .collect::<Vec<_>>()
                .into(),
        );
    }
    let mut groups = config
        .get("proxy-groups")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    for existing in &mut groups {
        let Some(existing) = existing.as_mapping_mut() else {
            continue;
        };
        if existing.get("type").and_then(Value::as_str) != Some("select") {
            continue;
        }
        if let Some(Value::Sequence(choices)) = existing.get_mut("proxies") {
            if !choices.iter().any(|choice| choice.as_str() == Some(GROUP)) {
                choices.push(GROUP.into());
            }
        } else if !existing.contains_key("proxies") {
            existing.insert("proxies".into(), vec![Value::from(GROUP)].into());
        }
    }
    groups.push(group.into());
    config.insert("proxies".into(), proxies.into());
    config.insert("proxy-groups".into(), groups.into());
    Ok(())
}

pub fn eligible(mode: Mode, latency: Option<u64>, speed: Option<f64>, ai: Option<bool>) -> bool {
    latency.is_some()
        && match mode {
            Mode::LowLatency => true,
            Mode::HighSpeed => speed.is_some_and(|speed| speed.is_finite() && speed > 0.0),
            Mode::Ai => ai == Some(true),
        }
}

pub fn require_inline(plan: &Plan) -> Result<()> {
    if plan.candidates.is_empty() {
        bail!("当前只有 provider 节点；可手动选择 SMARTCLASH，隔离优化暂需内联节点");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Result<Mapping> {
        Ok(serde_yaml_ng::from_str(
            r#"
proxies:
  - {name: HK01, type: ss, server: 192.0.2.1, port: 443, cipher: aes-128-gcm, password: fixture}
  - {name: JP01, type: ss, server: 192.0.2.2, port: 443, cipher: aes-128-gcm, password: fixture}
proxy-groups:
  - {name: Manual, type: select, proxies: [HK01, JP01, DIRECT]}
  - {name: Auto, type: url-test, proxies: [HK01, JP01]}
rules: ["DOMAIN,example.test,Manual", "MATCH,Manual"]
dns: {enable: true, nameserver: [1.1.1.1]}
tun: {enable: true, stack: mixed}
external-controller: '127.0.0.1:9090'
secret: fixture
"#,
        )?)
    }

    #[test]
    fn injection_preserves_rules_control_plane_and_manual_default() -> Result<()> {
        let original = config()?;
        let mut result = original.clone();
        let settings = Settings {
            enabled: true,
            ..Settings::default()
        };
        let plan = build(&result, &settings)?;
        inject(&mut result, &plan, None)?;
        for key in ["rules", "dns", "tun", "external-controller", "secret", "proxies"] {
            assert_eq!(result[key], original[key]);
        }
        assert_eq!(result["proxy-groups"][0]["proxies"][0].as_str(), Some("HK01"));
        assert_eq!(result["proxy-groups"][0]["proxies"][3].as_str(), Some(GROUP));
        assert_eq!(result["proxy-groups"][1], original["proxy-groups"][1]);
        assert_eq!(result["proxy-groups"][2]["proxies"][0].as_str(), Some("JP01"));
        Ok(())
    }

    #[test]
    fn explicit_chain_is_tcp_and_region_uses_exit_not_entry() -> Result<()> {
        let original = config()?;
        let settings = Settings {
            enabled: true,
            chain_enabled: true,
            entry_nodes: vec!["HK01".into()],
            exit_nodes: vec!["JP01".into()],
            ..Settings::default()
        };
        let plan = build(&original, &settings)?;
        assert!(!plan.candidates[0].hong_kong);
        assert_eq!(plan.candidates[0].chain, ["HK01", "JP01"]);
        assert_eq!(plan.definitions[2]["dialer-proxy"].as_str(), Some("HK01"));
        assert_eq!(plan.definitions[2]["udp"].as_bool(), Some(false));
        assert_eq!(plan.definitions[1], original["proxies"][1]);
        Ok(())
    }

    #[test]
    fn disabled_and_name_collisions_never_take_over_subscription() -> Result<()> {
        let mut original = config()?;
        let disabled = build(&original, &Settings::default())?;
        let before = original.clone();
        inject(&mut original, &disabled, None)?;
        assert_eq!(original, before);
        original
            .get_mut("proxies")
            .and_then(Value::as_sequence_mut)
            .unwrap()
            .push(serde_yaml_ng::from_str("{name: SMARTCLASH, type: ss}")?);
        assert!(
            build(
                &original,
                &Settings {
                    enabled: true,
                    ..Settings::default()
                }
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn labels_are_not_ai_or_throughput_evidence() {
        assert!(!eligible(Mode::Ai, Some(5), None, None));
        assert!(!eligible(Mode::HighSpeed, Some(5), None, Some(true)));
        assert!(!eligible(Mode::Ai, None, None, Some(true)));
        assert!(eligible(Mode::Ai, Some(5), None, Some(true)));
        for label in ["HK01", "HKG-2", "中国香港", "Hong Kong", "Hong-Kong"] {
            assert!(is_hong_kong(label));
        }
        for label in ["HKT", "SHK-1", "HKEX", "JP01"] {
            assert!(!is_hong_kong(label));
        }
    }

    #[test]
    fn automatic_exploration_keeps_raw_candidates_and_generates_acyclic_tcp_routes() -> Result<()> {
        let original = config()?;
        let plan = build(
            &original,
            &Settings {
                enabled: true,
                explore_multihop: true,
                ..Settings::default()
            },
        )?;
        assert_eq!(plan.candidates.len(), 4);
        assert!(plan.candidates.iter().any(|candidate| candidate.name == "JP01"));
        assert!(
            plan.candidates
                .iter()
                .any(|candidate| candidate.chain == ["HK01", "JP01"])
        );
        assert!(
            plan.candidates
                .iter()
                .any(|candidate| candidate.chain == ["JP01", "HK01"])
        );
        for node in plan.definitions.iter().skip(2) {
            assert_eq!(node["udp"].as_bool(), Some(false));
        }
        Ok(())
    }
}

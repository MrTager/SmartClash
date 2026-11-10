use crate::config::{IProfiles, smartclash::Settings};
use anyhow::{Context as _, Result, ensure};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, HashSet};

pub async fn aggregate(config: &mut Mapping, profiles: &IProfiles, settings: &Settings) -> Result<()> {
    if !settings.enabled || settings.source_profiles.is_empty() {
        return Ok(());
    }
    let mut nodes = config
        .get("proxies")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let mut names: HashSet<String> = nodes
        .iter()
        .filter_map(|node| node.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let mut seen: HashSet<String> = nodes.iter().map(fingerprint).collect::<Result<_>>()?;
    for uid in &settings.source_profiles {
        if profiles.current.as_ref().is_some_and(|current| current.as_str() == uid) {
            continue;
        }
        let item = profiles
            .items
            .as_ref()
            .into_iter()
            .flatten()
            .find(|item| item.uid.as_ref().is_some_and(|id| id.as_str() == uid))
            .context("聚合订阅已被删除，请重新选择来源")?;
        ensure!(
            matches!(item.itype.as_deref(), Some("remote" | "local")),
            "只能聚合远程或本地订阅"
        );
        let source: Mapping = serde_yaml_ng::from_str(&item.read_file().await?).context("无法解析聚合订阅缓存")?;
        append(
            &mut nodes,
            &mut names,
            &mut seen,
            source
                .get("proxies")
                .and_then(Value::as_sequence)
                .map_or(&[], Vec::as_slice),
        )?;
    }
    config.insert("proxies".into(), nodes.into());
    Ok(())
}

fn fingerprint(node: &Value) -> Result<String> {
    let mut identity = node.as_mapping().cloned().context("订阅节点不是映射")?;
    identity.remove("name");
    let canonical = serde_json::to_vec(&serde_json::to_value(identity)?)?;
    Ok(Sha256::digest(canonical)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn append(
    nodes: &mut Vec<Value>,
    names: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    source: &[Value],
) -> Result<()> {
    let mut renamed = HashMap::new();
    for node in source {
        let name = node.get("name").and_then(Value::as_str).context("聚合节点缺少名称")?;
        ensure!(!renamed.contains_key(name), "同一聚合订阅内存在重名节点");
        let id = fingerprint(node)?;
        let mut target = name.to_owned();
        if names.contains(name) {
            target = format!("{name} · {}", &id[..10]);
        }
        let mut suffix = 2;
        while names.contains(&target) {
            target = format!("{name} · {}-{suffix}", &id[..10]);
            suffix += 1;
        }
        names.insert(target.clone());
        renamed.insert(name.to_owned(), target);
    }
    for node in source {
        let mut node = node.as_mapping().cloned().context("聚合节点不是映射")?;
        let original = node
            .get("name")
            .and_then(Value::as_str)
            .context("聚合节点缺少名称")?
            .to_owned();
        if let Some(previous) = node.get("dialer-proxy").and_then(Value::as_str) {
            if previous != "DIRECT" {
                let mapped = renamed
                    .get(previous)
                    .context("聚合节点引用了来源策略组；请使用显式入口和出口节点")?;
                node.insert("dialer-proxy".into(), mapped.clone().into());
            }
        }
        node.insert("name".into(), renamed[&original].clone().into());
        let node = Value::Mapping(node);
        // Referenced entries retain their alias so dialer-proxy never points to a missing node.
        let referenced = source
            .iter()
            .any(|candidate| candidate.get("dialer-proxy").and_then(Value::as_str) == Some(&original));
        if seen.insert(fingerprint(&node)?) || referenced {
            nodes.push(node);
        }
    }
    Ok(())
}

pub fn rules(config: &mut Mapping) -> Result<()> {
    let mut providers = config
        .get("rule-providers")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    for (name, file, behavior) in [
        ("smartclash-cn", "direct", "domain"),
        ("smartclash-custom", "custom-direct", "domain"),
        ("smartclash-ip", "cncidr", "ipcidr"),
    ] {
        ensure!(!providers.contains_key(name), "订阅占用了 SmartClash 规则集名称");
        let provider = serde_json::json!({
            "type": "http", "behavior": behavior, "format": "yaml", "interval": 3600, "proxy": "SMARTCLASH",
            "url": format!("https://raw.githubusercontent.com/MrTager/SmartClash-Rules/main/rules/{file}.yaml"),
            "path": format!("./rules/{name}.yaml"),
        });
        providers.insert(name.into(), serde_yaml_ng::to_value(provider)?);
    }
    config.insert("rule-providers".into(), providers.into());
    config.insert(
        "rules".into(),
        serde_yaml_ng::to_value(vec![
            "DOMAIN-SUFFIX,local,DIRECT",
            "DOMAIN-SUFFIX,lan,DIRECT",
            "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve",
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
            "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
            "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
            "IP-CIDR6,::1/128,DIRECT,no-resolve",
            "IP-CIDR6,fc00::/7,DIRECT,no-resolve",
            "IP-CIDR6,fe80::/10,DIRECT,no-resolve",
            "RULE-SET,smartclash-cn,DIRECT",
            "RULE-SET,smartclash-custom,DIRECT",
            "RULE-SET,smartclash-ip,DIRECT,no-resolve",
            "MATCH,SMARTCLASH",
        ])?,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_keeps_collisions_distinct_and_rewrites_chain_references() -> Result<()> {
        let mut nodes: Vec<Value> =
            serde_yaml_ng::from_str("[{name: Entry, type: ss, server: 192.0.2.1, password: fixture}]")?;
        let source = serde_yaml_ng::from_str::<Vec<Value>>(
            "[{name: Entry, type: ss, server: 192.0.2.2, password: fixture}, {name: Exit, type: ss, server: 192.0.2.3, dialer-proxy: Entry}]",
        )?;
        let mut names = HashSet::from(["Entry".into()]);
        let mut seen = nodes.iter().map(fingerprint).collect::<Result<HashSet<_>>>()?;
        append(&mut nodes, &mut names, &mut seen, &source)?;
        assert_eq!(nodes.len(), 3);
        assert_ne!(nodes[0]["name"], nodes[1]["name"]);
        assert_eq!(nodes[2]["dialer-proxy"], nodes[1]["name"]);
        assert_eq!(nodes[0]["server"].as_str(), Some("192.0.2.1"));
        Ok(())
    }
}

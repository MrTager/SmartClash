use crate::config::{IProfiles, smartclash::Settings};
use anyhow::{Context as _, Result, ensure};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, HashSet};

fn aggregate_sources(profiles: &IProfiles) -> impl Iterator<Item = &crate::config::PrfItem> {
    profiles.items.as_ref().into_iter().flatten().filter(|item| {
        matches!(item.itype.as_deref(), Some("remote" | "local")) && item.uid.as_ref() != profiles.current.as_ref()
    })
}

pub async fn aggregate(config: &mut Mapping, profiles: &IProfiles, _settings: &Settings) -> Result<()> {
    // 来源在每次生成时动态读取，新增订阅无需手工加入固定 UID 列表。
    let mut nodes = config
        .get("proxies")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let mut names: HashSet<String> = nodes
        .iter()
        .filter_map(|node| node.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    // Proxy and group names share Mihomo's namespace, including our generated group.
    names.insert("ALL-SUBSCRIPTIONS".to_owned());
    names.extend(
        config
            .get("proxy-groups")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .filter_map(|group| group.get("name").and_then(Value::as_str).map(str::to_owned)),
    );
    let mut seen: HashSet<String> = nodes.iter().map(fingerprint).collect::<Result<_>>()?;
    for item in aggregate_sources(profiles) {
        let source: Mapping = serde_yaml_ng::from_str(&item.read_file().await?).context("无法解析聚合订阅缓存")?;
        merge_providers(config, &source, item.uid.as_deref().context("聚合订阅缺少 UID")?)?;
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
    let choices: Vec<Value> = nodes.iter().filter_map(|node| node.get("name").cloned()).collect();
    let providers: Vec<Value> = config
        .get("proxy-providers")
        .and_then(Value::as_mapping)
        .into_iter()
        .flat_map(|items| items.keys().cloned())
        .collect();
    let mut groups = config
        .get("proxy-groups")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    ensure!(
        !groups
            .iter()
            .any(|group| group.get("name").and_then(Value::as_str) == Some("ALL-SUBSCRIPTIONS")),
        "订阅占用了全量聚合组名称 ALL-SUBSCRIPTIONS"
    );
    if !choices.is_empty() || !providers.is_empty() {
        for group in &mut groups {
            if group.get("type").and_then(Value::as_str) == Some("select") {
                if let Some(Value::Sequence(choices)) = group.get_mut("proxies") {
                    choices.push("ALL-SUBSCRIPTIONS".into());
                }
            }
        }
        groups.push(serde_yaml_ng::to_value(serde_json::json!({
            "name": "ALL-SUBSCRIPTIONS", "type": "select", "proxies": choices, "use": providers
        }))?);
    }
    config.insert("proxy-groups".into(), groups.into());
    config.insert("proxies".into(), nodes.into());
    Ok(())
}

fn merge_providers(config: &mut Mapping, source: &Mapping, uid: &str) -> Result<()> {
    let mut providers = config
        .get("proxy-providers")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    for (name, provider) in source
        .get("proxy-providers")
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
    {
        let name = name.as_str().context("provider 名称无效")?;
        let mut provider = provider.as_mapping().cloned().context("provider 配置无效")?;
        if provider.get("type").and_then(Value::as_str) == Some("http") {
            let digest = Sha256::digest(format!("{uid}:{name}").as_bytes());
            let id: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            provider.insert("path".into(), format!("./providers/smartclash-{id}.yaml").into());
        } else if let Some(path) = provider.get("path").and_then(Value::as_str) {
            let path = std::path::Path::new(path);
            if path.is_relative() {
                provider.insert(
                    "path".into(),
                    crate::utils::dirs::app_home_dir()?
                        .join(path)
                        .to_string_lossy()
                        .into_owned()
                        .into(),
                );
            }
        }
        ensure!(
            provider
                .get("proxy")
                .is_none_or(|proxy| proxy.as_str() == Some("DIRECT")),
            "聚合 provider 引用了来源策略组，请移除 provider.proxy 后重试"
        );
        providers.insert(format!("{uid}::{name}").into(), provider.into());
    }
    if !providers.is_empty() {
        config.insert("proxy-providers".into(), providers.into());
    }
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
    fn new_subscriptions_join_without_a_saved_source_selection() -> Result<()> {
        let mut profiles: IProfiles = serde_yaml_ng::from_str(
            "current: first\nitems: [{uid: first, type: remote}, {uid: second, type: local}, {uid: script, type: script}]",
        )?;
        assert_eq!(aggregate_sources(&profiles).count(), 1);
        profiles
            .items
            .as_mut()
            .unwrap()
            .push(serde_yaml_ng::from_str("{uid: third, type: remote}")?);
        assert_eq!(aggregate_sources(&profiles).count(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn aggregation_is_available_without_smart_routing() -> Result<()> {
        let mut config: Mapping =
            serde_yaml_ng::from_str("proxies: [{name: fixture, type: ss, server: 192.0.2.1}]\nproxy-groups: []")?;
        aggregate(&mut config, &IProfiles::default(), &Settings::default()).await?;
        assert_eq!(config["proxy-groups"][0]["name"].as_str(), Some("ALL-SUBSCRIPTIONS"));
        assert_eq!(config["proxy-groups"][0]["proxies"][0].as_str(), Some("fixture"));
        Ok(())
    }

    #[test]
    fn aggregate_namespaces_provider_names_and_cache_paths() -> Result<()> {
        let source: Mapping = serde_yaml_ng::from_str(
            "proxy-providers: {airport: {type: http, url: 'https://example.test/nodes', path: ./cache.yaml}}",
        )?;
        let mut config = Mapping::new();
        merge_providers(&mut config, &source, "first")?;
        merge_providers(&mut config, &source, "second")?;
        let providers = &config["proxy-providers"];
        assert_ne!(
            providers["first::airport"]["path"],
            providers["second::airport"]["path"]
        );
        Ok(())
    }

    #[test]
    fn aggregate_renames_nodes_that_collide_with_group_names() -> Result<()> {
        let mut nodes = Vec::new();
        let source = serde_yaml_ng::from_str::<Vec<Value>>("[{name: ALL-SUBSCRIPTIONS, type: ss, server: 192.0.2.1}]")?;
        let mut names = HashSet::from(["ALL-SUBSCRIPTIONS".into()]);
        append(&mut nodes, &mut names, &mut HashSet::new(), &source)?;
        assert_ne!(nodes[0]["name"].as_str(), Some("ALL-SUBSCRIPTIONS"));
        Ok(())
    }

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

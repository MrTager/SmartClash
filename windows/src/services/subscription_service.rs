use crate::logic::model::{Source, PROXY_PORT};
use crate::logic::subscription;
use crate::logic::model::ProxyNode;
use crate::services::{command, paths};
use std::path::Path;

pub struct SubscriptionUpdate {
    pub nodes: Vec<ProxyNode>,
    pub warnings: Vec<String>,
}

fn cache_path(root: &Path, source: &Source) -> std::path::PathBuf {
    root.join(format!("subscription-{}.yaml", source.id))
}

fn valid_remote(source: &Source) -> bool {
    let url = source.url.trim();
    if !url.starts_with("https://") {
        return false;
    }
    let authority = url
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("");
    !authority.is_empty() && !authority.contains('@')
}

fn push_skipped(source: &Source, skipped: usize, warnings: &mut Vec<String>) {
    if skipped > 0 {
        warnings.push(format!(
            "{}：已跳过 {skipped} 个不支持或格式异常的节点",
            source.name
        ));
    }
}

pub fn update(sources: &[Source], through_proxy: bool) -> Result<SubscriptionUpdate, String> {
    let root = paths::ensure_root()?;
    let mut batches: Vec<Vec<ProxyNode>> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for source in sources {
        let cache = cache_path(&root, source);
        if source.is_local() {
            match std::fs::read(&cache)
                .ok()
                .and_then(|data| String::from_utf8(data).ok())
                .map(|text| subscription::parse_detailed(&text, source))
            {
                Some(Ok(outcome)) => {
                    push_skipped(source, outcome.skipped, &mut warnings);
                    batches.push(outcome.nodes);
                }
                Some(Err(error)) => warnings.push(format!("{}：{}", source.name, error)),
                None => warnings.push(format!("{}：本地快照缺失", source.name)),
            }
            continue;
        }

        if !valid_remote(source) {
            warnings.push(format!("{}：订阅地址须为 HTTPS", source.name));
            continue;
        }

        let temp = root.join(format!("download-{}.tmp", crate::logic::model::random_hex(8)));
        let mut args: Vec<String> = vec![
            "--fail".into(),
            "--silent".into(),
            "--show-error".into(),
            "--location".into(),
            "--max-redirs".into(),
            "3".into(),
            "--proto".into(),
            "=https".into(),
            "--proto-redir".into(),
            "=https".into(),
            "--connect-timeout".into(),
            "8".into(),
            "--max-time".into(),
            "30".into(),
            "--max-filesize".into(),
            "10000000".into(),
            "--user-agent".into(),
            "clash.meta/SmartClash".into(),
            "--output".into(),
            temp.display().to_string(),
            "--noproxy".into(),
            String::new(),
            "--proxy".into(),
            if through_proxy {
                format!("http://127.0.0.1:{PROXY_PORT}")
            } else {
                String::new()
            },
            "--url".into(),
            source.url.trim().into(),
        ];
        args.shrink_to_fit();

        let result = command::run(&paths::curl_binary(), &args);
        let downloaded = match result {
            Ok(output) if output.code == 0 => std::fs::read(&temp).ok(),
            _ => None,
        };
        let _ = std::fs::remove_file(&temp);

        match downloaded {
            Some(data) => {
                let text = String::from_utf8_lossy(&data).to_string();
                match subscription::parse_detailed(&text, source) {
                    Ok(outcome) => {
                        let _ = paths::write_private(&cache, &data);
                        push_skipped(source, outcome.skipped, &mut warnings);
                        batches.push(outcome.nodes);
                    }
                    Err(error) => {
                        warnings.push(format!("{}：{}", source.name, error));
                    }
                }
            }
            None => {
                // 下载失败保留该来源缓存。
                match std::fs::read(&cache)
                    .ok()
                    .and_then(|data| String::from_utf8(data).ok())
                    .map(|text| subscription::parse_detailed(&text, source))
                {
                    Some(Ok(outcome)) => {
                        push_skipped(source, outcome.skipped, &mut warnings);
                        batches.push(outcome.nodes);
                        warnings.push(format!("{}：更新失败，保留缓存", source.name));
                    }
                    _ => warnings.push(format!("{}：下载失败（网络或 HTTP 错误）", source.name)),
                }
            }
        }
    }

    let nodes = subscription::merge(batches);
    if nodes.is_empty() {
        return Err(if warnings.is_empty() {
            "请先添加订阅".into()
        } else {
            warnings.join("\n")
        });
    }
    Ok(SubscriptionUpdate { nodes, warnings })
}

pub fn cached(sources: &[Source]) -> Vec<ProxyNode> {
    let root = paths::data_root();
    let batches = sources
        .iter()
        .filter_map(|source| {
            let data = std::fs::read(cache_path(&root, source)).ok()?;
            let text = String::from_utf8(data).ok()?;
            subscription::parse(&text, source).ok()
        })
        .collect();
    subscription::merge(batches)
}

use crate::config::{Config, smartclash};
use anyhow::{Context as _, Result, ensure};
use serde_yaml_ng::{Mapping, Value};
use std::io::Write as _;

static TOKEN: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

pub async fn token() -> Result<&'static str> {
    TOKEN
        .get_or_try_init(|| async {
            let root = crate::utils::dirs::app_home_dir()?;
            tokio::fs::create_dir_all(&root).await?;
            let path = root.join("smartclash-subscription-token");
            if let Ok(token) = tokio::fs::read_to_string(&path).await {
                ensure!(
                    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "本机订阅令牌无效"
                );
                return Ok(token);
            }
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes)?;
            let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            #[cfg(windows)]
            let mut file = crate::core::owner_identity::create_private_current_user_file(&path)?;
            #[cfg(unix)]
            let mut file = {
                use std::os::unix::fs::OpenOptionsExt as _;
                std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&path)?
            };
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            Ok::<_, anyhow::Error>(token)
        })
        .await
        .map(String::as_str)
}

pub async fn link() -> Result<String> {
    Ok(format!(
        "http://127.0.0.1:{}/smartclash/subscription/{}",
        crate::utils::server::embedded_server_port()?,
        token().await?
    ))
}

pub async fn export() -> Result<String> {
    let settings = smartclash::load().await?;
    ensure!(settings.enabled, "请先启用 SmartClash 并应用订阅");
    let runtime = Config::runtime().await.data_arc();
    let source = runtime.config.as_ref().context("尚无已应用的订阅")?;
    let nodes = source
        .get("proxies")
        .and_then(Value::as_sequence)
        .context("订阅没有内联节点")?;
    let group = source
        .get("proxy-groups")
        .and_then(Value::as_sequence)
        .and_then(|groups| {
            groups
                .iter()
                .find(|group| group.get("name").and_then(Value::as_str) == Some("SMARTCLASH"))
        })
        .context("SmartClash 策略组尚未应用")?;
    ensure!(group.get("use").is_none(), "本机聚合导出暂不支持仅 provider 来源");
    let mut config = Mapping::new();
    config.insert("proxies".into(), nodes.clone().into());
    config.insert("proxy-groups".into(), vec![group.clone()].into());
    config.insert("rules".into(), vec![Value::from("MATCH,SMARTCLASH")].into());
    if settings.smart_rules {
        crate::enhance::smartclash::rules(&mut config)?;
    }
    Ok(serde_yaml_ng::to_string(&config)?)
}

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    LowLatency,
    HighSpeed,
    Ai,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub mode: Mode,
    pub automatic: bool,
    pub prefer_non_hong_kong: bool,
    pub chain_enabled: bool,
    pub entry_nodes: Vec<String>,
    pub exit_nodes: Vec<String>,
    pub globe_animation: bool,
    #[serde(default)]
    pub source_profiles: Vec<String>,
    #[serde(default)]
    pub smart_rules: bool,
    #[serde(default)]
    pub explore_multihop: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::LowLatency,
            automatic: true,
            prefer_non_hong_kong: true,
            chain_enabled: false,
            entry_nodes: Vec::new(),
            exit_nodes: Vec::new(),
            globe_animation: true,
            source_profiles: Vec::new(),
            smart_rules: false,
            explore_multihop: false,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.source_profiles.len() <= 64, "最多聚合 64 个订阅");
        let mut sources = std::collections::HashSet::new();
        for uid in &self.source_profiles {
            ensure!(
                !uid.is_empty() && uid.len() <= 128 && sources.insert(uid),
                "订阅标识为空、过长或重复"
            );
        }
        for names in [&self.entry_nodes, &self.exit_nodes] {
            ensure!(names.len() <= 128, "入口或出口最多允许 128 个节点");
            let mut seen = std::collections::HashSet::new();
            for name in names {
                ensure!(!name.trim().is_empty() && name.len() <= 1024, "节点名称为空或过长");
                ensure!(seen.insert(name), "节点名称重复：{name}");
            }
        }
        if self.enabled && self.chain_enabled {
            ensure!(
                !self.entry_nodes.is_empty() && !self.exit_nodes.is_empty(),
                "显式链路需要入口和出口"
            );
            ensure!(
                self.entry_nodes.len() * self.exit_nodes.len() <= 128,
                "入口 × 出口最多允许 128 个组合"
            );
        }
        Ok(())
    }
}

fn path() -> Result<PathBuf> {
    Ok(crate::utils::dirs::app_home_dir()?.join("smartclash.json"))
}

pub async fn load() -> Result<Settings> {
    match tokio::fs::read(path()?).await {
        Ok(data) => {
            let settings: Settings = serde_json::from_slice(&data).context("无法解析 smartclash.json")?;
            settings.validate()?;
            Ok(settings)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error).context("无法读取 SmartClash 独立设置"),
    }
}

pub async fn save(settings: &Settings) -> Result<()> {
    settings.validate()?;
    let path = path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    crate::utils::help::save_yaml_str(&path, &serde_json::to_string_pretty(settings)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_use_the_exact_camel_case_contract() -> Result<()> {
        let value = serde_json::to_value(Settings::default())?;
        assert_eq!(
            value,
            serde_json::json!({
                "enabled": false, "mode": "lowLatency", "automatic": true,
                "preferNonHongKong": true, "chainEnabled": false, "entryNodes": [],
                "exitNodes": [], "globeAnimation": true, "sourceProfiles": [], "smartRules": false, "exploreMultihop": false
            })
        );
        assert!(serde_json::from_value::<Settings>(serde_json::json!({"enabled": true})).is_err());
        Ok(())
    }
}

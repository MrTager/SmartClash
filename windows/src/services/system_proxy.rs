use crate::logic::model::proxy_port;
use crate::services::paths;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const INTERNET_SETTINGS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Snapshot {
    proxy_enable: u32,
    proxy_server: Option<String>,
    proxy_override: Option<String>,
    autoconfig_url: Option<String>,
    auto_detect: u32,
}

#[link(name = "wininet")]
extern "system" {
    fn InternetSetOptionW(
        internet: *mut c_void,
        option: u32,
        buffer: *mut c_void,
        length: u32,
    ) -> i32;
}

/// 通知 WinINET 立即重新读取代理设置。
fn notify_change() {
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
    const INTERNET_OPTION_REFRESH: u32 = 37;
    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
}

pub struct SystemProxy;

impl SystemProxy {
    fn backup_path() -> std::path::PathBuf {
        paths::data_root().join("proxy-backup.json")
    }

    pub fn has_backup() -> bool {
        Self::backup_path().exists()
    }

    fn read_snapshot() -> Result<Snapshot, String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法读取系统代理设置：{e}"))?;
        Ok(Snapshot {
            proxy_enable: key.get_value::<u32, _>("ProxyEnable").unwrap_or(0),
            proxy_server: key.get_value::<String, _>("ProxyServer").ok(),
            proxy_override: key.get_value::<String, _>("ProxyOverride").ok(),
            autoconfig_url: key.get_value::<String, _>("AutoConfigURL").ok(),
            auto_detect: key.get_value::<u32, _>("AutoDetect").unwrap_or(0),
        })
    }

    pub fn enable() -> Result<(), String> {
        if Self::has_backup() {
            return Err("已有代理恢复记录，请先恢复旧设置".into());
        }
        let snapshot = Self::read_snapshot()?;
        if snapshot
            .autoconfig_url
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            || snapshot.auto_detect == 1
        {
            return Err("系统已启用 PAC / 自动代理发现，请在 Windows 设置中关闭后再试，避免代理策略冲突".into());
        }

        // 先落盘备份，再修改设置：即使中途崩溃也能恢复。
        let encoded = serde_json::to_vec_pretty(&snapshot).map_err(|e| e.to_string())?;
        paths::write_private(&Self::backup_path(), &encoded)?;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey_with_flags(INTERNET_SETTINGS, KEY_WRITE)
            .map_err(|e| format!("无法写入系统代理设置：{e}"))?;
        let proxy = format!("127.0.0.1:{}", proxy_port());
        let result = (|| -> Result<(), String> {
            key.set_value("ProxyEnable", &1u32)
                .map_err(|e| format!("无法启用系统代理：{e}"))?;
            key.set_value("ProxyServer", &proxy)
                .map_err(|e| format!("无法写入代理地址：{e}"))?;
            if snapshot.proxy_override.is_none() {
                let _ = key.set_value("ProxyOverride", &"<local>".to_string());
            }
            Ok(())
        })();
        notify_change();
        if let Err(error) = result {
            let _ = Self::restore();
            return Err(error);
        }
        Ok(())
    }

    pub fn restore() -> Result<(), String> {
        if !Self::has_backup() {
            return Ok(());
        }
        let data = std::fs::read(Self::backup_path()).map_err(|e| format!("无法读取代理备份：{e}"))?;
        let snapshot: Snapshot =
            serde_json::from_slice(&data).map_err(|e| format!("代理备份格式无效：{e}"))?;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法恢复系统代理设置：{e}"))?;

        key.set_value("ProxyEnable", &snapshot.proxy_enable)
            .map_err(|e| format!("无法恢复代理开关：{e}"))?;
        let mut errors: Vec<String> = Vec::new();
        match snapshot.proxy_server.as_deref() {
            Some(value) => {
                if let Err(e) = key.set_value("ProxyServer", &value.to_string()) {
                    errors.push(format!("ProxyServer：{e}"));
                }
            }
            None => {
                if let Err(e) = key.delete_value("ProxyServer") {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        errors.push(format!("ProxyServer：{e}"));
                    }
                }
            }
        }
        match snapshot.proxy_override.as_deref() {
            Some(value) => {
                if let Err(e) = key.set_value("ProxyOverride", &value.to_string()) {
                    errors.push(format!("ProxyOverride：{e}"));
                }
            }
            None => {
                if let Err(e) = key.delete_value("ProxyOverride") {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        errors.push(format!("ProxyOverride：{e}"));
                    }
                }
            }
        }
        match snapshot.autoconfig_url.as_deref() {
            Some(value) => {
                if let Err(e) = key.set_value("AutoConfigURL", &value.to_string()) {
                    errors.push(format!("AutoConfigURL：{e}"));
                }
            }
            None => {
                if let Err(e) = key.delete_value("AutoConfigURL") {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        errors.push(format!("AutoConfigURL：{e}"));
                    }
                }
            }
        }
        if let Err(e) = key.set_value("AutoDetect", &snapshot.auto_detect) {
            errors.push(format!("AutoDetect：{e}"));
        }
        notify_change();
        if !errors.is_empty() {
            // 保留备份，下一次启动仍可继续恢复，避免丢失唯一的回滚记录。
            return Err(format!("系统代理仅部分恢复：{}", errors.join("；")));
        }
        std::fs::remove_file(Self::backup_path()).map_err(|e| format!("无法删除代理备份：{e}"))?;
        Ok(())
    }
}

use crate::logic::model::proxy_port;
use crate::services::{lifecycle, native, paths};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::c_void, path::Path};
use winreg::{
    enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
    RegKey, RegValue,
};
const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Value {
    kind: u32,
    bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Field {
    original: Option<Value>,
    applied: Option<Value>,
    conflict: bool,
    #[serde(default)]
    current: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Transaction {
    version: u32,
    session: String,
    phase: String,
    fields: BTreeMap<String, Field>,
}
trait Registry {
    fn get(&self, name: &str) -> Result<Option<Value>, String>;
    fn set(&mut self, name: &str, value: &Option<Value>) -> Result<(), String>;
    fn flush_notify(&mut self) -> Result<(), String>;
}
struct UserRegistry(RegKey);
impl UserRegistry {
    fn open() -> Result<Self, String> {
        #[cfg(test)]
        panic!("测试禁止访问真实用户代理注册表");
        #[allow(unreachable_code)]
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ | KEY_WRITE)
            .map(Self)
            .map_err(|e| format!("无法打开同用户系统代理：{e}"))
    }
}
impl Registry for UserRegistry {
    fn get(&self, name: &str) -> Result<Option<Value>, String> {
        match self.0.get_raw_value(name) {
            Ok(v) => Ok(Some(Value {
                kind: v.vtype as u32,
                bytes: v.bytes,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("无法读取 {name}：{e}")),
        }
    }
    fn set(&mut self, name: &str, value: &Option<Value>) -> Result<(), String> {
        match value {
            Some(v) => {
                let kind = match v.kind {
                    1 => RegType::REG_SZ,
                    2 => RegType::REG_EXPAND_SZ,
                    3 => RegType::REG_BINARY,
                    4 => RegType::REG_DWORD,
                    7 => RegType::REG_MULTI_SZ,
                    11 => RegType::REG_QWORD,
                    _ => return Err("备份包含不支持的注册表类型".into()),
                };
                self.0
                    .set_raw_value(
                        name,
                        &RegValue {
                            vtype: kind,
                            bytes: v.bytes.clone(),
                        },
                    )
                    .map_err(|e| e.to_string())
            }
            None => match self.0.delete_value(name) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            },
        }
    }
    fn flush_notify(&mut self) -> Result<(), String> {
        let status =
            unsafe { windows_sys::Win32::System::Registry::RegFlushKey(self.0.raw_handle() as _) };
        if status != 0 {
            return Err(format!("代理注册表 flush 失败：{status}"));
        }
        notify_change()
    }
}
fn dword(value: u32) -> Option<Value> {
    Some(Value {
        kind: 4,
        bytes: value.to_le_bytes().to_vec(),
    })
}
fn string(value: &str) -> Option<Value> {
    Some(Value {
        kind: 1,
        bytes: value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect(),
    })
}

fn notify_change() -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::FreeLibrary,
        System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
    };
    // 不依赖 MinGW 的占位 wininet 导入库；失败必须保留恢复事务。
    unsafe {
        let library = LoadLibraryExW(
            native::wide("wininet.dll").as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if library.is_null() {
            return Err(native::error("无法加载系统 WinINET"));
        }
        let function = GetProcAddress(library, b"InternetSetOptionW\0".as_ptr());
        let result = if let Some(function) = function {
            let call: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32 =
                std::mem::transmute(function);
            let changed = call(std::ptr::null_mut(), 39, std::ptr::null_mut(), 0);
            let refreshed = call(std::ptr::null_mut(), 37, std::ptr::null_mut(), 0);
            if changed == 0 || refreshed == 0 {
                Err(native::error("WinINET 代理通知失败，保留恢复记录"))
            } else {
                Ok(())
            }
        } else {
            Err(native::error("找不到 WinINET 通知入口"))
        };
        FreeLibrary(library);
        result
    }
}
fn save(path: &Path, tx: &Transaction) -> Result<(), String> {
    native::atomic_write(
        path,
        &serde_json::to_vec_pretty(tx).map_err(|e| e.to_string())?,
    )
}
fn prepare(registry: &impl Registry, session: String, port: u16) -> Result<Transaction, String> {
    let pac = registry.get("AutoConfigURL")?;
    if pac
        .as_ref()
        .map(|v| v.bytes.iter().any(|b| *b != 0))
        .unwrap_or(false)
        || registry.get("AutoDetect")? == dword(1)
    {
        return Err("系统已启用 PAC / 自动代理发现，请先关闭以避免策略冲突".into());
    }
    let mut fields = BTreeMap::new();
    fields.insert(
        "ProxyServer".into(),
        Field {
            original: registry.get("ProxyServer")?,
            applied: string(&format!("127.0.0.1:{port}")),
            conflict: false,
            current: None,
        },
    );
    fields.insert(
        "ProxyEnable".into(),
        Field {
            original: registry.get("ProxyEnable")?,
            applied: dword(1),
            conflict: false,
            current: None,
        },
    );
    if registry.get("ProxyOverride")?.is_none() {
        fields.insert(
            "ProxyOverride".into(),
            Field {
                original: None,
                applied: string("<local>"),
                conflict: false,
                current: None,
            },
        );
    }
    Ok(Transaction {
        version: 1,
        session,
        phase: "prepared".into(),
        fields,
    })
}
fn apply(registry: &mut impl Registry, path: &Path, tx: &mut Transaction) -> Result<(), String> {
    // 写入意图和原值必须先 flush + 原子替换；任意字段中途崩溃仍能逐字段回滚。
    save(path, tx)?;
    for name in ["ProxyServer", "ProxyOverride", "ProxyEnable"] {
        if let Some(field) = tx.fields.get(name) {
            if registry.get(name)? != field.original {
                return Err(format!("{name} 在准备阶段已被外部修改，保留事务"));
            }
            registry.set(name, &field.applied)?;
        }
    }
    registry.flush_notify()?;
    tx.phase = "applied".into();
    save(path, tx)
}
fn recover(registry: &mut impl Registry, path: &Path, tx: &mut Transaction) -> Result<(), String> {
    if tx.version != 1
        || !["prepared", "applied", "restoring", "conflict", "restored"]
            .contains(&tx.phase.as_str())
        || !native::valid_nonce(&tx.session)
        || tx
            .fields
            .keys()
            .any(|k| !["ProxyEnable", "ProxyServer", "ProxyOverride"].contains(&k.as_str()))
    {
        return Err("代理事务身份或版本不合法，保留备份".into());
    }
    tx.phase = "restoring".into();
    save(path, tx)?;
    let mut conflicts = Vec::new();
    // 注册表没有可与其他软件共享的 CAS；锁内逐字段读取后立即写入，不做整快照覆盖。
    for (name, field) in &mut tx.fields {
        let current = registry.get(name)?;
        if field.conflict {
            conflicts.push(name.clone());
            continue;
        }
        if current == field.original {
            continue;
        }
        if current != field.applied {
            field.conflict = true;
            field.current = current;
            conflicts.push(name.clone());
            continue;
        }
        registry.set(name, &field.original)?;
    }
    // 通知失败也不能丢掉首次观察到的外部值。
    save(path, tx)?;
    registry.flush_notify()?;
    if !conflicts.is_empty() {
        tx.phase = "conflict".into();
        save(path, tx)?;
        let archive = path.with_file_name(format!(
            "proxy-conflict-{}-{}.json",
            tx.session,
            &native::nonce()?[..16]
        ));
        save(&archive, tx)?;
        // 证据持久化后解除 active 责任，不再因外部所有权阻塞守护和新会话。
        return std::fs::remove_file(path).map_err(|e| e.to_string());
    }
    tx.phase = "restored".into();
    save(path, tx)?;
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySnapshot {
    proxy_enable: u32,
    proxy_server: Option<String>,
    proxy_override: Option<String>,
    autoconfig_url: Option<String>,
    auto_detect: u32,
}
fn recover_legacy(registry: &mut impl Registry, path: &Path, data: &[u8]) -> Result<(), String> {
    let snapshot: LegacySnapshot =
        serde_json::from_slice(data).map_err(|_| "代理备份无法解析，已保留原件供人工恢复")?;
    let fields = [
        ("ProxyEnable", dword(snapshot.proxy_enable)),
        (
            "ProxyServer",
            snapshot.proxy_server.as_deref().and_then(string),
        ),
        (
            "ProxyOverride",
            snapshot.proxy_override.as_deref().and_then(string),
        ),
        (
            "AutoConfigURL",
            snapshot.autoconfig_url.as_deref().and_then(string),
        ),
        ("AutoDetect", dword(snapshot.auto_detect)),
    ];
    for (name, original) in fields {
        if registry.get(name)? != original {
            return Err("发现旧版代理备份：缺少 session 和实际写入值，不能安全覆盖当前设置；原件已保留，请人工恢复".into());
        }
    }
    registry.flush_notify()?;
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

pub struct SystemProxy;
impl SystemProxy {
    fn backup_path() -> std::path::PathBuf {
        paths::data_root().join("proxy-backup.json")
    }
    pub fn has_backup() -> bool {
        Self::backup_path().exists()
    }
    pub fn enable() -> Result<(), String> {
        let session = lifecycle::ensure_ready()?;
        let _lock = native::lock(&native::domain("proxy")?, 30_000)?;
        if Self::has_backup() {
            return Err("已有代理恢复记录，请先恢复旧设置".into());
        }
        if lifecycle::ensure_ready()? != session {
            return Err("代理会话已失效".into());
        }
        let mut registry = UserRegistry::open()?;
        let mut tx = prepare(&registry, session, proxy_port())?;
        apply(&mut registry, &Self::backup_path(), &mut tx)
    }
    pub fn restore() -> Result<(), String> {
        let _lock = native::lock(&native::domain("proxy")?, 30_000)?;
        let path = Self::backup_path();
        if !path.exists() {
            return Ok(());
        }
        let data = std::fs::read(&path).map_err(|e| e.to_string())?;
        let mut registry = UserRegistry::open()?;
        match serde_json::from_slice::<Transaction>(&data) {
            Ok(mut tx) => recover(&mut registry, &path, &mut tx),
            Err(_) => recover_legacy(&mut registry, &path, &data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Mock {
        values: BTreeMap<String, Value>,
        notify_fail: bool,
    }
    impl Registry for Mock {
        fn get(&self, name: &str) -> Result<Option<Value>, String> {
            Ok(self.values.get(name).cloned())
        }
        fn set(&mut self, name: &str, value: &Option<Value>) -> Result<(), String> {
            if let Some(v) = value {
                self.values.insert(name.into(), v.clone());
            } else {
                self.values.remove(name);
            }
            Ok(())
        }
        fn flush_notify(&mut self) -> Result<(), String> {
            if self.notify_fail {
                Err("模拟通知失败".into())
            } else {
                Ok(())
            }
        }
    }
    fn fixture() -> (std::path::PathBuf, Mock, Transaction) {
        let dir = std::env::temp_dir().join(format!(
            "smartclash-proxy-test-{}",
            &native::nonce().unwrap()[..12]
        ));
        std::fs::create_dir(&dir).unwrap();
        let registry = Mock::default();
        let tx = prepare(&registry, native::nonce().unwrap(), 17890).unwrap();
        (dir.join("backup.json"), registry, tx)
    }
    #[test]
    fn transaction_partial_crash_and_recovery_idempotence() {
        let (path, mut registry, mut tx) = fixture();
        save(&path, &tx).unwrap();
        registry
            .set("ProxyServer", &tx.fields["ProxyServer"].applied)
            .unwrap();
        recover(&mut registry, &path, &mut tx).unwrap();
        assert!(registry.values.is_empty());
        assert!(!path.exists());
        save(&path, &tx).unwrap();
        recover(&mut registry, &path, &mut tx).unwrap();
        assert!(registry.values.is_empty());
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn external_proxy_conflict_is_never_overwritten() {
        let (path, mut registry, mut tx) = fixture();
        apply(&mut registry, &path, &mut tx).unwrap();
        registry
            .set("ProxyServer", &string("other-app:8899"))
            .unwrap();
        recover(&mut registry, &path, &mut tx).unwrap();
        assert_eq!(
            registry.get("ProxyServer").unwrap(),
            string("other-app:8899")
        );
        assert!(!path.exists());
        let archive = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("proxy-conflict-")
            })
            .unwrap();
        let evidence: Transaction =
            serde_json::from_slice(&std::fs::read(archive).unwrap()).unwrap();
        let field = &evidence.fields["ProxyServer"];
        assert!(field.conflict);
        assert_eq!(field.original, None);
        assert_eq!(field.applied, string("127.0.0.1:17890"));
        assert_eq!(field.current, string("other-app:8899"));
        let mut next = prepare(&registry, native::nonce().unwrap(), 17891).unwrap();
        apply(&mut registry, &path, &mut next).unwrap();
        recover(&mut registry, &path, &mut next).unwrap();
        assert_eq!(
            registry.get("ProxyServer").unwrap(),
            string("other-app:8899")
        );
        assert!(!path.exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn legacy_backup_never_guesses_applied_values() {
        let (path, mut registry, _) = fixture();
        let data = br#"{"proxy_enable":0,"proxy_server":null,"proxy_override":null,"autoconfig_url":null,"auto_detect":0}"#;
        native::atomic_write(&path, data).unwrap();
        assert!(recover_legacy(&mut registry, &path, data).is_err());
        assert!(path.exists());
        registry.set("ProxyEnable", &dword(0)).unwrap();
        registry.set("AutoDetect", &dword(0)).unwrap();
        recover_legacy(&mut registry, &path, data).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn notification_failure_keeps_unique_recovery_record() {
        let (path, mut registry, mut tx) = fixture();
        registry.notify_fail = true;
        assert!(apply(&mut registry, &path, &mut tx).is_err());
        assert!(path.exists());
        assert!(recover(&mut registry, &path, &mut tx).is_err());
        assert!(path.exists());
        registry.notify_fail = false;
        recover(&mut registry, &path, &mut tx).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }
}

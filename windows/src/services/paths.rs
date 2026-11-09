use std::path::{Path, PathBuf};

/// 工作数据目录：%APPDATA%\SmartClash（对应 macOS 版的 Application Support）。
pub fn data_root() -> PathBuf {
    #[cfg(test)]
    if let Some(root) = std::env::var_os("SMARTCLASH_FIXTURE_ROOT") {
        return PathBuf::from(root);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("SmartClash")
}

pub fn ensure_root() -> Result<PathBuf, String> {
    let root = data_root();
    std::fs::create_dir_all(&root).map_err(|e| format!("无法创建工作目录：{e}"))?;
    Ok(root)
}

pub fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    }
    crate::services::native::atomic_write(path, data)
}

pub fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 打包后资源位于 exe 同级的 resources\；开发时回退到 windows\resources\。
pub fn resource(name: &str) -> PathBuf {
    let beside = app_dir().join("resources").join(name);
    if beside.exists() {
        return beside;
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(name);
    if dev.exists() {
        return dev;
    }
    beside
}

pub fn core_binary(kind: crate::logic::kernel::KernelKind) -> PathBuf {
    resource(kind.binary_name())
}

/// Xray 的 geoip.dat / geosite.dat 所在目录（打包后与内核同目录）。
pub fn asset_dir() -> PathBuf {
    let beside = app_dir().join("resources");
    if beside.exists() {
        return beside;
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    if dev.exists() {
        return dev;
    }
    beside
}

pub fn curl_binary() -> PathBuf {
    let bundled = resource("curl.exe");
    if bundled.exists() {
        return bundled;
    }
    let system = PathBuf::from(r"C:\Windows\System32\curl.exe");
    if system.exists() {
        return system;
    }
    PathBuf::from("curl.exe")
}

pub fn open_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

#[repr(C)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

extern "system" {
    fn GetLocalTime(system_time: *mut SystemTime);
}

fn local_now() -> SystemTime {
    let mut value = SystemTime {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe { GetLocalTime(&mut value) };
    value
}

pub fn time_text() -> String {
    let now = local_now();
    format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second)
}

pub fn datetime_text(epoch_ms: u64) -> String {
    if epoch_ms == 0 {
        return "未测速".into();
    }
    // 延迟结果只用于展示，直接显示本地当前时间与记录时间差即可。
    let now = crate::logic::health::now_ms();
    let delta = now.saturating_sub(epoch_ms) / 1000;
    if delta < 60 {
        format!("{delta} 秒前")
    } else if delta < 3600 {
        format!("{} 分钟前", delta / 60)
    } else {
        format!("{} 小时前", delta / 3600)
    }
}

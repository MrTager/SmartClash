use crate::logic::kernel::{KernelKind, XRAY_API_PORT, XRAY_SOCKS_PORT};
use crate::services::{
    core_client::CoreClient,
    lifecycle::{self, Broker},
    native, paths,
};
use std::{net::TcpListener, path::PathBuf, time::Duration};

pub struct CoreProcess {
    broker: Option<Broker>,
    kind: KernelKind,
}
impl CoreProcess {
    pub fn new(kind: KernelKind) -> Self {
        Self { broker: None, kind }
    }
    pub fn set_kind(&mut self, kind: KernelKind) {
        self.kind = kind;
    }
    pub fn kind(&self) -> KernelKind {
        self.kind
    }
    pub fn executable(&self) -> PathBuf {
        paths::core_binary(self.kind)
    }
    pub fn recovery_needed(&self) -> bool {
        paths::data_root().join("core.pid").exists()
            || paths::data_root().join("tun.pid").exists()
            || self
                .broker
                .as_ref()
                .map(|broker| broker.recovery_needed())
                .unwrap_or(false)
    }
    pub fn start(&mut self, data: &[u8], tun: bool, client: &CoreClient) -> Result<(), String> {
        lifecycle::ensure_ready()?;
        if self.broker.is_some()
            || paths::data_root().join("core.pid").exists()
            || paths::data_root().join("tun.pid").exists()
        {
            return Err("请先停止内核 / 恢复遗留服务".into());
        }
        if tun && !self.kind.capabilities().tun {
            return Err("当前内核不支持 TUN".into());
        }
        let ports: Vec<u16> = match self.kind {
            KernelKind::Mihomo | KernelKind::SingBox => vec![
                crate::logic::model::CONTROLLER_PORT,
                crate::logic::model::proxy_port(),
                crate::logic::model::PROBE_PORT,
            ],
            KernelKind::Xray => vec![
                crate::logic::model::proxy_port(),
                XRAY_SOCKS_PORT,
                XRAY_API_PORT,
            ],
        };
        for port in ports {
            if TcpListener::bind(("127.0.0.1", port)).is_err() {
                return Err(format!("端口 {port} 已占用，请关闭冲突的代理程序"));
            }
        }
        if tun && !paths::resource("wintun.dll").exists() {
            return Err("缺少 wintun.dll，拒绝启动 TUN".into());
        }
        validate(self.kind, data)?;
        self.broker = Some(Broker::start(self.kind, data, tun, false)?);
        let result = (|| {
            for _ in 0..60 {
                lifecycle::ensure_ready()?;
                if !self.broker.as_ref().unwrap().alive()? {
                    return Err("broker 或内核提前退出，请恢复网络并检查会话日志".into());
                }
                if client.version().is_ok() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err("内核启动超时，请检查会话 core.log".into())
        })();
        if result.is_err() {
            let _ = self.stop();
        }
        result
    }
    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(broker) = self.broker.as_ref() {
            broker.stop()?;
            self.broker = None;
        }
        recover_legacy()
    }
}
impl Default for CoreProcess {
    fn default() -> Self {
        Self::new(KernelKind::Mihomo)
    }
}

pub fn validate(kind: KernelKind, data: &[u8]) -> Result<(), String> {
    lifecycle::ensure_ready()?;
    let root = paths::ensure_root()?;
    let dir = root.join(format!("config-check-{}", native::nonce()?));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    paths::write_private(&dir.join("config.json"), data)?;
    let mut command = std::process::Command::new(paths::core_binary(kind));
    command
        .args(lifecycle::core_args(kind, &dir, true))
        .env("SAFE_PATHS", paths::data_root())
        .current_dir(&dir)
        .stdin(std::process::Stdio::null());
    if kind == KernelKind::Xray {
        command.env("XRAY_LOCATION_ASSET", paths::asset_dir());
    }
    let result = lifecycle::output(&mut command);
    let _ = std::fs::remove_dir_all(&dir);
    let output = result?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    paths::write_private(&root.join("core-test.log"), text.as_bytes())?;
    if output.status.success() {
        Ok(())
    } else {
        Err("内核配置校验失败，请检查 core-test.log".into())
    }
}
pub fn process_alive(pid: u32) -> bool {
    native::inspect(pid).map(|v| v.is_some()).unwrap_or(true)
}
pub fn recover_legacy() -> Result<(), String> {
    for name in ["core.pid", "tun.pid"] {
        let path = paths::data_root().join(name);
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let pid = text
            .trim()
            .parse::<u32>()
            .map_err(|_| "旧版 PID 记录无法解析，已保留，请人工恢复")?;
        match native::inspect(pid)? {
            None => std::fs::remove_file(&path).map_err(|e| e.to_string())?,
            Some(identity) => {
                let bundled = KernelKind::ALL
                    .iter()
                    .any(|kind| native::same_path(&identity.exe, &paths::core_binary(*kind)));
                return Err(if bundled {
                    format!("旧版内核 {pid} 路径匹配，但缺少创建时间及会话归属；请人工确认并结束，保留 {name}")
                } else {
                    format!(
                        "旧版 PID {pid} 已被其他程序复用；未终止或删除唯一记录 {name}，请人工确认"
                    )
                });
            }
        }
    }
    Ok(())
}

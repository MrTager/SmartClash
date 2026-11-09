//! 临时单节点内核探测。
//!
//! Xray 没有 Clash 控制接口，无法像 mihomo / sing-box 那样用 PROBE 组测速；
//! 这里按需启动一个只监听本机随机端口的临时内核，探测完立即结束。

use crate::logic::health::now_ms;
use crate::logic::kernel::KernelKind;
use crate::logic::kernel_config;
use crate::logic::model::{NodeLatency, ProxyNode};
use crate::services::core_client::CoreClient;
use crate::services::paths;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct EphemeralProbe {
    child: Child,
    dir: PathBuf,
    pub port: u16,
}

impl EphemeralProbe {
    pub fn start(kind: KernelKind, node: &ProxyNode) -> Result<Self, String> {
        let exe = paths::core_binary(kind);
        if !exe.exists() {
            return Err(format!("缺少 {} 内核", kind.title()));
        }
        let port = free_port()?;
        let sequence = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = paths::data_root().join(format!("probe-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建探测目录：{e}"))?;
        let config_path = dir.join("probe.json");
        let config = kernel_config::probe_config(kind, node, port)?;
        paths::write_private(&config_path, &config)?;

        let log_path = dir.join("probe.log");
        let log = std::fs::File::create(&log_path).map_err(|e| format!("无法创建探测日志：{e}"))?;
        let err = log
            .try_clone()
            .map_err(|e| format!("无法创建探测日志：{e}"))?;
        let mut cmd = Command::new(&exe);
        cmd.creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err))
            .current_dir(&dir);
        match kind {
            KernelKind::Mihomo => {
                cmd.args([
                    "-d",
                    &dir.display().to_string(),
                    "-f",
                    &config_path.display().to_string(),
                ]);
            }
            KernelKind::SingBox => {
                cmd.args([
                    "run",
                    "-c",
                    &config_path.display().to_string(),
                    "-D",
                    &dir.display().to_string(),
                ]);
            }
            KernelKind::Xray => {
                cmd.args(["run", "-c", &config_path.display().to_string()]);
                cmd.env("XRAY_LOCATION_ASSET", paths::asset_dir());
            }
        }
        let child = crate::services::lifecycle::spawn_child(&mut cmd)
            .map_err(|e| format!("无法启动受控探测内核：{e}"))?;
        let mut probe = Self { child, dir, port };
        if let Err(error) = probe.wait_ready() {
            let detail = probe.log_tail();
            return Err(if detail.is_empty() {
                error
            } else {
                format!("{error}：{detail}")
            });
        }
        Ok(probe)
    }

    fn wait_ready(&mut self) -> Result<(), String> {
        let address = SocketAddr::from(([127, 0, 0, 1], self.port));
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!(
                    "探测内核提前退出（{}）",
                    status.code().unwrap_or(-1)
                ));
            }
            if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(120));
        }
        Err("探测内核启动超时".into())
    }

    fn log_tail(&self) -> String {
        let path = self.dir.join("probe.log");
        let Ok(text) = std::fs::read_to_string(path) else {
            return String::new();
        };
        let text = text.trim();
        // 按字符截断，避免把多字节中文切在中间导致 panic。
        let chars: Vec<char> = text.chars().collect();
        let start = chars.len().saturating_sub(240);
        chars[start..].iter().collect()
    }
}

impl Drop for EphemeralProbe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("无法分配探测端口：{e}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|e| format!("无法读取探测端口：{e}"))
}

/// 通过临时内核测 HTTPS 延迟。
pub fn latency(kind: KernelKind, node: &ProxyNode, timeout_ms: u64) -> NodeLatency {
    let start = Instant::now();
    // internet_probe_timeout 的参数单位是秒，这里把毫秒配置换算过去。
    let timeout_secs = (timeout_ms / 1000).max(1);
    let result = run(kind, node, |client, port| {
        client.internet_probe_timeout(port, timeout_secs)
    });
    match result {
        Ok(result) if result.health == crate::logic::model::Health::Healthy => NodeLatency {
            milliseconds: Some(result.milliseconds.max(0)),
            checked: now_ms(),
            failure: None,
        },
        Ok(result) => NodeLatency::new(None, Some(result.detail)),
        Err(error) => NodeLatency::new(
            None,
            Some(format!("{error}（{} ms）", start.elapsed().as_millis())),
        ),
    }
}

fn run<T>(
    kind: KernelKind,
    node: &ProxyNode,
    operation: impl FnOnce(&CoreClient, u16) -> T,
) -> Result<T, String> {
    let probe = EphemeralProbe::start(kind, node)?;
    let client = CoreClient::new(kind, "");
    Ok(operation(&client, probe.port))
}

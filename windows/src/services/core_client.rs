use crate::logic::health::{delay_path, now_ms, HealthPolicy, InternetPolicy};
use crate::logic::kernel::{KernelKind, XRAY_API_PORT};
use crate::logic::model::{
    ConnectionSnapshot, NodeLatency, ProbeResult, ThroughputSample, TrafficMode, CONTROLLER_PORT,
};
use crate::services::{command, paths};
use serde_json::Value;
use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static RELOAD_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct CoreClient {
    pub kind: KernelKind,
    pub secret: String,
}

impl CoreClient {
    pub fn new(kind: KernelKind, secret: &str) -> Self {
        Self {
            kind,
            secret: secret.to_string(),
        }
    }

    /// 返回 HTTP 状态码与响应体；curl 自身失败（连不上、超时）仍按错误处理。
    fn request(
        &self,
        path: &str,
        method: &str,
        body: Option<String>,
        timeout_secs: u64,
    ) -> Result<(u16, Value), String> {
        let url = format!("http://127.0.0.1:{CONTROLLER_PORT}{path}");
        let mut args: Vec<String> = vec![
            "-s".into(),
            "-S".into(),
            "--max-time".into(),
            timeout_secs.to_string(),
            "-X".into(),
            method.into(),
            "-H".into(),
            format!("Authorization: Bearer {}", self.secret),
        ];
        if let Some(body) = body {
            args.push("-H".into());
            args.push("Content-Type: application/json".into());
            args.push("--data-binary".into());
            args.push(body);
        }
        // 状态码单独输出到最后一行，便于区分 2xx 与 4xx/5xx。
        args.push("--write-out".into());
        args.push("\n%{http_code}".into());
        args.push(url);
        let result = command::run(&paths::curl_binary(), &args)
            .map_err(|e| format!("无法调用 curl：{e}"))?;
        if result.code != 0 {
            return Err(format!("内核控制接口调用失败：{path}"));
        }
        let (text, status) = match result.text.rsplit_once('\n') {
            Some((text, status)) => (text, status.trim().parse::<u16>().unwrap_or(0)),
            None => (result.text.as_str(), 0),
        };
        Ok((status, serde_json::from_str::<Value>(text).unwrap_or(Value::Null)))
    }

    /// 与 `request` 相同，但把 4xx/5xx 也视为失败。
    fn api(&self, path: &str, method: &str, body: Option<String>, timeout_secs: u64) -> Result<Value, String> {
        let (status, value) = self.request(path, method, body, timeout_secs)?;
        if (200..300).contains(&status) {
            Ok(value)
        } else {
            let detail = value
                .get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("未知错误");
            Err(format!("内核控制接口返回 {status}：{detail}"))
        }
    }

    pub fn version(&self) -> Result<Value, String> {
        if self.kind == KernelKind::Xray {
            // Xray 没有 Clash 控制接口；能连上 Stats API 即视为内核已就绪。
            let address = SocketAddr::from(([127, 0, 0, 1], XRAY_API_PORT));
            TcpStream::connect_timeout(&address, Duration::from_millis(800))
                .map(|_| serde_json::json!({ "version": "xray" }))
                .map_err(|_| "Xray 控制接口未就绪".to_string())
        } else {
            self.api("/version", "GET", None, 4)
        }
    }

    /// 拉取内核实时连接快照（对应 macOS 版 client.connections()）。
    pub fn connections(&self) -> Result<ConnectionSnapshot, String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 内核不提供实时连接列表".into());
        }
        let value = self.api("/connections", "GET", None, 5)?;
        serde_json::from_value(value).map_err(|e| format!("连接数据解析失败：{e}"))
    }

    pub fn select(&self, name: &str, group: &str) -> Result<(), String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 内核切换节点需要重启内核".into());
        }
        let body = serde_json::json!({ "name": name }).to_string();
        self.api(&format!("/proxies/{group}"), "PUT", Some(body), 5)
            .map(|_| ())
    }

    /// 切换流量模式：全局模式先让 GLOBAL 组跟随 SMART，再改内核 mode，最后回读确认。
    pub fn set_traffic_mode(&self, mode: TrafficMode) -> Result<(), String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 内核切换流量模式需要重启内核".into());
        }
        if mode == TrafficMode::Global {
            self.select("SMART", "GLOBAL")?;
        }
        let body = serde_json::json!({ "mode": mode.raw() }).to_string();
        self.api("/configs", "PATCH", Some(body), 5)?;
        let current = self.api("/configs", "GET", None, 4)?;
        if current.get("mode").and_then(|value| value.as_str()) == Some(mode.raw()) {
            Ok(())
        } else {
            Err("内核未确认流量模式，已尝试恢复原模式".into())
        }
    }

    /// 手动触发规则集更新。mihomo 会按 provider 配置的 `proxy: SMART` 下载并替换本地缓存，
    /// 失败时保留旧规则。
    pub fn update_rule_providers(&self) -> Result<(), String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 使用内置 geo 数据，无法在线更新规则".into());
        }
        let current = self.api("/providers/rules", "GET", None, 5)?;
        let providers = current
            .get("providers")
            .and_then(|value| value.as_object())
            .ok_or_else(|| "内核未返回规则集列表".to_string())?;
        for name in crate::logic::configuration::RULE_PROVIDERS {
            if !providers.contains_key(name) {
                return Err(format!("内核缺少规则集 {name}，请重新启动内核"));
            }
        }
        for name in crate::logic::configuration::RULE_PROVIDERS {
            self.api(&format!("/providers/rules/{name}"), "PUT", None, 30)
                .map_err(|error| format!("规则集 {name} 更新失败：{error}"))?;
        }
        Ok(())
    }

    /// 旧连接会保留切换前的路由，切换模式后必须断开。
    pub fn close_connections(&self) -> Result<(), String> {
        if self.kind == KernelKind::Xray {
            // Xray 没有连接控制接口；重启内核时连接自然重建。
            return Ok(());
        }
        self.api("/connections", "DELETE", None, 5).map(|_| ())
    }

    pub fn reload_config(&self, payload: &str) -> Result<(), String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 内核不支持热加载配置，请重启内核".into());
        }
        // Windows 命令行上限约 32K 字符，完整配置通常远超该值；
        // 写临时文件后让内核按路径读取，避免 CreateProcess 直接失败。
        let sequence = RELOAD_SEQ.fetch_add(1, Ordering::SeqCst);
        let temp = paths::data_root().join(format!(
            "reload-{}-{sequence}.json",
            std::process::id()
        ));
        paths::write_private(&temp, payload.as_bytes())?;
        let result = self.reload_config_path(&temp.display().to_string());
        let _ = std::fs::remove_file(&temp);
        result
    }

    pub fn reload_config_path(&self, path: &str) -> Result<(), String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 内核不支持热加载配置，请重启内核".into());
        }
        let body = serde_json::json!({ "path": path }).to_string();
        self.api("/configs?force=true", "PUT", Some(body), 10)
            .map(|_| ())
    }

    pub fn delay(&self, name: &str, timeout_ms: u64) -> NodeLatency {
        if self.kind == KernelKind::Xray {
            return NodeLatency::new(None, Some("请使用临时内核探测".into()));
        }
        let path = delay_path(name, timeout_ms);
        match self.api(&path, "GET", None, timeout_ms / 1000 + 2) {
            Ok(value) => {
                let milliseconds = value.get("delay").and_then(|v| v.as_i64());
                match milliseconds {
                    Some(ms) if ms >= 0 => NodeLatency {
                        milliseconds: Some(ms),
                        checked: now_ms(),
                        failure: None,
                    },
                    _ => NodeLatency::new(None, Some("未返回有效延迟".into())),
                }
            }
            Err(_) => NodeLatency::new(None, Some("超时或不可达".into())),
        }
    }

    pub fn internet_probe(&self, port: u16) -> ProbeResult {
        self.internet_probe_timeout(port, 6)
    }

    /// 与 `internet_probe` 相同，可指定总超时（秒）。
    pub fn internet_probe_timeout(&self, port: u16, timeout_secs: u64) -> ProbeResult {
        let start = Instant::now();
        let mut args = probe_args(
            port,
            3,
            timeout_secs.max(1),
            None,
            "https://cp.cloudflare.com/generate_204",
            false,
        );
        // 响应体输出到 NUL，必须让 curl 把状态码写到 stdout，否则永远解析成 0。
        args.push("--write-out".into());
        args.push("%{http_code}".into());
        let code = match command::run(&paths::curl_binary(), &args) {
            Ok(output) if output.code == 0 => output.text.trim().parse::<i64>().unwrap_or(0),
            _ => 0,
        };
        InternetPolicy::classify(code, start.elapsed().as_millis() as i64)
    }

    pub fn openai_probe(&self, port: u16) -> ProbeResult {
        let start = Instant::now();
        let mut args = probe_args(
            port,
            4,
            8,
            Some(131_072),
            "https://api.openai.com/v1/models",
            true,
        );
        args.push("--write-out".into());
        args.push("\n%{http_code}".into());
        let elapsed = || start.elapsed().as_millis() as i64;
        match command::run(&paths::curl_binary(), &args) {
            Ok(output) if output.code == 0 => {
                if let Some(cut) = output.text.rfind('\n') {
                    let body = &output.text[..cut];
                    let code = output.text[cut + 1..].trim().parse::<i64>().unwrap_or(0);
                    HealthPolicy::classify(code, body, elapsed())
                } else {
                    ProbeResult::new(
                        crate::logic::model::Health::Offline,
                        elapsed(),
                        "连接失败 / TLS 错误 / 超时".into(),
                    )
                }
            }
            _ => ProbeResult::new(
                crate::logic::model::Health::Offline,
                elapsed(),
                "连接失败 / TLS 错误 / 超时".into(),
            ),
        }
    }

    pub fn throughput(&self, port: u16) -> Option<ThroughputSample> {
        let mut args = probe_args(
            port,
            3,
            10,
            Some(1_048_576),
            "https://speed.cloudflare.com/__down?bytes=1048576",
            false,
        );
        args.push("-H".into());
        args.push("Accept-Encoding: identity".into());
        args.push("--write-out".into());
        args.push("%{speed_download}\n%{size_download}\n%{http_code}".into());
        let output = command::run(&paths::curl_binary(), &args).ok()?;
        if output.code != 0 {
            return None;
        }
        let fields: Vec<&str> = output.text.trim().split('\n').collect();
        if fields.len() != 3 || fields[2].trim() != "200" {
            return None;
        }
        let speed = fields[0].trim().parse::<f64>().ok()?;
        let bytes = fields[1].trim().parse::<f64>().ok()?;
        if speed <= 0.0 || bytes < 1_048_576.0 {
            return None;
        }
        Some(ThroughputSample {
            bytes_per_second: speed,
            downloaded_bytes: bytes as u64,
            checked: now_ms(),
        })
    }

    /// 内核累计流量（上行、下行）。Clash 系内核读 /connections 的累计值；
    /// Xray 通过 StatsService 汇总各入站计数。
    pub fn traffic_totals(&self) -> Result<(u64, u64), String> {
        if self.kind == KernelKind::Xray {
            return xray_traffic_totals();
        }
        let value = self.api("/connections", "GET", None, 4)?;
        let up = value
            .get("uploadTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let down = value
            .get("downloadTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok((up, down))
    }

    /// mihomo / sing-box 的 /traffic 是持续输出的流；用 curl 逐行读取。
    pub fn traffic_stream(&self) -> Result<Child, String> {
        if self.kind == KernelKind::Xray {
            return Err("Xray 内核不支持流量事件流".into());
        }
        let url = format!("http://127.0.0.1:{CONTROLLER_PORT}/traffic");
        Command::new(paths::curl_binary())
            .args([
                "-s",
                "-N",
                "--no-buffer",
                "--max-time",
                "0",
                "-H",
                &format!("Authorization: Bearer {}", self.secret),
                &url,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("无法读取内核流量统计：{e}"))
    }
}

/// 调用 Xray 内置 API 汇总入站流量；排除统计接口自身的流量。
fn xray_traffic_totals() -> Result<(u64, u64), String> {
    let exe = paths::core_binary(KernelKind::Xray);
    if !exe.exists() {
        return Err("缺少 Xray 内核".into());
    }
    let output = command::run(
        &exe,
        &[
            "api".into(),
            "statsquery".into(),
            format!("--server=127.0.0.1:{XRAY_API_PORT}"),
            "-pattern".into(),
            String::new(),
        ],
    )
    .map_err(|e| format!("无法读取 Xray 流量统计：{e}"))?;
    if output.code != 0 {
        return Err("Xray 流量统计不可用".into());
    }
    let value: Value =
        serde_json::from_str(&output.text).map_err(|e| format!("Xray 流量统计解析失败：{e}"))?;
    let mut up = 0u64;
    let mut down = 0u64;
    for stat in value
        .get("stat")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
    {
        let Some(name) = stat.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix("inbound>>>") else {
            continue;
        };
        let mut parts = rest.split(">>>traffic>>>");
        let tag = parts.next().unwrap_or_default();
        let direction = parts.next().unwrap_or_default();
        if tag == "api" {
            continue;
        }
        let value = stat
            .get("value")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
            .unwrap_or(0);
        match direction {
            "uplink" => up = up.saturating_add(value),
            "downlink" => down = down.saturating_add(value),
            _ => {}
        }
    }
    Ok((up, down))
}

fn probe_args(
    port: u16,
    connect_timeout: u64,
    max_time: u64,
    max_filesize: Option<u64>,
    url: &str,
    include_body: bool,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-s".into(),
        "-S".into(),
        "--noproxy".into(),
        String::new(),
        "--proxy".into(),
        format!("http://127.0.0.1:{port}"),
        "--connect-timeout".into(),
        connect_timeout.to_string(),
        "--max-time".into(),
        max_time.to_string(),
    ];
    if !include_body {
        args.push("--output".into());
        args.push("NUL".into());
    }
    args.push("--header".into());
    args.push("Cache-Control: no-cache".into());
    if let Some(size) = max_filesize {
        args.push("--max-filesize".into());
        args.push(size.to_string());
    }
    args.push(url.into());
    args
}

use crate::logic::health::{delay_path, now_ms, HealthPolicy, InternetPolicy};
use crate::logic::model::{NodeLatency, ProbeResult, ThroughputSample, TrafficMode, CONTROLLER_PORT};
use crate::services::{command, paths};
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone)]
pub struct CoreClient {
    pub secret: String,
}

impl CoreClient {
    pub fn new(secret: &str) -> Self {
        Self {
            secret: secret.to_string(),
        }
    }

    fn api(&self, path: &str, method: &str, body: Option<String>, timeout_secs: u64) -> Result<Value, String> {
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
        args.push(url);
        let result = command::run(&paths::curl_binary(), &args)
            .map_err(|e| format!("无法调用 curl：{e}"))?;
        if result.code != 0 {
            return Err(format!("内核控制接口调用失败：{path}"));
        }
        Ok(serde_json::from_str::<Value>(&result.text).unwrap_or(Value::Null))
    }

    pub fn version(&self) -> Result<Value, String> {
        self.api("/version", "GET", None, 4)
    }

    pub fn select(&self, name: &str, group: &str) -> Result<(), String> {
        let body = serde_json::json!({ "name": name }).to_string();
        self.api(&format!("/proxies/{group}"), "PUT", Some(body), 5)
            .map(|_| ())
    }

    /// 切换流量模式：全局模式先让 GLOBAL 组跟随 SMART，再改内核 mode，最后回读确认。
    pub fn set_traffic_mode(&self, mode: TrafficMode) -> Result<(), String> {
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

    /// 旧连接会保留切换前的路由，切换模式后必须断开。
    pub fn close_connections(&self) -> Result<(), String> {
        self.api("/connections", "DELETE", None, 5).map(|_| ())
    }

    pub fn reload_config(&self, payload: &str) -> Result<(), String> {
        let body = serde_json::json!({ "payload": payload }).to_string();
        self.api("/configs?force=true", "PUT", Some(body), 10)
            .map(|_| ())
    }

    pub fn reload_config_path(&self, path: &str) -> Result<(), String> {
        let body = serde_json::json!({ "path": path }).to_string();
        self.api("/configs?force=true", "PUT", Some(body), 10)
            .map(|_| ())
    }

    pub fn delay(&self, name: &str, timeout_ms: u64) -> NodeLatency {
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
        let start = Instant::now();
        let mut args = probe_args(
            port,
            3,
            6,
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

    /// mihomo 的 /traffic 是持续输出的流；用 curl 逐行读取。
    pub fn traffic_stream(&self) -> Result<Child, String> {
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

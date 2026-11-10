use super::smartclash_plan::{Candidate, Plan};
use crate::config::smartclash::Mode;
use anyhow::{Context as _, Result, ensure};
use reqwest::{Client, Response};
use serde_json::Value as Json;
use serde_yaml_ng::{Mapping, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    process::{Child, Command},
};

const HTTPS_TARGET: &str = "https://cp.cloudflare.com/generate_204";
const AI_TARGET: &str = "https://api.openai.com/v1/models";
const SPEED_TARGET: &str = "https://speed.cloudflare.com/__down?bytes=1048576";
const DOWNLOAD_LIMIT: usize = 1_048_576;

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub name: String,
    pub latency_ms: Option<u64>,
    pub bytes_per_second: Option<f64>,
    pub ai_reachable: Option<bool>,
}

impl Measurement {
    pub fn empty(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            latency_ms: None,
            bytes_per_second: None,
            ai_reachable: None,
        }
    }
}

pub fn classify_ai(status: u16, body: &[u8]) -> Option<bool> {
    if status == 0 || status >= 500 {
        return Some(false);
    }
    let value: Json = serde_json::from_slice(body).ok()?;
    let message = value
        .pointer("/error/message")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let code = value.pointer("/error/code").and_then(Json::as_str).unwrap_or_default();
    if code == "unsupported_country"
        || [
            "unsupported country",
            "country, region, or territory not supported",
            "not available in your country",
            "region is not supported",
            "country is not supported",
        ]
        .iter()
        .any(|marker| message.contains(marker))
    {
        return Some(false);
    }
    if (200..300).contains(&status)
        && value.get("object").and_then(Json::as_str) == Some("list")
        && value.get("data").is_some_and(Json::is_array)
    {
        return Some(true);
    }
    // 无密钥的结构化鉴权错误只能证明端点可达，不代表账号或模型权限。
    if status == 401 && (message.contains("api key") || message.contains("authentication")) {
        return Some(true);
    }
    None
}

pub fn isolated_config(plan: &Plan, candidate: &Candidate, port: u16, credential: &str) -> Result<Mapping> {
    let mut needed = HashSet::new();
    let mut cursor = Some(candidate.name.as_str());
    while let Some(name) = cursor {
        if name == "DIRECT" {
            break;
        }
        ensure!(
            needed.insert(name.to_owned()) && needed.len() <= 16,
            "探测链路循环或过深"
        );
        let node = plan
            .definitions
            .iter()
            .find(|node| node.get("name").and_then(Value::as_str) == Some(name))
            .context("隔离内核缺少链路定义")?;
        cursor = node.get("dialer-proxy").and_then(Value::as_str);
    }
    let proxies: Vec<_> = plan
        .definitions
        .iter()
        .filter(|node| {
            node.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| needed.contains(name))
        })
        .cloned()
        .collect();
    let mut config: Mapping = serde_yaml_ng::from_str(
        "mixed-port: 0\nport: 0\nsocks-port: 0\nredir-port: 0\ntproxy-port: 0\nallow-lan: false\nbind-address: 127.0.0.1\nexternal-controller: ''\nmode: rule\nlog-level: silent\nipv6: false\ntun: {enable: false}\ndns: {enable: false}\nprofile: {store-selected: false, store-fake-ip: false}\n",
    )?;
    config.insert("mixed-port".into(), port.into());
    config.insert("authentication".into(), vec![Value::from(credential)].into());
    config.insert("proxies".into(), proxies.into());
    let rule = serde_yaml_ng::to_value(
        serde_json::json!({"name": "SMARTCLASH-PROBE", "type": "select", "proxies": [candidate.name]}),
    )?;
    config.insert("proxy-groups".into(), vec![rule].into());
    config.insert("rules".into(), vec![Value::from("MATCH,SMARTCLASH-PROBE")].into());
    Ok(config)
}

struct ProbeCore {
    child: Option<Child>,
    directory: PathBuf,
}

impl Drop for ProbeCore {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let directory = self.directory.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                    let _ = tokio::fs::remove_dir_all(directory).await;
                });
            }
        }
    }
}

async fn start(binary: &Path, plan: &Plan, candidate: &Candidate) -> Result<(ProbeCore, Client)> {
    ensure!(binary.is_file(), "缺少隔离探测 Mihomo 内核：{}", binary.display());
    let reservation = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = reservation.local_addr()?.port();
    let token = nanoid::nanoid!(32);
    let credential = format!("smartclash:{token}");
    let config = isolated_config(plan, candidate, port, &credential)?;
    let root = crate::utils::dirs::app_home_dir()?;
    let directory = root.join(format!("smartclash-probe-{}-{}", std::process::id(), nanoid::nanoid!()));
    tokio::fs::create_dir_all(&root).await?;
    #[allow(unused_mut)]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let config_path = directory.join("probe.yaml");
    let write_config = (|| -> Result<()> {
        use std::io::Write as _;
        #[cfg(windows)]
        let mut file = crate::core::owner_identity::create_private_current_user_file(&config_path)?;
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&config_path)?
        };
        file.write_all(serde_yaml_ng::to_string(&config)?.as_bytes())?;
        Ok(())
    })();
    if let Err(error) = write_config {
        let _ = tokio::fs::remove_dir_all(&directory).await;
        return Err(error.into());
    }
    let mut command = Command::new(binary);
    command
        .arg("-d")
        .arg(&directory)
        .arg("-f")
        .arg(&config_path)
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    drop(reservation);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&directory).await;
            return Err(error).context("无法启动隔离探测内核");
        }
    };
    let mut core = ProbeCore {
        child: Some(child),
        directory,
    };
    let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?.basic_auth("smartclash", &token);
    let client = Client::builder()
        .no_proxy()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .no_gzip()
        .build()?;
    let mut readiness = tokio::time::interval(Duration::from_millis(100));
    for _ in 0..50 {
        readiness.tick().await;
        if let Some(status) = core.child.as_mut().context("隔离内核已释放")?.try_wait()? {
            anyhow::bail!("隔离探测内核提前退出：{status}");
        }
        if tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok()
        {
            return Ok((core, client));
        }
    }
    anyhow::bail!("隔离探测内核启动超时")
}

async fn limited_body(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(limit.min(16_384));
    while let Some(chunk) = response.chunk().await? {
        ensure!(body.len().saturating_add(chunk.len()) <= limit, "探测响应超过大小上限");
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn measure(binary: &Path, plan: &Plan, candidate: &Candidate, mode: Mode) -> Result<Measurement> {
    let (_core, client) = start(binary, plan, candidate).await?;
    let mut result = Measurement::empty(&candidate.name);
    let started = Instant::now();
    let response = client.get(HTTPS_TARGET).send().await?;
    ensure!(
        response.status().as_u16() == 204,
        "HTTPS 出网未通过：{}",
        response.status()
    );
    limited_body(response, 4096).await?;
    result.latency_ms = Some(started.elapsed().as_millis().max(1) as u64);
    if mode == Mode::Ai {
        result.ai_reachable = Some(false);
        if let Ok(response) = client.get(AI_TARGET).send().await {
            let status = response.status().as_u16();
            if let Ok(body) = limited_body(response, 65_536).await {
                result.ai_reachable = classify_ai(status, &body);
            }
        }
    }
    if mode == Mode::HighSpeed {
        let started = Instant::now();
        if let Ok(response) = client
            .get(SPEED_TARGET)
            .header(reqwest::header::RANGE, "bytes=0-1048575")
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(body) = limited_body(response, DOWNLOAD_LIMIT).await {
                    if body.len() == DOWNLOAD_LIMIT {
                        result.bytes_per_second = Some(body.len() as f64 / started.elapsed().as_secs_f64().max(0.001));
                    }
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_requires_structured_response_not_labels_or_generic_http_status() {
        assert_eq!(
            classify_ai(401, br#"{"error":{"message":"You didn't provide an API key"}}"#),
            Some(true)
        );
        assert_eq!(
            classify_ai(
                403,
                br#"{"error":{"message":"Country, region, or territory not supported"}}"#
            ),
            Some(false)
        );
        assert_eq!(classify_ai(429, br#"{"error":{"message":"rate limit"}}"#), None);
        assert_eq!(classify_ai(200, b"<html>AI available</html>"), None);
        assert_eq!(classify_ai(401, br#"{"message":"unauthorized"}"#), None);
    }

    #[test]
    fn isolated_config_has_no_tun_controller_providers_or_subscription_rules() -> Result<()> {
        let plan = Plan {
            definitions: vec![serde_yaml_ng::from_str(
                "{name: fixture, type: socks5, server: 127.0.0.1, port: 9}",
            )?],
            ..Plan::default()
        };
        let candidate = Candidate {
            name: "fixture".into(),
            chain: vec!["fixture".into()],
            hong_kong: false,
        };
        let config = isolated_config(&plan, &candidate, 12345, "fixture:password")?;
        assert_eq!(config["tun"]["enable"].as_bool(), Some(false));
        assert_eq!(config["dns"]["enable"].as_bool(), Some(false));
        assert_eq!(config["external-controller"].as_str(), Some(""));
        assert!(!config.contains_key("proxy-providers"));
        assert_eq!(config["rules"][0].as_str(), Some("MATCH,SMARTCLASH-PROBE"));
        assert_eq!(config["authentication"][0].as_str(), Some("fixture:password"));
        Ok(())
    }

    #[tokio::test]
    async fn fixture_response_body_is_bounded() -> Result<()> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut buffer = [0u8; 1024];
            stream.read(&mut buffer).await?;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n12345678")
                .await
        });
        let response = Client::builder()
            .no_proxy()
            .build()?
            .get(format!("http://{address}"))
            .send()
            .await?;
        assert!(limited_body(response, 4).await.is_err());
        server.await??;
        Ok(())
    }
}

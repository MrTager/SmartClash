use crate::services::core_client::CoreClient;
use crate::services::{command, paths};
use std::io::Write;
use std::net::TcpListener;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct CoreProcess {
    child: Option<Child>,
    tun_pid: Option<u32>,
    root: PathBuf,
    config_path: PathBuf,
    pid_path: PathBuf,
    exe: PathBuf,
}

impl CoreProcess {
    pub fn new() -> Self {
        let root = paths::data_root();
        Self {
            child: None,
            tun_pid: None,
            config_path: root.join("config.json"),
            pid_path: root.join("core.pid"),
            root,
            exe: paths::core_binary(),
        }
    }

    pub fn executable(&self) -> PathBuf {
        self.exe.clone()
    }

    pub fn recovery_needed(&self) -> bool {
        // TUN 模式只写 tun.pid（不写 core.pid）；两类 PID 文件任一存在都视为需要恢复。
        self.pid_path.exists() || self.root.join("tun.pid").exists()
    }

    pub fn start(&mut self, data: &[u8], tun: bool, client: &CoreClient) -> Result<(), String> {
        if self.child.is_some() {
            return Err("请先停止内核 / 恢复遗留服务".into());
        }
        if self.recovery_needed() {
            return Err("请先停止内核 / 恢复遗留服务".into());
        }
        if !self.exe.exists() {
            return Err(format!(
                "缺少 Mihomo 内核：{}，请重新安装完整安装包",
                self.exe.display()
            ));
        }
        // 拒绝接入其他程序占用的控制器或代理端口。
        for port in [crate::logic::model::CONTROLLER_PORT, crate::logic::model::PROXY_PORT, crate::logic::model::PROBE_PORT] {
            if port_in_use(port) {
                return Err(format!("端口 {port} 已占用，请关闭冲突的代理程序"));
            }
        }
        if tun && !paths::resource("wintun.dll").exists() {
            return Err("缺少 wintun.dll，无法启用 TUN；请重新安装完整安装包".into());
        }

        paths::write_private(&self.config_path, data)?;
        let test = command::run(
            &self.exe,
            &[
                "-t".into(),
                "-d".into(),
                self.root.display().to_string(),
                "-f".into(),
                self.config_path.display().to_string(),
            ],
        )
        .map_err(|e| format!("无法启动内核校验：{e}"))?;
        paths::write_private(&self.root.join("core-test.log"), test.text.as_bytes())?;
        if test.code != 0 {
            return Err("内核配置校验失败，请检查订阅节点格式（详情见 core-test.log）".into());
        }

        let result = if tun {
            self.start_elevated()
        } else {
            self.start_child()
        };
        if let Err(error) = result {
            let _ = self.stop();
            return Err(error);
        }

        for _ in 0..60 {
            if let Some(child) = self.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    let _ = self.stop();
                    return Err(format!("内核提前退出（{}），请查看 core.log", status.code().unwrap_or(-1)));
                }
            }
            if client.version().is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = self.stop();
        Err("内核启动超时，请检查 core.log".into())
    }

    fn start_child(&mut self) -> Result<(), String> {
        let log_path = self.root.join("core.log");
        let log = std::fs::File::create(&log_path).map_err(|e| format!("无法创建内核日志：{e}"))?;
        let err = log.try_clone().map_err(|e| format!("无法创建内核日志：{e}"))?;
        let child = Command::new(&self.exe)
            .args([
                "-d",
                &self.root.display().to_string(),
                "-f",
                &self.config_path.display().to_string(),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("无法启动内核：{e}"))?;
        paths::write_private(&self.pid_path, child.id().to_string().as_bytes())?;
        self.child = Some(child);
        Ok(())
    }

    fn start_elevated(&mut self) -> Result<(), String> {
        let pid_file = self.root.join("tun.pid");
        let _ = std::fs::remove_file(&pid_file);
        let log = self.root.join("core.log");
        let err_log = self.root.join("core.err.log");
        let script = self.root.join("start-tun.ps1");
        let content = format!(
            "$ErrorActionPreference = 'Stop'\n\
             $p = Start-Process -FilePath {} -ArgumentList @('-d',{},'-f',{}) -WorkingDirectory {} -WindowStyle Hidden -RedirectStandardOutput {} -RedirectStandardError {} -PassThru\n\
             Set-Content -LiteralPath {} -Value $p.Id -Encoding ascii\n",
            command::ps_quote(&self.exe.display().to_string()),
            command::ps_quote(&self.root.display().to_string()),
            command::ps_quote(&self.config_path.display().to_string()),
            command::ps_quote(&self.root.display().to_string()),
            command::ps_quote(&log.display().to_string()),
            command::ps_quote(&err_log.display().to_string()),
            command::ps_quote(&pid_file.display().to_string())
        );
        paths::write_private(&script, content.as_bytes())?;
        run_elevated(&script)?;

        // 等待管理员授权并写入 PID；取消 UAC 时脚本不会产生 PID。
        for _ in 0..120 {
            if let Some(text) = std::fs::read_to_string(&pid_file).ok() {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    self.tun_pid = Some(pid);
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err("管理员授权未完成或 TUN 内核未启动（可能取消了 UAC 授权）".into())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        let mut errors: Vec<String> = Vec::new();
        let mut attempted_tun_kill = false;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(pid) = self.tun_pid.take() {
            if process_alive(pid) {
                attempted_tun_kill = true;
                if let Err(e) = stop_elevated(pid) {
                    errors.push(e);
                }
            }
        }
        // 上次会话遗留的提权 TUN 内核：本实例没有句柄，只能从 tun.pid 找回。
        let tun_pid_path = self.root.join("tun.pid");
        if !attempted_tun_kill && tun_pid_path.exists() {
            let orphan = std::fs::read_to_string(&tun_pid_path)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            match orphan {
                Some(pid)
                    if process_alive(pid)
                        && process_image(pid).eq_ignore_ascii_case("mihomo.exe") =>
                {
                    if let Err(e) = stop_elevated(pid) {
                        errors.push(e);
                    }
                }
                _ => {
                    // 进程已不在（或 PID 已被复用），只清理标记文件。
                    let _ = std::fs::remove_file(&tun_pid_path);
                }
            }
        }
        if self.pid_path.exists() {
            if let Some(text) = std::fs::read_to_string(&self.pid_path).ok() {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    // 只终止可确认是本应用内核路径的遗留进程。
                    if process_alive(pid) && process_image(pid).eq_ignore_ascii_case("mihomo.exe") {
                        if let Err(e) = stop_elevated(pid) {
                            errors.push(e);
                        }
                    }
                }
            }
            let _ = std::fs::remove_file(&self.pid_path);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }
}

impl Default for CoreProcess {
    fn default() -> Self {
        Self::new()
    }
}

fn port_in_use(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

pub fn process_alive(pid: u32) -> bool {
    let result = command::run(
        Path::new("tasklist.exe"),
        &[
            "/FI".into(),
            format!("PID eq {pid}"),
            "/FO".into(),
            "CSV".into(),
            "/NH".into(),
        ],
    );
    match result {
        Ok(output) => output.text.to_lowercase().contains("\"mihomo.exe\""),
        Err(_) => false,
    }
}

fn process_image(pid: u32) -> String {
    let result = command::run(
        Path::new("tasklist.exe"),
        &[
            "/FI".into(),
            format!("PID eq {pid}"),
            "/FO".into(),
            "CSV".into(),
            "/NH".into(),
        ],
    );
    let Ok(output) = result else {
        return String::new();
    };
    output
        .text
        .split(',')
        .next()
        .unwrap_or("")
        .trim_matches('"')
        .to_string()
}

fn run_elevated(script: &Path) -> Result<(), String> {
    let script_path = script.display().to_string();
    let parts = [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script_path,
    ];
    let list = parts
        .iter()
        .map(|part| format!("'{}'", part.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let command_text = format!(
        // 不能等待提权脚本退出：脚本会启动常驻的 mihomo 进程，等待它退出会让 UI 永久 busy。
        "Start-Process -FilePath 'powershell.exe' -ArgumentList @({list}) -Verb RunAs -WindowStyle Hidden"
    );
    let result = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command_text])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("无法请求管理员授权：{e}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err("管理员操作未完成（可能取消授权）".into())
    }
}

fn stop_elevated(pid: u32) -> Result<(), String> {
    let root = paths::data_root();
    let script = root.join("stop-tun.ps1");
    let pid_file = root.join("tun.pid");
    let content = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         Stop-Process -Id {pid} -Force\n\
         Remove-Item -LiteralPath {} -Force\n",
        command::ps_quote(&pid_file.display().to_string())
    );
    let mut file = std::fs::File::create(&script).map_err(|e| format!("无法写入脚本：{e}"))?;
    let _ = file.write_all(content.as_bytes());
    drop(file);
    run_elevated(&script)?;
    let _ = std::fs::remove_file(&script);
    // 必须验证进程真的退出：提权脚本失败/被取消时不能谎报已恢复，
    // 否则 TUN 会继续劫持系统流量而界面以为已停止。
    for _ in 0..30 {
        if !process_alive(pid) {
            let _ = std::fs::remove_file(&pid_file);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("内核进程 {pid} 仍在运行（可能取消了管理员授权），TUN 可能仍接管网络"))
}

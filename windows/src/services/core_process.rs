use crate::logic::kernel::{KernelKind, XRAY_API_PORT, XRAY_SOCKS_PORT};
use crate::services::core_client::CoreClient;
use crate::services::{command, paths};
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
    kind: KernelKind,
}

impl CoreProcess {
    pub fn new(kind: KernelKind) -> Self {
        let root = paths::data_root();
        Self {
            child: None,
            tun_pid: None,
            config_path: root.join("config.json"),
            pid_path: root.join("core.pid"),
            root,
            exe: paths::core_binary(kind),
            kind,
        }
    }

    /// 用户切换内核后，重启进程时使用新的可执行文件。
    pub fn set_kind(&mut self, kind: KernelKind) {
        self.kind = kind;
        self.exe = paths::core_binary(kind);
    }

    pub fn kind(&self) -> KernelKind {
        self.kind
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
                "缺少 {} 内核：{}，请重新安装完整安装包",
                self.kind.title(),
                self.exe.display()
            ));
        }
        if tun && !self.kind.capabilities().tun {
            return Err(format!("{} 内核不支持 TUN，请改用 Mihomo 或 sing-box", self.kind.title()));
        }
        // 拒绝接入其他程序占用的控制器或代理端口。
        let ports: &[u16] = match self.kind {
            KernelKind::Mihomo | KernelKind::SingBox => &[
                crate::logic::model::CONTROLLER_PORT,
                crate::logic::model::proxy_port(),
                crate::logic::model::PROBE_PORT,
            ],
            KernelKind::Xray => &[
                crate::logic::model::proxy_port(),
                XRAY_SOCKS_PORT,
                XRAY_API_PORT,
            ],
        };
        for &port in ports {
            if port_in_use(port) {
                return Err(format!("端口 {port} 已占用，请关闭冲突的代理程序"));
            }
        }
        if tun && !paths::resource("wintun.dll").exists() {
            return Err("缺少 wintun.dll，无法启用 TUN；请重新安装完整安装包".into());
        }

        paths::write_private(&self.config_path, data)?;
        let test = self.run_check()?;
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

    /// 按内核类型执行配置校验。
    fn run_check(&self) -> Result<command::CmdOutput, String> {
        let args: Vec<String> = match self.kind {
            KernelKind::Mihomo => vec![
                "-t".into(),
                "-d".into(),
                self.root.display().to_string(),
                "-f".into(),
                self.config_path.display().to_string(),
            ],
            KernelKind::SingBox => vec![
                "check".into(),
                "-c".into(),
                self.config_path.display().to_string(),
                "-D".into(),
                self.root.display().to_string(),
            ],
            KernelKind::Xray => vec![
                "run".into(),
                "-test".into(),
                "-c".into(),
                self.config_path.display().to_string(),
            ],
        };
        let mut cmd = Command::new(&self.exe);
        cmd.args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null());
        if self.kind == KernelKind::Xray {
            cmd.env("XRAY_LOCATION_ASSET", paths::asset_dir());
        }
        let output = cmd
            .output()
            .map_err(|e| format!("无法启动内核校验：{e}"))?;
        Ok(command::CmdOutput {
            code: output.status.code().unwrap_or(-1),
            text: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    fn launch_args(&self) -> Vec<String> {
        match self.kind {
            KernelKind::Mihomo => vec![
                "-d".into(),
                self.root.display().to_string(),
                "-f".into(),
                self.config_path.display().to_string(),
            ],
            KernelKind::SingBox => vec![
                "run".into(),
                "-c".into(),
                self.config_path.display().to_string(),
                "-D".into(),
                self.root.display().to_string(),
            ],
            KernelKind::Xray => vec![
                "run".into(),
                "-c".into(),
                self.config_path.display().to_string(),
            ],
        }
    }

    fn apply_launch_env(&self, cmd: &mut Command) {
        if self.kind == KernelKind::Xray {
            cmd.env("XRAY_LOCATION_ASSET", paths::asset_dir());
        }
    }

    fn start_child(&mut self) -> Result<(), String> {
        let log_path = self.root.join("core.log");
        let log = std::fs::File::create(&log_path).map_err(|e| format!("无法创建内核日志：{e}"))?;
        let err = log.try_clone().map_err(|e| format!("无法创建内核日志：{e}"))?;
        let mut cmd = Command::new(&self.exe);
        cmd.args(self.launch_args())
            .current_dir(&self.root)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err));
        self.apply_launch_env(&mut cmd);
        let child = cmd.spawn().map_err(|e| format!("无法启动内核：{e}"))?;
        paths::write_private(&self.pid_path, child.id().to_string().as_bytes())?;
        self.child = Some(child);
        Ok(())
    }

    fn start_elevated(&mut self) -> Result<(), String> {
        let pid_file = self.root.join("tun.pid");
        let _ = std::fs::remove_file(&pid_file);
        let log = self.root.join("core.log");
        let err_log = self.root.join("core.err.log");
        let script = helper_script_path();
        let args = self
            .launch_args()
            .iter()
            .map(|arg| command::ps_quote(arg))
            .collect::<Vec<_>>()
            .join(",");
        let content = format!(
            "$ErrorActionPreference = 'Stop'\n\
             schtasks.exe /Delete /TN 'SmartClashTunHelper' /F 2>$null | Out-Null\n\
             $p = Start-Process -FilePath {} -ArgumentList @({}) -WorkingDirectory {} -WindowStyle Hidden -RedirectStandardOutput {} -RedirectStandardError {} -PassThru\n\
             Set-Content -LiteralPath {} -Value $p.Id -Encoding ascii\n",
            command::ps_quote(&self.exe.display().to_string()),
            args,
            command::ps_quote(&self.root.display().to_string()),
            command::ps_quote(&log.display().to_string()),
            command::ps_quote(&err_log.display().to_string()),
            command::ps_quote(&pid_file.display().to_string())
        );
        paths::write_private(&script, content.as_bytes())?;
        elevate_helper_script()?;

        // 等待提权脚本写入 PID；授权被取消或任务执行失败时不会产生 PID。
        for _ in 0..120 {
            if let Some(text) = std::fs::read_to_string(&pid_file).ok() {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    self.tun_pid = Some(pid);
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err("TUN 内核未启动（授权被取消或提权任务执行失败）".into())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        let mut errors: Vec<String> = Vec::new();
        let mut attempted_tun_kill = false;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(pid) = self.tun_pid.take() {
            if process_owned(pid) {
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
                    if process_owned(pid) =>
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
                    if process_owned(pid) {
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
        Self::new(KernelKind::Mihomo)
    }
}

fn port_in_use(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// 用对应内核校验一段配置，不启动常驻进程。
pub fn validate(kind: KernelKind, data: &[u8]) -> Result<(), String> {
    let exe = paths::core_binary(kind);
    if !exe.exists() {
        return Err(format!("缺少 {} 内核", kind.title()));
    }
    let root = paths::data_root();
    let dir = root.join(format!("config-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建校验目录：{e}"))?;
    let path = dir.join("config.json");
    paths::write_private(&path, data)?;
    let args: Vec<String> = match kind {
        KernelKind::Mihomo => vec![
            "-t".into(),
            "-d".into(),
            dir.display().to_string(),
            "-f".into(),
            path.display().to_string(),
        ],
        KernelKind::SingBox => vec![
            "check".into(),
            "-c".into(),
            path.display().to_string(),
            "-D".into(),
            dir.display().to_string(),
        ],
        KernelKind::Xray => vec![
            "run".into(),
            "-test".into(),
            "-c".into(),
            path.display().to_string(),
        ],
    };
    let mut cmd = Command::new(&exe);
    cmd.args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null());
    if kind == KernelKind::Xray {
        cmd.env("XRAY_LOCATION_ASSET", paths::asset_dir());
    }
    let output = cmd
        .output()
        .map_err(|e| format!("无法启动内核校验：{e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = paths::write_private(&root.join("core-test.log"), text.as_bytes());
    let _ = std::fs::remove_dir_all(&dir);
    if output.status.success() {
        Ok(())
    } else {
        Err("内核配置校验失败，请检查订阅节点格式（详情见 core-test.log）".into())
    }
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
        Ok(output) => {
            let text = output.text.to_lowercase();
            KernelKind::ALL
                .iter()
                .any(|kind| text.contains(&format!("\"{}\"", kind.binary_name())))
        }
        Err(_) => false,
    }
}

fn process_executable(pid: u32) -> Option<PathBuf> {
    let output = command::run(
        Path::new("powershell.exe"),
        &[
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            format!("(Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').ExecutablePath"),
        ],
    )
    .ok()?;
    let text = output.text.trim();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    fn normalize(path: &Path) -> String {
        path.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_lowercase()
    }
    normalize(left) == normalize(right)
}

/// 仅当 PID 仍存活且可执行文件就是本应用自带的任一内核时才认为归属本应用，
/// 避免 PID 复用后误杀其他代理客户端；允许任一内核是为了能在切换内核后清理遗留进程。
fn process_owned(pid: u32) -> bool {
    if !process_alive(pid) {
        return false;
    }
    process_executable(pid)
        .map(|actual| {
            KernelKind::ALL
                .iter()
                .any(|kind| same_path(&actual, &paths::core_binary(*kind)))
        })
        .unwrap_or(false)
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
    let pid_file = root.join("tun.pid");
    let script = helper_script_path();
    let content = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         schtasks.exe /Delete /TN 'SmartClashTunHelper' /F 2>$null | Out-Null\n\
         Stop-Process -Id {pid} -Force\n\
         Remove-Item -LiteralPath {} -Force\n",
        command::ps_quote(&pid_file.display().to_string())
    );
    paths::write_private(&script, content.as_bytes())?;
    elevate_helper_script()?;
    // 必须验证进程真的退出：提权脚本失败/被取消时不能谎报已恢复，
    // 否则 TUN 会继续劫持系统流量而界面以为已停止。
    for _ in 0..30 {
        if !process_alive(pid) {
            let _ = std::fs::remove_file(&pid_file);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("内核进程 {pid} 仍在运行（提权未完成或授权被取消），TUN 可能仍接管网络"))
}

// ---- 提权：每次都经过 UAC ------------------------------------------------
//
// 不再注册长期提权计划任务：任务会执行用户可写目录中的脚本与内核，
// 任何本地进程都能替换后被管理员执行。现在每次启动 / 停止 TUN 都通过
// Start-Process -Verb RunAs，由用户确认 UAC；脚本内容里顺带清理旧任务。

/// 提权脚本统一落点：每次操作前由应用改写内容（启动或停止）。
fn helper_script_path() -> PathBuf {
    paths::data_root().join("tun-elevated.ps1")
}

/// 以最高权限执行 helper_script_path() 指向的脚本（每次弹 UAC）。
fn elevate_helper_script() -> Result<(), String> {
    run_elevated(&helper_script_path())
}

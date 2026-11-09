//! 独立守护与受限 broker。内部入口在任何 GUI / HKCU 初始化前执行。
use super::{
    native::{self, Handle, Identity, Job},
    paths,
    system_proxy::SystemProxy,
};
use crate::logic::kernel::KernelKind;
use serde::{Deserialize, Serialize};
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    version: u32,
    nonce: String,
    owner: Identity,
    phase: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    version: u32,
    session: String,
    nonce: String,
    owner: Identity,
    kind: KernelKind,
    tun: bool,
    check: bool,
    config_hash: String,
    phase: String,
    broker: Option<Identity>,
    core: Option<Identity>,
}
struct Runtime {
    session: Session,
    guardian: Handle,
    stop: Handle,
    done: Handle,
    _ready: Handle,
    _job: Job,
    _recovery_handle: Handle,
    _proxy_mutex: Handle,
    _resources: Handle,
    closed: AtomicBool,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static SEALED: AtomicBool = AtomicBool::new(false);
static FINISH_GATE: Mutex<()> = Mutex::new(());
static GUARDIAN_FAILURE: Mutex<Option<String>> = Mutex::new(None);
static CHILDREN: OnceLock<Job> = OnceLock::new();
static SPAWN_GATE: Mutex<bool> = Mutex::new(false);

fn encode<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    native::atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
fn decode<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("恢复记录无效，保留供人工检查：{e}"))
}
fn restore_proxy() -> Result<(), String> {
    #[cfg(test)]
    if let Some(root) = std::env::var_os("SMARTCLASH_FIXTURE_ROOT") {
        let path = PathBuf::from(root).join("mock-proxy");
        if path.exists() {
            let value = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            if value == "applied" {
                native::atomic_write(&path, b"original")?;
            }
        }
        return Ok(());
    }
    SystemProxy::restore()
}
fn session_dir(nonce: &str) -> PathBuf {
    paths::data_root().join("sessions").join(nonce)
}
fn validate_session(nonce: &str) -> Result<Session, String> {
    let session = read_session(nonce)?;
    if !native::same_path(
        &session.owner.exe,
        &std::env::current_exe().map_err(|e| e.to_string())?,
    ) {
        return Err("活动会话固定程序路径不符".into());
    }
    Ok(session)
}
fn read_session(nonce: &str) -> Result<Session, String> {
    if !native::valid_nonce(nonce) {
        return Err("会话参数不合法".into());
    }
    let session: Session = decode(&session_dir(nonce).join("session.json"))?;
    if !["prepared", "guarding", "recovering", "clean"].contains(&session.phase.as_str()) {
        return Err("未知会话阶段，保留恢复线索".into());
    }
    if session.version != 1
        || session.nonce != nonce
        || session.owner.sid != native::user_sid()?
        || session.owner.pid == 0
        || session.owner.created == 0
        || !session.owner.exe.is_absolute()
    {
        return Err("会话身份、版本或记录路径不符".into());
    }
    Ok(session)
}
fn internal_command(mode: &str, session: &str, resource: Option<&str>) -> Result<Command, String> {
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    #[cfg(not(test))]
    command.args(internal_args(mode, session, resource));
    #[cfg(test)]
    {
        command.args([
            "--exact",
            "services::lifecycle::tests::fixture_entry",
            "--ignored",
            "--nocapture",
        ]);
        command.env(
            "SMARTCLASH_FIXTURE_INTERNAL",
            serde_json::to_string(&internal_args(mode, session, resource)).unwrap(),
        );
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths::data_root().join("fixture-diagnostic.log"))
            .unwrap();
        command
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
    }
    Ok(command)
}
fn internal_args(mode: &str, session: &str, resource: Option<&str>) -> Vec<String> {
    let mut args = vec![mode.into(), session.into()];
    if let Some(resource) = resource {
        args.push(resource.into());
    }
    args
}
fn hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}
pub fn internal_mode() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(mode) = args.first() else {
        return false;
    };
    if !["--guardian", "--core-broker", "--tun-broker"].contains(&mode.as_str()) {
        return false;
    }
    let result = match (mode.as_str(), args.len()) {
        ("--guardian", 2) => guardian(&args[1]),
        ("--core-broker", 3) => broker(&args[1], &args[2], false),
        ("--tun-broker", 3) => broker(&args[1], &args[2], true),
        _ => Err("内部模式参数数量不合法".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
    true
}

pub fn initialize() -> Result<(), String> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let _gate = native::lock(&native::domain("launch")?, 0)?;
    // guardian 持锁直到清理完成；新实例不能抢在旧会话恢复之前修改网络。
    let recovery = native::lock(&native::domain("recovery")?, 30_000)?;
    reconcile()?;
    restore_proxy()?;
    super::core_process::recover_legacy()?;
    let root = paths::ensure_root()?;
    native::protect(&root)?;
    let _pin = pin_directory(&root)?;
    let nonce = native::nonce()?;
    let dir = session_dir(&nonce);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    native::protect(&dir)?;
    let session = Session {
        version: 1,
        nonce: nonce.clone(),
        owner: native::current_identity()?,
        phase: "prepared".into(),
    };
    encode(&dir.join("session.json"), &session)?;
    let resources = native::mutex(&native::event_name(&nonce, "resources"))?;
    let ready = native::event(&nonce, "ready", true)?;
    let stop = native::event(&nonce, "stop", true)?;
    let done = native::event(&nonce, "done", true)?;
    let child = internal_command("--guardian", &nonce, None)?
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let guardian_id = native::identity_handle(child.as_raw_handle().cast(), child.id())?;
    encode(&dir.join("guardian.json"), &guardian_id)?;
    let guardian = native::open_identity(&guardian_id)?.ok_or("守护提前退出")?;
    drop(recovery);
    if !ready.wait(30_000)? || guardian.signaled()? {
        return Err("独立守护未就绪，拒绝启动内核或修改网络".into());
    }
    // guardian 已独立启动，不加入 GUI Job。后续普通子进程自动继承此 Job。
    let job = Job::new()?;
    job.assign_current()?;
    CHILDREN
        .set(Job::new()?)
        .map_err(|_| "子进程 Job 已初始化")?;
    let recovery_handle = native::mutex(&native::domain("recovery")?)?;
    let proxy_mutex = native::mutex(&native::domain("proxy")?)?;
    RUNTIME
        .set(Runtime {
            session,
            guardian,
            stop,
            done,
            _ready: ready,
            _job: job,
            _recovery_handle: recovery_handle,
            _proxy_mutex: proxy_mutex,
            _resources: resources,
            closed: AtomicBool::new(false),
        })
        .map_err(|_| "生命周期重复初始化")?;
    std::thread::Builder::new()
        .name("guardian-watchdog".into())
        .spawn(watch_guardian)
        .map_err(|e| {
            let _ = request_stop();
            format!("无法启动独立守护监测：{e}")
        })?;
    Ok(())
}
fn watch_guardian() {
    let runtime = RUNTIME.get().unwrap();
    loop {
        if runtime.closed.load(Ordering::SeqCst) {
            return;
        }
        match runtime.guardian.wait(100) {
            Ok(false) => continue,
            Ok(true) if runtime.done.signaled().unwrap_or(false) => return,
            result => {
                SEALED.store(true, Ordering::SeqCst);
                let reason = match result {
                    Err(error) => format!("独立守护监测失败：{error}"),
                    _ => "独立守护意外退出，资源入口已封闭".into(),
                };
                *GUARDIAN_FAILURE.lock().unwrap() = Some(reason.clone());
                let _ = request_stop();
                let _ = stop_children();
                loop {
                    match finish() {
                        Ok(()) => return,
                        Err(error) => {
                            *GUARDIAN_FAILURE.lock().unwrap() =
                                Some(format!("{reason}；恢复未完成：{error}"));
                            let _ = native::atomic_write(
                                &session_dir(&runtime.session.nonce).join("recovery-error.txt"),
                                error.as_bytes(),
                            );
                            std::thread::park_timeout(Duration::from_secs(1));
                        }
                    }
                }
            }
        }
    }
}
pub fn guardian_failure() -> Option<String> {
    GUARDIAN_FAILURE.lock().unwrap().clone()
}
pub fn request_stop() -> Result<(), String> {
    SEALED.store(true, Ordering::SeqCst);
    if let Some(runtime) = RUNTIME.get() {
        native::signal(&runtime.stop)?;
    }
    Ok(())
}
pub fn ensure_ready() -> Result<String, String> {
    if SEALED.load(Ordering::SeqCst) || !super::workers::valid() {
        return Err("退出或过期后台操作已封闭，拒绝修改网络和启动资源".into());
    }
    let runtime = RUNTIME.get().ok_or("独立守护尚未握手，操作被拒绝")?;
    if runtime.closed.load(Ordering::SeqCst)
        || runtime.stop.signaled()?
        || runtime.guardian.signaled()?
    {
        return Err("独立守护不可用，拒绝网络修改和内核启动".into());
    }
    if !native::mutex_held_by_other(&runtime._recovery_handle)? {
        return Err("守护恢复锁意外释放，拒绝接管网络".into());
    }
    Ok(runtime.session.nonce.clone())
}
pub fn finish() -> Result<(), String> {
    request_stop()?;
    let Some(runtime) = RUNTIME.get() else {
        return Ok(());
    };
    let _finish = FINISH_GATE
        .try_lock()
        .map_err(|_| "已有清理协调器正在运行，可重试或明确强制退出")?;
    if runtime.closed.load(Ordering::SeqCst) {
        return Ok(());
    }
    if runtime.guardian.signaled()? && !runtime.done.signaled()? {
        let _recovery = native::lock(&native::domain("recovery")?, 0)?;
        let proxy = restore_proxy();
        let resources = cleanup(&runtime.session);
        resources.and(proxy)?;
        let mut session = runtime.session.clone();
        session.phase = "clean".into();
        encode(&session_dir(&session.nonce).join("session.json"), &session)?;
        native::signal(&runtime.done)?;
        runtime.closed.store(true, Ordering::SeqCst);
        return Ok(());
    }
    if !runtime.done.wait(30_000)? {
        return Err("守护尚未确认清理完成，可重试或明确强制退出".into());
    }
    if !runtime.guardian.wait(5_000)? {
        return Err("守护尚未退出".into());
    }
    runtime.closed.store(true, Ordering::SeqCst);
    Ok(())
}

pub fn spawn_child(command: &mut Command) -> Result<Child, String> {
    let gate = SPAWN_GATE.lock().unwrap();
    if *gate {
        return Err("程序正在退出，拒绝启动探测或校验子进程".into());
    }
    ensure_ready()?;
    CHILDREN
        .get()
        .ok_or("子进程 Job 尚未初始化")?
        .spawn(command)
}
pub fn stop_children() -> Result<(), String> {
    *SPAWN_GATE.lock().unwrap() = true;
    if let Some(job) = CHILDREN.get() {
        job.terminate()?;
    }
    Ok(())
}
pub fn output(command: &mut Command) -> Result<std::process::Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = spawn_child(command)?;
    child.wait_with_output().map_err(|e| e.to_string())
}

pub struct Broker {
    session: String,
    nonce: String,
    process: Handle,
    stop: Handle,
    done: Handle,
    _ready: Handle,
}
impl Broker {
    pub fn start(kind: KernelKind, data: &[u8], tun: bool, check: bool) -> Result<Self, String> {
        let session = ensure_ready()?;
        let launch_gate = native::lock(&native::event_name(&session, "resources"), 30_000)?;
        ensure_ready()?;
        let nonce = native::nonce()?;
        let dir = session_dir(&session).join(&nonce);
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        native::protect(&dir)?;
        paths::write_private(&dir.join("config.json"), data)?;
        let resource = Resource {
            version: 1,
            session: session.clone(),
            nonce: nonce.clone(),
            owner: native::current_identity()?,
            kind,
            tun,
            check,
            config_hash: hash(data),
            phase: "prepared".into(),
            broker: None,
            core: None,
        };
        encode(&dir.join("resource.json"), &resource)?;
        let ready = native::event(&nonce, "ready", true)?;
        let stop = native::event(&nonce, "stop", true)?;
        let done = native::event(&nonce, "done", true)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mode = if tun { "--tun-broker" } else { "--core-broker" };
        let process = if tun {
            native::elevate(&exe, &format!("{mode} {session} {nonce}"))?
        } else {
            let child = internal_command(mode, &session, Some(&nonce))?
                .creation_flags(0x0800_0000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            let identity = native::identity_handle(child.as_raw_handle().cast(), child.id())?;
            native::open_identity(&identity)?.ok_or("broker 提前退出")?
        };
        let broker = Self {
            session,
            nonce,
            process,
            stop,
            done,
            _ready: ready,
        };
        drop(launch_gate);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if broker._ready.wait(100)? {
                return Ok(broker);
            }
            if broker.process.signaled()? {
                return Err("broker 拒绝启动，请检查会话身份、配置及打包资源".into());
            }
            ensure_ready()?;
        }
        Err("broker 未握手，未允许无约束内核运行".into())
    }
    pub fn alive(&self) -> Result<bool, String> {
        self.process.signaled().map(|v| !v)
    }
    pub fn recovery_needed(&self) -> bool {
        let path = session_dir(&self.session)
            .join(&self.nonce)
            .join("resource.json");
        match decode::<Resource>(&path) {
            Ok(resource) => {
                (resource.phase != "running" && resource.phase != "stopped")
                    || self.process.signaled().unwrap_or(true)
            }
            Err(_) => true,
        }
    }
    pub fn stop(&self) -> Result<(), String> {
        native::signal(&self.stop)?;
        if !self.process.wait(20_000)? {
            return Err(format!("broker {} 尚未确认退出", self.nonce));
        }
        // broker 意外死亡时 Job 同样关闭；恢复阶段仍会核实每个内核身份。
        if !self.done.signaled()? {
            let session = validate_session(&self.session)?;
            cleanup(&session)?;
        }
        Ok(())
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        let _ = native::signal(&self.stop);
    }
}

fn pin_read(path: &Path) -> Result<std::fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|e| e.to_string())?;
    reject_reparse(&file)?;
    Ok(file)
}
fn pin_directory(path: &Path) -> Result<std::fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x0220_0000)
        .open(path)
        .map_err(|e| e.to_string())?;
    reject_reparse(&file)?;
    Ok(file)
}
fn reject_reparse(file: &std::fs::File) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(native::error("无法查询受控资源"));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("受控资源不能是重解析点".into());
    }
    Ok(())
}
fn resource_dirs(session: &str) -> Result<Vec<PathBuf>, String> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(session_dir(session)).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            if !native::valid_nonce(&entry.file_name().to_string_lossy()) {
                return Err("未知会话资源目录，保留记录".into());
            }
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}
fn broker(session_nonce: &str, nonce: &str, elevated: bool) -> Result<(), String> {
    if !native::valid_nonce(nonce) {
        return Err("broker 参数不合法".into());
    }
    if elevated && !native::elevated()? {
        return Err("TUN broker 未经 UAC 提权，拒绝启动".into());
    }
    let session = validate_session(session_nonce)?;
    let owner = native::open_identity(&session.owner)?.ok_or("GUI 已结束，拒绝启动")?;
    let guardian_id: Identity = decode(&session_dir(session_nonce).join("guardian.json"))?;
    if guardian_id.sid != session.owner.sid
        || !native::same_path(&guardian_id.exe, &session.owner.exe)
    {
        return Err("守护身份不符".into());
    }
    let guardian = native::open_identity(&guardian_id)?.ok_or("守护已结束")?;
    let launch_gate = native::lock(&native::event_name(session_nonce, "resources"), 30_000)?;
    let dir = session_dir(session_nonce).join(nonce);
    // 打开并保持父目录/输入文件，不共享写入或删除，阻止 UAC 后的替换和重解析点跳转。
    let _root_pin = pin_directory(&paths::data_root())?;
    let _sessions_pin = pin_directory(&paths::data_root().join("sessions"))?;
    let _session_pin = pin_directory(&session_dir(session_nonce))?;
    let _resource_pin = pin_directory(&dir)?;
    let _config_pin = pin_read(&dir.join("config.json"))?;
    let mut resource: Resource = decode(&dir.join("resource.json"))?;
    if resource.version != 1
        || resource.session != session_nonce
        || resource.nonce != nonce
        || resource.owner != session.owner
        || resource.tun != elevated
        || resource.phase != "prepared"
    {
        return Err("broker 清单身份或阶段不合法".into());
    }
    let ready = native::event(nonce, "ready", false)?;
    let stop = native::event(nonce, "stop", false)?;
    let done = native::event(nonce, "done", false)?;
    let session_stop = native::event(session_nonce, "stop", false)?;
    let _single = native::lock(&native::event_name(nonce, "broker"), 0)?;
    let manifest_pin = pin_read(&dir.join("resource.json"))?;
    let resource_again: Resource = decode(&dir.join("resource.json"))?;
    if serde_json::to_vec(&resource_again).map_err(|e| e.to_string())?
        != serde_json::to_vec(&resource).map_err(|e| e.to_string())?
    {
        return Err("清单在验证期间被替换".into());
    }
    let data = std::fs::read(dir.join("config.json")).map_err(|e| e.to_string())?;
    if hash(&data) != resource.config_hash {
        return Err("受控配置摘要不符".into());
    }
    if elevated {
        if !resource.kind.capabilities().tun || resource.check {
            return Err("提权模式仅允许受控 TUN 内核".into());
        }
        validate_tun_config(resource.kind, &data)?;
    }
    #[cfg(not(test))]
    let exe = paths::core_binary(resource.kind)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    #[cfg(test)]
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(not(test))]
    if !native::same_path(&exe, &paths::core_binary(resource.kind)) {
        return Err("bundled 内核路径不符".into());
    }
    let _exe_parent_pin = pin_directory(exe.parent().ok_or("bundled 内核缺少父目录")?)?;
    let _exe_pin = pin_read(&exe)?;
    #[cfg(not(test))]
    let expected_hash = match resource.kind {
        KernelKind::Mihomo => env!("SMARTCLASH_MIHOMO_HASH"),
        KernelKind::SingBox => env!("SMARTCLASH_SINGBOX_HASH"),
        KernelKind::Xray => env!("SMARTCLASH_XRAY_HASH"),
    };
    #[cfg(not(test))]
    if hash(&std::fs::read(&exe).map_err(|e| e.to_string())?) != expected_hash {
        return Err("bundled 内核与构建时摘要不符，拒绝执行".into());
    }
    let dll = paths::resource("wintun.dll");
    let _dll_pin = if elevated {
        let pin = pin_read(&dll)?;
        if hash(&std::fs::read(&dll).map_err(|e| e.to_string())?) != env!("SMARTCLASH_WINTUN_HASH")
        {
            return Err("wintun 摘要不符".into());
        }
        Some(pin)
    } else {
        None
    };
    // 在启动之前持久化 broker 身份。CreateProcess 的 Job 属性原子约束首条指令。
    resource.broker = Some(native::current_identity()?);
    resource.phase = "launching".into();
    drop(manifest_pin);
    encode(&dir.join("resource.json"), &resource)?;
    if owner.signaled()? || guardian.signaled()? || stop.signaled()? || session_stop.signaled()? {
        return Err("会话已取消".into());
    }
    if resource.kind == KernelKind::Xray {
        std::env::set_var("XRAY_LOCATION_ASSET", paths::asset_dir());
    }
    // mihomo 1.19.6+ 将 /configs 热重载的 path 限制在工作目录或 SAFE_PATHS 内；
    // 热重载临时文件写入数据根目录（reload-<pid>-<seq>.json），需显式加入安全路径。
    std::env::set_var("SAFE_PATHS", paths::data_root());
    let job = Job::named(&native::event_name(nonce, "kernel-job"))?;
    // 提权内核只读取由 broker 重建、管理员拥有的工作目录，不能写同用户可变路径。
    let elevated_dir = if elevated {
        Some(prepare_admin_directory(&resource, &data, &exe, &dll)?)
    } else {
        None
    };
    let work = elevated_dir.as_deref().unwrap_or(&dir);
    let launch_exe = if elevated {
        work.join(resource.kind.binary_name())
    } else {
        exe.clone()
    };
    #[cfg(not(test))]
    let args = core_args(resource.kind, work, resource.check);
    #[cfg(test)]
    let args = vec![
        "--exact".into(),
        "services::lifecycle::tests::fixture_core".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    let mut log_options = std::fs::OpenOptions::new();
    use std::os::windows::fs::OpenOptionsExt;
    let log = if elevated {
        native::create_admin_file(&work.join("core.log"), &resource.owner.sid)?
    } else {
        log_options
            .write(true)
            .create_new(true)
            .share_mode(1)
            .open(work.join("core.log"))
            .map_err(|e| e.to_string())?
    };
    #[cfg(test)]
    if std::env::var_os("SMARTCLASH_FIXTURE_RECORD_CRASH").is_some() {
        let child = job.spawn_atomic(&launch_exe, &args, work, &log)?;
        encode(
            &paths::data_root().join("fixture-unrecorded-core.json"),
            &native::identity_handle(child.0, unsafe {
                windows_sys::Win32::System::Threading::GetProcessId(child.0)
            })?,
        )?;
        std::process::exit(91);
    }
    let child = job.spawn_atomic(&launch_exe, &args, work, &log)?;
    resource.core = Some(native::identity_handle(child.0, unsafe {
        windows_sys::Win32::System::Threading::GetProcessId(child.0)
    })?);
    resource.phase = "running".into();
    encode(&dir.join("resource.json"), &resource)?;
    native::signal(&ready)?;
    drop(launch_gate);
    loop {
        if owner.wait(100)?
            || guardian.signaled()?
            || stop.signaled()?
            || session_stop.signaled()?
            || child.signaled()?
        {
            break;
        }
    }
    job.terminate()?;
    if !child.wait(15_000)? {
        return Err("内核退出未确认".into());
    }
    resource.phase = "stopped".into();
    encode(&dir.join("resource.json"), &resource)?;
    drop(log);
    if let Some(root) = elevated_dir {
        std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    }
    native::signal(&done)
}
pub fn core_args(kind: KernelKind, dir: &Path, check: bool) -> Vec<String> {
    let config = dir.join("config.json").display().to_string();
    let dir = dir.display().to_string();
    match kind {
        KernelKind::Mihomo => {
            let mut args = vec!["-d".into(), dir, "-f".into(), config];
            if check {
                args.push("-t".into());
            }
            args
        }
        KernelKind::SingBox => vec![
            if check { "check" } else { "run" }.into(),
            "-c".into(),
            config,
            "-D".into(),
            dir,
        ],
        KernelKind::Xray => {
            let mut args = vec!["run".into(), "-c".into(), config];
            if check {
                args.push("-test".into());
            }
            args
        }
    }
}
fn admin_root() -> Result<PathBuf, String> {
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath},
    };
    unsafe {
        let mut text = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, std::ptr::null_mut(), &mut text) < 0 {
            return Err("无法读取系统 ProgramData 路径".into());
        }
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let root = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
            text, len,
        )));
        CoTaskMemFree(text.cast());
        Ok(root)
    }
}
fn admin_directory(resource: &Resource) -> Result<PathBuf, String> {
    Ok(admin_root()?.join(format!(
        "SmartClash-{}-{}",
        resource.session, resource.nonce
    )))
}
fn prepare_admin_directory(
    resource: &Resource,
    data: &[u8],
    exe: &Path,
    dll: &Path,
) -> Result<PathBuf, String> {
    let root = admin_directory(resource)?;
    // 随机新建，拒绝预置目录，不跟随同用户创建的 junction。
    native::create_admin_directory(&root, &resource.owner.sid)?;
    let _pin = pin_directory(&root)?;
    for (name, bytes) in [
        ("config.json", data.to_vec()),
        (
            resource.kind.binary_name(),
            std::fs::read(exe).map_err(|e| e.to_string())?,
        ),
        ("wintun.dll", std::fs::read(dll).map_err(|e| e.to_string())?),
    ] {
        use std::io::Write;
        let path = root.join(name);
        let mut file = native::create_admin_file(&path, &resource.owner.sid)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    // cache 目录在管理员私有根内新建，不能重定向至用户指定路径。
    native::create_admin_directory(&root.join("rules"), &resource.owner.sid)?;
    Ok(root)
}
fn validate_tun_config(kind: KernelKind, data: &[u8]) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_slice(data).map_err(|_| "提权配置必须为 JSON")?;
    let top = value.as_object().ok_or("提权配置必须为对象")?;
    let secret = match kind {
        KernelKind::Mihomo => value.get("secret"),
        KernelKind::SingBox => value.pointer("/experimental/clash_api/secret"),
        _ => None,
    };
    if secret
        .and_then(|v| v.as_str())
        .map(|s| s.len() < 16)
        .unwrap_or(true)
    {
        return Err("提权控制接口必须具有非空随机密钥".into());
    }
    let allowed: &[&str] = match kind {
        KernelKind::Mihomo => &[
            "mixed-port",
            "allow-lan",
            "bind-address",
            "mode",
            "log-level",
            "ipv6",
            "external-controller",
            "secret",
            "profile",
            "find-process-mode",
            "proxies",
            "proxy-groups",
            "listeners",
            "tun",
            "dns",
            "rule-providers",
            "rules",
            "interface-name",
        ],
        KernelKind::SingBox => &[
            "log",
            "inbounds",
            "outbounds",
            "route",
            "dns",
            "experimental",
        ],
        _ => return Err("此内核不允许提权".into()),
    };
    if top.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("提权配置出现非生成器字段，拒绝作为通用内核执行器".into());
    }
    if kind == KernelKind::Mihomo {
        if let Some(providers) = value.get("rule-providers").and_then(|v| v.as_object()) {
            for (name, provider) in providers {
                if !["cn-domains", "cn-custom", "cn-ips"].contains(&name.as_str())
                    || provider.get("path").and_then(|v| v.as_str())
                        != Some(format!("./rules/{name}.yaml").as_str())
                {
                    return Err("TUN 规则缓存路径不属于固定资源".into());
                }
            }
        }
    }
    match kind {
        KernelKind::Mihomo => {
            if value.get("allow-lan") != Some(&serde_json::json!(false))
                || value.get("bind-address").and_then(|v| v.as_str()) != Some("127.0.0.1")
                || value.get("external-controller").and_then(|v| v.as_str())
                    != Some("127.0.0.1:9090")
            {
                return Err("提权控制器和代理必须固定为本机监听".into());
            }
        }
        KernelKind::SingBox => {
            if let Some(inbounds) = value.get("inbounds").and_then(|v| v.as_array()) {
                for inbound in inbounds {
                    if inbound.get("type").and_then(|v| v.as_str()) != Some("tun")
                        && inbound.get("listen").and_then(|v| v.as_str()) != Some("127.0.0.1")
                    {
                        return Err("提权入站不允许公网监听".into());
                    }
                }
            }
            if value
                .pointer("/experimental/clash_api/external_controller")
                .and_then(|v| v.as_str())
                != Some("127.0.0.1:9090")
            {
                return Err("提权控制器地址不合法".into());
            }
        }
        _ => {}
    }
    // 不把用户可变配置当管理员通用执行器；拒绝外部文件、脚本和可重定向输出。
    fn safe(value: &serde_json::Value, kind: KernelKind, context: &str) -> Result<(), String> {
        match value {
            serde_json::Value::Object(map) => for (key, value) in map {
                let location = format!("{context}/{key}");
                if key == "path" {
                    let parts: Vec<_> = context.split('/').skip(1).collect();
                    let transport = parts.len() == 3 && parts[1].parse::<usize>().is_ok()
                        && match kind {
                            KernelKind::Mihomo => parts[0] == "proxies"
                                && ["ws-opts", "http-opts", "h2-opts"].contains(&parts[2]),
                            KernelKind::SingBox => parts[0] == "outbounds" && parts[2] == "transport"
                                && map.get("type").and_then(|v| v.as_str())
                                    .map(|v| ["ws", "http", "httpupgrade"].contains(&v)).unwrap_or(false),
                            _ => false,
                        };
                    let url_path = |v: &serde_json::Value| v.as_str().map(|path| {
                        path.starts_with('/') && !path.starts_with("//") && !path.contains('\\')
                    }).unwrap_or(false);
                    let allowed = if transport {
                        url_path(value) || (kind == KernelKind::Mihomo && parts[2] == "http-opts"
                            && value.as_array().map(|paths| paths.iter().all(url_path)).unwrap_or(false))
                    } else {
                        kind == KernelKind::Mihomo && parts.len() == 2 && parts[0] == "rule-providers"
                            && ["cn-domains", "cn-custom", "cn-ips"].contains(&parts[1])
                            && value.as_str() == Some(format!("./rules/{}.yaml", parts[1]).as_str())
                    };
                    if !allowed {
                        return Err(format!("TUN 配置包含外部路径或执行资源：{location}"));
                    }
                    continue;
                }
                if [
                    "file",
                    "proxy-providers",
                    "hosts",
                    "ca-cert",
                    "ca_str",
                    "ca",
                    "cert",
                    "key",
                    "client-cert",
                    "client-key",
                    "certificate-path",
                    "private-key-path",
                    "command",
                    "script",
                    "plugin",
                    "plugin-opts",
                    "external-ui",
                    "external-ui-url",
                    "external-ui-name",
                    "external-controller-unix",
                    "external-controller-pipe",
                    "certificate",
                    "private-key",
                    "certificate_path",
                    "key_path",
                    "output",
                    "cache_file",
                ]
                .contains(&key.as_str()) {
                    return Err(format!("TUN 配置包含外部路径或执行资源：{location}"));
                }
                safe(value, kind, &location)?;
            },
            serde_json::Value::Array(values) => for (index, value) in values.iter().enumerate() {
                safe(value, kind, &format!("{context}/{index}"))?;
            },
            _ => {},
        }
        Ok(())
    }
    safe(&value, kind, "")?;
    let enabled = match kind {
        KernelKind::Mihomo => value.pointer("/tun/enable").and_then(|v| v.as_bool()) == Some(true),
        KernelKind::SingBox => value
            .get("inbounds")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .any(|v| v.get("type").and_then(|v| v.as_str()) == Some("tun"))
            })
            .unwrap_or(false),
        _ => false,
    };
    if !enabled {
        return Err("提权配置没有固定 TUN 入站".into());
    }
    Ok(())
}
fn guardian(nonce: &str) -> Result<(), String> {
    let mut session = validate_session(nonce)?;
    let owner = native::open_identity(&session.owner)?.ok_or("GUI 已结束")?;
    let ready = native::event(nonce, "ready", false)?;
    let stop = native::event(nonce, "stop", false)?;
    let done = native::event(nonce, "done", false)?;
    let _resources = native::mutex(&native::event_name(nonce, "resources"))?;
    let _proxy_mutex = native::mutex(&native::domain("proxy")?)?;
    let _recovery = native::lock(&native::domain("recovery")?, 30_000)?;
    session.phase = "guarding".into();
    encode(&session_dir(nonce).join("session.json"), &session)?;
    native::signal(&ready)?;
    while !owner.wait(100)? && !stop.signaled()? {}
    session.phase = "recovering".into();
    encode(&session_dir(nonce).join("session.json"), &session)?;
    // 失败持续保留域锁，阻止新实例越过恢复。正常关闭可重试，强退仍由此进程负责。
    loop {
        let resources = cleanup(&session);
        let proxy = restore_proxy();
        match resources.and(proxy) {
            Ok(()) => break,
            Err(error) => {
                let _ = native::atomic_write(
                    &session_dir(nonce).join("recovery-error.txt"),
                    error.as_bytes(),
                );
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
    session.phase = "clean".into();
    encode(&session_dir(nonce).join("session.json"), &session)?;
    native::signal(&done)
}
fn cleanup(session: &Session) -> Result<(), String> {
    let _launch_gate = native::lock(&native::event_name(&session.nonce, "resources"), 30_000)?;
    for dir in resource_dirs(&session.nonce)? {
        let resource: Resource = decode(&dir.join("resource.json"))?;
        if resource.version != 1
            || !["prepared", "launching", "running", "stopped"].contains(&resource.phase.as_str())
        {
            return Err("未知资源版本或阶段，保留记录".into());
        }
        if resource.owner != session.owner
            || resource.session != session.nonce
            || !native::valid_nonce(&resource.nonce)
        {
            return Err("资源身份冲突，保留记录".into());
        }
        if let Ok(stop) = native::event(&resource.nonce, "stop", false) {
            native::signal(&stop)?;
        }
        if let Some(broker) = &resource.broker {
            if broker.sid != session.owner.sid
                || !native::same_path(&broker.exe, &session.owner.exe)
            {
                return Err("broker 身份冲突".into());
            }
            if let Some(handle) = native::open_identity(broker)? {
                if !handle.wait(20_000)? {
                    return Err("等待 broker 清理超时".into());
                }
            }
        }
        Job::wait_named_empty(&native::event_name(&resource.nonce, "kernel-job"))?;
        if let Some(core) = &resource.core {
            #[cfg(not(test))]
            let expected_exe = if resource.tun {
                admin_directory(&resource)?.join(resource.kind.binary_name())
            } else {
                let beside = session.owner.exe.parent().ok_or("记录程序缺少父目录")?
                    .join("resources").join(resource.kind.binary_name());
                let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources").join(resource.kind.binary_name());
                if native::same_path(&core.exe, &dev) { dev } else { beside }
            };
            #[cfg(test)]
            let expected_exe = session.owner.exe.clone();
            if core.sid != session.owner.sid || !native::same_path(&core.exe, &expected_exe) {
                return Err("内核身份冲突".into());
            }
            if let Some(handle) = native::open_identity(core)? {
                if !handle.wait(15_000)? {
                    return Err("内核仍在运行，拒绝丢弃记录".into());
                }
            }
        }
        // 不删除唯一身份线索，clean 会话保留可审计的无 secret 清单。
    }
    Ok(())
}
#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

fn reconcile() -> Result<(), String> {
    let root = paths::data_root().join("sessions");
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let nonce = entry.file_name().to_string_lossy().into_owned();
        let mut session = read_session(&nonce)?;
        if session.phase == "clean" {
            continue;
        }
        if native::open_identity(&session.owner)?.is_some() {
            return Err("已有同用户实例仍在运行".into());
        }
        let resources = cleanup(&session);
        let proxy = restore_proxy();
        resources.and(proxy)?;
        session.phase = "clean".into();
        encode(&entry.path().join("session.json"), &session)?;
    }
    Ok(())
}

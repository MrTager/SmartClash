use super::*;

#[test]
#[ignore]
fn fixture_entry() {
    let args: Vec<String> =
        serde_json::from_str(&std::env::var("SMARTCLASH_FIXTURE_INTERNAL").unwrap()).unwrap();
    match args[0].as_str() {
        "--guardian" => guardian(&args[1]).unwrap(),
        "--core-broker" => broker(&args[1], &args[2], false).unwrap(),
        "--fixture-owner" => {
            initialize().unwrap();
            if std::env::var_os("SMARTCLASH_FIXTURE_GATE").is_some() {
                crate::services::workers::with_epoch(0, || {
                    crate::services::workers::invalidate();
                    assert!(Broker::start(KernelKind::Mihomo, b"{}", false, false).is_err());
                });
            }
            let broker = Broker::start(KernelKind::Mihomo, b"{}", false, false).unwrap();
            crate::services::workers::invalidate();
            if std::env::var_os("SMARTCLASH_FIXTURE_PROBE").is_some() {
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "services::lifecycle::tests::fixture_core",
                        "--ignored",
                        "--nocapture",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let child = spawn_child(&mut command).unwrap();
                encode(
                    &paths::data_root().join("fixture-probe.json"),
                    &native::identity_handle(child.as_raw_handle().cast(), child.id()).unwrap(),
                )
                .unwrap();
            }
            let root = paths::data_root();
            native::atomic_write(&root.join("mock-proxy"), b"applied").unwrap();
            let session = ensure_ready().unwrap();
            native::atomic_write(&root.join("fixture-ready"), session.as_bytes()).unwrap();
            loop {
                if root.join("stop-owner").exists() {
                    crate::services::workers::close();
                    stop_children().unwrap();
                    assert!(Broker::start(KernelKind::Mihomo, b"{}", false, false).is_err());
                    broker.stop().unwrap();
                    if let Err(error) = finish() {
                        panic!(
                            "{error}；{}",
                            std::fs::read_to_string(
                                session_dir(&session).join("recovery-error.txt")
                            )
                            .unwrap_or_default()
                        );
                    }
                    native::atomic_write(&root.join("fixture-clean"), b"ok").unwrap();
                    return;
                }
                std::thread::park_timeout(Duration::from_millis(30));
            }
        }
        "--fixture-crash-before-record" => {
            initialize().unwrap();
            assert!(Broker::start(KernelKind::Mihomo, b"{}", false, false).is_err());
            let id: Identity =
                decode(&paths::data_root().join("fixture-unrecorded-core.json")).unwrap();
            wait_until(|| native::open_identity(&id).unwrap().is_none());
            finish().unwrap();
        }
        "--fixture-reconcile" => {
            let _lock = native::lock(&native::domain("recovery").unwrap(), 30_000).unwrap();
            reconcile().unwrap();
        }
        "--fixture-history" => {
            let root = paths::data_root();
            let nonce = native::nonce().unwrap();
            let dir = session_dir(&nonce);
            std::fs::create_dir_all(&dir).unwrap();
            let mut owner = native::current_identity().unwrap();
            owner.exe = root.join("old-portable").join("SmartClash.exe");
            let mut session = Session { version: 1, nonce, owner, phase: "clean".into() };
            encode(&dir.join("session.json"), &session).unwrap();
            reconcile().unwrap();
            assert!(validate_session(&session.nonce).err().unwrap().contains("固定程序路径"));
            session.phase = "recovering".into();
            encode(&dir.join("session.json"), &session).unwrap();
            assert!(reconcile().unwrap_err().contains("进程身份不符"));
            session.owner.sid.push('0');
            session.phase = "clean".into();
            encode(&dir.join("session.json"), &session).unwrap();
            assert!(reconcile().unwrap_err().contains("会话身份"));
        }
        _ => panic!("未知测试内部模式"),
    }
}
#[test]
#[ignore]
fn fixture_core() {
    // 仅测试二进制可进入；不启动真实内核、不访问注册表、不执行 UAC。
    loop {
        std::thread::park_timeout(Duration::from_secs(10));
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !condition() {
        assert!(Instant::now() < deadline, "多进程状态等待超时");
        std::thread::park_timeout(Duration::from_millis(30));
    }
}
struct Fixture {
    root: PathBuf,
    scope: String,
    owner: Child,
}
impl Fixture {
    fn new() -> Self {
        Self::with_gate(false)
    }
    fn with_gate(test_gate: bool) -> Self {
        let scope = native::nonce().unwrap();
        let root = std::env::temp_dir().join(format!("sc-lc-{}", &scope[..12]));
        std::fs::create_dir(&root).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "services::lifecycle::tests::fixture_entry",
                "--ignored",
                "--nocapture",
            ])
            .env("SMARTCLASH_FIXTURE_ROOT", &root)
            .env("SMARTCLASH_FIXTURE_SCOPE", &scope)
            .env(
                "SMARTCLASH_FIXTURE_INTERNAL",
                serde_json::to_string(&vec!["--fixture-owner"]).unwrap(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        command.env("SMARTCLASH_FIXTURE_PROBE", "1");
        if test_gate {
            command.env("SMARTCLASH_FIXTURE_GATE", "1");
        }
        let owner = command.spawn().unwrap();
        let mut fixture = Self { root, scope, owner };
        wait_until(|| {
            if let Some(status) = fixture.owner.try_wait().unwrap() {
                panic!(
                    "假 GUI 提前退出：{status}；诊断：{}",
                    std::fs::read_to_string(fixture.root.join("fixture-diagnostic.log"))
                        .unwrap_or_default()
                );
            }
            fixture.root.join("fixture-ready").exists()
        });
        fixture
    }
    fn session(&self) -> PathBuf {
        let nonce = std::fs::read_to_string(self.root.join("fixture-ready")).unwrap();
        self.root.join("sessions").join(nonce)
    }
    fn resource(&self) -> Resource {
        let entry = std::fs::read_dir(self.session())
            .unwrap()
            .flatten()
            .find(|e| e.file_type().unwrap().is_dir())
            .unwrap();
        decode(&entry.path().join("resource.json")).unwrap()
    }
    fn guardian(&self) -> Handle {
        let id: Identity = decode(&self.session().join("guardian.json")).unwrap();
        native::open_identity(&id).unwrap().unwrap()
    }
    fn assert_clean(&self) {
        wait_until(|| {
            let session: Session = decode(&self.session().join("session.json")).unwrap();
            session.phase == "clean"
        });
        assert_eq!(
            std::fs::read_to_string(self.root.join("mock-proxy")).unwrap(),
            "original"
        );
        let core = self.resource().core.unwrap();
        assert!(native::open_identity(&core).unwrap().is_none());
        let probe: Identity = decode(&self.root.join("fixture-probe.json")).unwrap();
        wait_until(|| native::open_identity(&probe).unwrap().is_none());
        // 重启独立进程跑同一恢复算法，证明清理结果可重复接管。
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::lifecycle::tests::fixture_entry",
                "--ignored",
                "--nocapture",
            ])
            .env("SMARTCLASH_FIXTURE_ROOT", &self.root)
            .env("SMARTCLASH_FIXTURE_SCOPE", &self.scope)
            .env("SMARTCLASH_FIXTURE_INTERNAL", "[\"--fixture-reconcile\"]")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.owner.kill();
        let _ = self.owner.wait();
        // 假守护应在父进程退出后自行完成；只清理测试专属目录。
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn multiprocess_parent_death_reclaims_core_and_proxy() {
    let mut fixture = Fixture::new();
    let guardian = fixture.guardian();
    fixture.owner.kill().unwrap();
    fixture.owner.wait().unwrap();
    assert!(guardian.wait(40_000).unwrap());
    fixture.assert_clean();
}
#[test]
fn multiprocess_broker_death_closes_kernel_job() {
    let mut fixture = Fixture::new();
    let resource = fixture.resource();
    let id = resource.broker.unwrap();
    let raw = unsafe {
        windows_sys::Win32::System::Threading::OpenProcess(
            windows_sys::Win32::System::Threading::PROCESS_TERMINATE,
            0,
            id.pid,
        )
    };
    assert!(!raw.is_null());
    let handle = Handle(raw);
    assert_ne!(
        unsafe { windows_sys::Win32::System::Threading::TerminateProcess(handle.0, 3) },
        0
    );
    let core = resource.core.unwrap();
    wait_until(|| native::open_identity(&core).unwrap().is_none());
    let guardian = fixture.guardian();
    fixture.owner.kill().unwrap();
    fixture.owner.wait().unwrap();
    assert!(guardian.wait(40_000).unwrap());
    fixture.assert_clean();
}
#[test]
fn multiprocess_guardian_death_forces_broker_stop_and_reconciliation() {
    let mut fixture = Fixture::new();
    let id: Identity = decode(&fixture.session().join("guardian.json")).unwrap();
    let raw = unsafe {
        windows_sys::Win32::System::Threading::OpenProcess(
            windows_sys::Win32::System::Threading::PROCESS_TERMINATE,
            0,
            id.pid,
        )
    };
    assert!(!raw.is_null());
    let handle = Handle(raw);
    assert_ne!(
        unsafe { windows_sys::Win32::System::Threading::TerminateProcess(handle.0, 4) },
        0
    );
    let core = fixture.resource().core.unwrap();
    wait_until(|| native::open_identity(&core).unwrap().is_none());
    // GUI 仍存活且没有 stop-owner；失效恢复不能依赖普通 workers 的 epoch。
    fixture.assert_clean();
    assert!(fixture.owner.try_wait().unwrap().is_none());
    assert!(!fixture.root.join("stop-owner").exists());
    native::atomic_write(&fixture.root.join("stop-owner"), b"stop").unwrap();
    wait_until(|| fixture.root.join("fixture-clean").exists());
    assert!(fixture.owner.wait().unwrap().success());
    fixture.assert_clean();
}
#[test]
fn multiprocess_stop_and_crash_race_is_idempotent() {
    let mut fixture = Fixture::new();
    let guardian = fixture.guardian();
    native::atomic_write(&fixture.root.join("stop-owner"), b"stop").unwrap();
    fixture.owner.kill().unwrap();
    fixture.owner.wait().unwrap();
    assert!(guardian.wait(40_000).unwrap());
    fixture.assert_clean();
}
#[test]
fn multiprocess_normal_stop_waits_for_clean_ack() {
    let mut fixture = Fixture::new();
    native::atomic_write(&fixture.root.join("stop-owner"), b"stop").unwrap();
    wait_until(|| fixture.root.join("fixture-clean").exists());
    assert!(fixture.owner.wait().unwrap().success());
    fixture.assert_clean();
}
#[test]
fn multiprocess_stale_epoch_cannot_start_resources() {
    let mut fixture = Fixture::with_gate(true);
    native::atomic_write(&fixture.root.join("stop-owner"), b"stop").unwrap();
    wait_until(|| fixture.root.join("fixture-clean").exists());
    assert!(fixture.owner.wait().unwrap().success());
    fixture.assert_clean();
}
#[test]
fn clean_history_allows_portable_move_but_unresolved_checks_recorded_identity() {
    let scope = native::nonce().unwrap();
    let root = std::env::temp_dir().join(format!("sc-hist-{}", &scope[..12]));
    std::fs::create_dir(&root).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "services::lifecycle::tests::fixture_entry", "--ignored", "--nocapture"])
        .env("SMARTCLASH_FIXTURE_ROOT", &root)
        .env("SMARTCLASH_FIXTURE_SCOPE", &scope)
        .env("SMARTCLASH_FIXTURE_INTERNAL", "[\"--fixture-history\"]")
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit())
        .status().unwrap();
    assert!(status.success());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn identity_conflict_does_not_touch_unrelated_process() {
    let identity = native::current_identity().unwrap();
    let mut reused = identity.clone();
    reused.created = reused.created.wrapping_add(1);
    assert!(native::open_identity(&reused).is_err());
    let mut wrong_sid = identity.clone();
    wrong_sid.sid.push('0');
    assert!(native::open_identity(&wrong_sid).is_err());
    let mut wrong_exe = identity.clone();
    wrong_exe.exe = PathBuf::from("not-smartclash.exe");
    assert!(native::open_identity(&wrong_exe).is_err());
    assert!(native::open_identity(&identity).unwrap().is_some());
}
#[test]
fn multiprocess_broker_crash_between_spawn_and_record_is_contained() {
    let scope = native::nonce().unwrap();
    let root = std::env::temp_dir().join(format!("sc-rec-{}", &scope[..12]));
    std::fs::create_dir(&root).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "services::lifecycle::tests::fixture_entry",
            "--ignored",
            "--nocapture",
        ])
        .env("SMARTCLASH_FIXTURE_ROOT", &root)
        .env("SMARTCLASH_FIXTURE_SCOPE", &scope)
        .env("SMARTCLASH_FIXTURE_RECORD_CRASH", "1")
        .env(
            "SMARTCLASH_FIXTURE_INTERNAL",
            "[\"--fixture-crash-before-record\"]",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn multiprocess_startup_waits_for_previous_guardian() {
    let mut fixture = Fixture::new();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "services::lifecycle::tests::fixture_lock_probe",
            "--ignored",
            "--nocapture",
        ])
        .env("SMARTCLASH_FIXTURE_SCOPE", &fixture.scope)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    native::atomic_write(&fixture.root.join("stop-owner"), b"stop").unwrap();
    wait_until(|| fixture.root.join("fixture-clean").exists());
    assert!(fixture.owner.wait().unwrap().success());
    fixture.assert_clean();
}
#[test]
#[ignore]
fn fixture_lock_probe() {
    assert!(native::lock(&native::domain("recovery").unwrap(), 50).is_err());
}
#[test]
fn shutdown_timeout_never_enters_held_lifecycle_lock() {
    let lock = Mutex::new(());
    let _held = lock.lock().unwrap();
    let entered = AtomicBool::new(false);
    let error = crate::state::actions::shutdown_barrier(
        || Err("模拟后台任务等待超时".into()),
        || {
            entered.store(true, Ordering::SeqCst);
            let _guard = lock.lock().unwrap();
            Ok(())
        },
    ).unwrap_err();
    assert_eq!(error, "模拟后台任务等待超时");
    assert!(!entered.load(Ordering::SeqCst));
}
#[test]
fn elevated_config_rejects_external_execution_and_files() {
    let generated = crate::logic::configuration::make_for(
        KernelKind::Mihomo,
        &[],
        "fixture-secret-not-a-real-node",
        true,
        None,
        &[],
        None,
        None,
        crate::logic::model::TrafficMode::Direct,
    )
    .unwrap();
    validate_tun_config(KernelKind::Mihomo, &generated.data).unwrap();
    let mut baseline: serde_json::Value = serde_json::from_slice(&generated.data).unwrap();
    baseline["proxies"] = serde_json::json!([{
        "name":"fake-ws", "type":"vmess", "server":"fixture.invalid", "port":443,
        "uuid":"00000000-0000-4000-8000-000000000000", "network":"ws",
        "ws-opts":{"path":"/ws"}
    }]);
    validate_tun_config(KernelKind::Mihomo, &serde_json::to_vec(&baseline).unwrap()).unwrap();
    for key in ["command", "script", "plugin", "output", "certificate_path", "path"] {
        let mut unsafe_value = baseline.clone();
        unsafe_value["proxies"][0]["ws-opts"]["nested"] = serde_json::json!({key:"C:/untrusted"});
        let error = validate_tun_config(KernelKind::Mihomo, &serde_json::to_vec(&unsafe_value).unwrap()).unwrap_err();
        assert_eq!(error, format!("TUN 配置包含外部路径或执行资源：/proxies/0/ws-opts/nested/{key}"));
    }
    for path in ["//external/share", "C:/other", "/ws\\escape"] {
        let mut unsafe_value = baseline.clone();
        unsafe_value["proxies"][0]["ws-opts"]["path"] = serde_json::json!(path);
        assert_eq!(validate_tun_config(KernelKind::Mihomo, &serde_json::to_vec(&unsafe_value).unwrap()).unwrap_err(),
            "TUN 配置包含外部路径或执行资源：/proxies/0/ws-opts/path");
    }
    let generated = crate::logic::configuration::make_for(
        KernelKind::SingBox,
        &[],
        "fixture-secret-not-a-real-node",
        true,
        None,
        &[],
        None,
        None,
        crate::logic::model::TrafficMode::Direct,
    )
    .unwrap();
    validate_tun_config(KernelKind::SingBox, &generated.data).unwrap();
}

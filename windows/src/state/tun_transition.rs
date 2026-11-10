/// A TUN restart temporarily removes the local proxy endpoint. Restore the
/// system proxy only after the replacement core is ready and verified.
pub trait TunRuntime {
    fn stop(&mut self) -> Result<(), String>;
    fn start(&mut self, tun: bool) -> Result<(), String>;
    fn verify_tun(&mut self) -> Result<(), String>;
    fn enable_system_proxy(&mut self) -> Result<(), String>;
}

pub fn restart(runtime: &mut impl TunRuntime, tun: bool, system_proxy: bool) -> Result<(), String> {
    runtime.stop()?;
    let result = runtime
        .start(tun)
        .and_then(|_| if tun { runtime.verify_tun() } else { Ok(()) });
    if let Err(error) = result {
        return match runtime.stop() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!(
                "{error}；自动恢复未完成（{cleanup}），请点击“恢复网络”"
            )),
        };
    }
    if system_proxy {
        // Failure here must not stop an otherwise healthy TUN core.
        runtime.enable_system_proxy()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Runtime {
        running: bool,
        tun: bool,
        system_proxy: bool,
        fail_start: bool,
        fail_verify: bool,
        fail_proxy: bool,
        fail_cleanup: bool,
        stops: usize,
        events: Vec<&'static str>,
    }

    impl TunRuntime for Runtime {
        fn stop(&mut self) -> Result<(), String> {
            self.events.push("stop");
            self.stops += 1;
            if self.fail_cleanup && self.stops > 1 {
                return Err("cleanup failed".into());
            }
            self.running = false;
            self.tun = false;
            self.system_proxy = false;
            Ok(())
        }

        fn start(&mut self, tun: bool) -> Result<(), String> {
            self.events.push("start");
            if self.fail_start {
                return Err("start failed".into());
            }
            self.running = true;
            self.tun = tun;
            Ok(())
        }

        fn verify_tun(&mut self) -> Result<(), String> {
            self.events.push("verify");
            if self.fail_verify {
                Err("verification failed".into())
            } else {
                Ok(())
            }
        }

        fn enable_system_proxy(&mut self) -> Result<(), String> {
            assert!(
                self.running,
                "never point the system proxy at a stopped core"
            );
            self.events.push("proxy");
            if self.fail_proxy {
                return Err("proxy failed".into());
            }
            self.system_proxy = true;
            Ok(())
        }
    }

    #[test]
    fn toggling_tun_preserves_the_other_switch_in_all_combinations() {
        for tun in [false, true] {
            for system_proxy in [false, true] {
                let mut runtime = Runtime {
                    running: true,
                    tun: !tun,
                    system_proxy,
                    ..Default::default()
                };
                restart(&mut runtime, tun, system_proxy).unwrap();
                assert!(runtime.running);
                assert_eq!(runtime.tun, tun);
                assert_eq!(runtime.system_proxy, system_proxy);
                let mut expected = vec!["stop", "start"];
                if tun {
                    expected.push("verify");
                }
                if system_proxy {
                    expected.push("proxy");
                }
                assert_eq!(runtime.events, expected);
            }
        }
    }

    #[test]
    fn failed_start_or_verification_restores_network_without_reenabling_proxy() {
        for fail_start in [false, true] {
            let mut runtime = Runtime {
                system_proxy: true,
                fail_start,
                fail_verify: !fail_start,
                ..Default::default()
            };
            let error = restart(&mut runtime, true, true).unwrap_err();
            assert_eq!(
                error,
                if fail_start {
                    "start failed"
                } else {
                    "verification failed"
                }
            );
            assert!(!runtime.running);
            assert!(!runtime.tun);
            assert!(!runtime.system_proxy);
            assert_eq!(runtime.stops, 2);
            assert!(!runtime.events.contains(&"proxy"));
        }
    }

    #[test]
    fn proxy_failure_keeps_a_verified_tun_running() {
        let mut runtime = Runtime {
            fail_proxy: true,
            ..Default::default()
        };
        assert_eq!(
            restart(&mut runtime, true, true).unwrap_err(),
            "proxy failed"
        );
        assert!(runtime.running);
        assert!(runtime.tun);
        assert!(!runtime.system_proxy);
        assert_eq!(runtime.stops, 1);
    }

    #[test]
    fn cleanup_failure_is_reported_with_the_original_error() {
        let mut runtime = Runtime {
            fail_verify: true,
            fail_cleanup: true,
            ..Default::default()
        };
        let error = restart(&mut runtime, true, true).unwrap_err();
        assert!(error.contains("verification failed"));
        assert!(error.contains("cleanup failed"));
        assert!(error.contains("恢复网络"));
        assert!(!runtime.events.contains(&"proxy"));
    }
}

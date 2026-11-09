//! 登记所有后台任务。退出先永久封闭入口，再取消、回收子进程并等待任务归零。
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
static EPOCH: AtomicU64 = AtomicU64::new(0);
static EXITING: AtomicBool = AtomicBool::new(false);
thread_local! { static WORK_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
pub fn valid() -> bool {
    !EXITING.load(Ordering::SeqCst)
        && WORK_EPOCH.with(|epoch| {
            epoch
                .get()
                .map(|value| value == EPOCH.load(Ordering::SeqCst))
                .unwrap_or(true)
        })
}
pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}
pub fn current_epoch() -> u64 {
    WORK_EPOCH
        .with(|epoch| epoch.get())
        .unwrap_or_else(|| EPOCH.load(Ordering::SeqCst))
}
pub fn with_epoch<R>(epoch: u64, operation: impl FnOnce() -> R) -> R {
    WORK_EPOCH.with(|value| {
        let previous = value.replace(Some(epoch));
        struct Reset<'a>(&'a std::cell::Cell<Option<u64>>, Option<u64>);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _reset = Reset(value, previous);
        operation()
    })
}
#[derive(Default)]
struct State {
    closed: bool,
    active: usize,
}
static STATE: Mutex<State> = Mutex::new(State {
    closed: false,
    active: 0,
});
static IDLE: Condvar = Condvar::new();
struct Ticket;
impl Ticket {
    fn enter() -> Option<Self> {
        let mut state = STATE.lock().unwrap();
        if state.closed {
            return None;
        }
        state.active += 1;
        Some(Self)
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        let mut state = STATE.lock().unwrap();
        state.active -= 1;
        IDLE.notify_all();
    }
}
pub fn spawn(operation: impl FnOnce() + Send + 'static) {
    let Some(ticket) = Ticket::enter() else {
        return;
    };
    let epoch = current_epoch();
    std::thread::spawn(move || {
        let _ticket = ticket;
        with_epoch(epoch, operation);
    });
}
pub fn close() {
    STATE.lock().unwrap().closed = true;
    EXITING.store(true, Ordering::SeqCst);
    invalidate();
}
pub fn wait() -> Result<(), String> {
    let state = STATE.lock().unwrap();
    let (state, _) = IDLE
        .wait_timeout_while(state, std::time::Duration::from_secs(30), |s| s.active != 0)
        .unwrap();
    if state.active == 0 {
        Ok(())
    } else {
        Err(format!("仍有 {} 个在途后台任务，可重试退出", state.active))
    }
}

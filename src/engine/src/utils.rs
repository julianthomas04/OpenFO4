use std::collections::HashSet;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut core::ffi::c_void;
    fn SetThreadPriority(handle: *mut core::ffi::c_void, priority: i32) -> i32;
}

/// An optional debug environment variable: unset is `None` and stays silent,
/// a value that is not valid Unicode is reported (it would otherwise read as
/// unset) and ignored.
pub fn optional_env_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(value)) => {
            tracing::warn!("{name} is not valid Unicode ({value:?}); ignoring it");
            None
        }
    }
}

/// Drop the calling thread to below-normal scheduling priority. Background
/// loading threads (cell workers, cache loader pool) call this so they can
/// saturate every core during loading-mode spawns while the main thread —
/// which must keep integrating results — always wins the scheduler.
/// Measured before this: any loader configuration wide enough to keep up
/// with 31 workers halved the main thread's frame rate and integration
/// throughput collapsed 5x. No-op on non-Windows.
pub fn lower_thread_priority() {
    #[cfg(windows)]
    unsafe {
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

static LOGGED_ONCE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn first_time(key: &str) -> bool {
    let set = LOGGED_ONCE.get_or_init(|| Mutex::new(HashSet::new()));
    set.lock().unwrap().insert(key.to_string())
}

/// Logs the message with `tracing::warn!`, but only the first time `key` is seen. Use for
/// per-asset failures that would otherwise spam every streamed cell or frame.
pub fn warn_once(key: &str, msg: impl FnOnce() -> String) {
    if first_time(key) {
        tracing::warn!("{}", msg());
    }
}

/// `tracing::error`! counterpart of [`warn_once`] for invariant violations.
pub fn error_once(key: &str, msg: impl FnOnce() -> String) {
    if first_time(key) {
        tracing::error!("{}", msg());
    }
}

/// `tracing::debug`! counterpart of [`warn_once`] for diagnostics worth noting once per asset.
pub fn debug_once(key: &str, msg: impl FnOnce() -> String) {
    if first_time(key) {
        tracing::debug!("{}", msg());
    }
}

/// `tracing::info`! counterpart of [`warn_once`] for expected, non-failure
/// outcomes worth noting once per asset.
pub fn info_once(key: &str, msg: impl FnOnce() -> String) {
    if first_time(key) {
        tracing::info!("{}", msg());
    }
}

pub struct ProcessMemory {
    sys: sysinfo::System,
    pid: Option<sysinfo::Pid>,
    cached_bytes: u64,
    frames_since_refresh: u32,
}

impl Default for ProcessMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessMemory {
    pub fn new() -> Self {
        let pid = sysinfo::get_current_pid().ok();
        let mut s = Self {
            sys: sysinfo::System::new(),
            pid,
            cached_bytes: 0,
            frames_since_refresh: u32::MAX,
        };
        s.refresh();
        s
    }

    fn refresh(&mut self) {
        if let Some(pid) = self.pid {
            self.sys
                .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            if let Some(proc) = self.sys.process(pid) {
                self.cached_bytes = proc.memory();
            }
        }
        self.frames_since_refresh = 0;
    }

    pub fn bytes(&mut self) -> u64 {
        self.frames_since_refresh += 1;
        if self.frames_since_refresh >= 60 {
            self.refresh();
        }
        self.cached_bytes
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// The warning for an environment variable whose value is not used: the variable, the value
/// and the reason, so a bad value can be fixed without reading the source.
fn invalid_value_message(name: &str, raw: &str, reason: &dyn Display, outcome: &str) -> String {
    format!("{name}={raw:?} is not a valid value ({reason}); {outcome}")
}

/// Reads env var `name` parsed as `T`; `default` when unset, and a warning plus
/// `default` when the value does not parse.
pub fn env_or<T: FromStr>(name: &str, default: T) -> T
where
    T::Err: Display,
{
    env_or_valid(name, default, |_| true)
}

/// Like `env_or`, but a parsed value failing `is_valid` also warns and takes
/// `default`.
pub fn env_or_valid<T: FromStr>(name: &str, default: T, is_valid: impl Fn(&T) -> bool) -> T
where
    T::Err: Display,
{
    resolve_env_or(name, std::env::var(name).ok().as_deref(), default, is_valid)
}

fn resolve_env_or<T: FromStr>(
    name: &str,
    raw: Option<&str>,
    default: T,
    is_valid: impl Fn(&T) -> bool,
) -> T
where
    T::Err: Display,
{
    const USING_DEFAULT: &str = "using the default";
    let Some(raw) = raw else {
        return default;
    };
    match raw.trim().parse() {
        Ok(value) if is_valid(&value) => value,
        Ok(_) => {
            let reason = "rejected by the variable's validity check";
            tracing::warn!(
                "{}",
                invalid_value_message(name, raw, &reason, USING_DEFAULT)
            );
            default
        }
        Err(error) => {
            tracing::warn!(
                "{}",
                invalid_value_message(name, raw, &error, USING_DEFAULT)
            );
            default
        }
    }
}

/// Like `env_or`, but `None` when the variable is unset or invalid; the warning
/// for an invalid value carries the parser's reason (for a name list, the
/// valid names).
pub fn env_opt<T: FromStr>(name: &str) -> Option<T>
where
    T::Err: Display,
{
    resolve_env_opt(name, std::env::var(name).ok().as_deref())
}

fn resolve_env_opt<T: FromStr>(name: &str, raw: Option<&str>) -> Option<T>
where
    T::Err: Display,
{
    let raw = raw?;
    match raw.trim().parse() {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                "{}",
                invalid_value_message(name, raw, &error, "ignoring it")
            );
            None
        }
    }
}

/// `1`/`true` -> `true`, `0`/`false` -> `false` (case-insensitive, trimmed);
/// anything else is not a flag.
fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

/// Reads env var `name` as a flag (`1`/`true`/`0`/`false`). Reports invalid
/// values on stderr rather than through `tracing`, so it is usable before the
/// logging subscriber is installed.
pub fn env_flag(name: &str, default: bool) -> bool {
    let Ok(raw) = std::env::var(name) else {
        return default;
    };
    parse_flag(&raw).unwrap_or_else(|| {
        eprintln!("{name}={raw:?} is not 1/true/0/false; using the default");
        default
    })
}

const DROP_QUEUE_CAPACITY: usize = 256;

static DROP_SENDER: OnceLock<std::sync::mpsc::SyncSender<Box<dyn std::any::Any + Send>>> =
    OnceLock::new();
static DROP_SEND_FAILURE_LOGGED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Free `value` on a background thread. Large payloads (decoded cell results,
/// texture bytes) cost milliseconds to free; the frame loop hands them here
/// instead. The queue is bounded: when it is full, or the drop thread is
/// gone, the value is freed inline (the disconnected case is logged once).
pub fn drop_off_thread<T: Send + 'static>(value: T) {
    use std::sync::mpsc::TrySendError;
    let sender = DROP_SENDER.get_or_init(|| {
        let (tx, rx) =
            std::sync::mpsc::sync_channel::<Box<dyn std::any::Any + Send>>(DROP_QUEUE_CAPACITY);
        let spawned = std::thread::Builder::new()
            .name("deferred-drop".into())
            .spawn(move || {
                lower_thread_priority();
                for item in rx {
                    drop(item);
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("deferred-drop thread failed to spawn: {e}; dropping inline");
        }
        tx
    });
    match sender.try_send(Box::new(value)) {
        Ok(()) => {}
        Err(TrySendError::Full(unsent)) => {
            warn_once("deferred-drop-queue-full", || {
                format!("deferred-drop queue is full ({DROP_QUEUE_CAPACITY}); dropping inline")
            });
            drop(unsent);
        }
        Err(TrySendError::Disconnected(unsent)) => {
            if !DROP_SEND_FAILURE_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!("deferred-drop thread is gone; dropping inline");
            }
            drop(unsent);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;

    #[test]
    fn parse_flag_accepts_one_and_true_in_any_case() {
        for raw in ["1", "true", "TRUE", " True "] {
            assert_eq!(parse_flag(raw), Some(true), "{raw:?}");
        }
    }

    #[test]
    fn parse_flag_accepts_zero_and_false_as_off() {
        for raw in ["0", "false", "FALSE", " 0 "] {
            assert_eq!(parse_flag(raw), Some(false), "{raw:?}");
        }
    }

    #[test]
    fn parse_flag_rejects_other_values() {
        for raw in ["", "yes", "2", "on", "-1"] {
            assert_eq!(parse_flag(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn resolve_env_or_uses_default_when_unset() {
        assert_eq!(resolve_env_or("X", None, 7_u32, |_| true), 7);
    }

    #[test]
    fn resolve_env_or_parses_trimmed_value() {
        assert_eq!(resolve_env_or("X", Some(" 12 "), 7_u32, |_| true), 12);
    }

    #[test]
    fn resolve_env_or_falls_back_on_unparsable_value() {
        assert_eq!(resolve_env_or("X", Some("many"), 7_u32, |_| true), 7);
    }

    #[test]
    fn resolve_env_or_falls_back_on_negative_for_unsigned() {
        assert_eq!(resolve_env_or("X", Some("-3"), 7_u32, |_| true), 7);
    }

    #[test]
    fn resolve_env_or_falls_back_when_value_fails_validation() {
        assert_eq!(resolve_env_or("X", Some("0"), 4_usize, |&n| n >= 1), 4);
    }

    #[test]
    fn resolve_env_or_keeps_value_passing_validation() {
        assert_eq!(resolve_env_or("X", Some("1"), 4_usize, |&n| n >= 1), 1);
    }

    #[test]
    fn resolve_env_opt_is_none_when_unset_or_invalid() {
        assert_eq!(resolve_env_opt::<u32>("X", None), None);
        assert_eq!(resolve_env_opt::<u32>("X", Some("abc")), None);
    }

    #[test]
    fn the_invalid_value_warning_names_the_variable_the_value_and_the_reason() {
        let reason = "many".parse::<u32>().unwrap_err();
        let message = invalid_value_message("OPENFO4_SOMETHING", "many", &reason, "ignoring it");
        for part in ["OPENFO4_SOMETHING", "many", &reason.to_string()] {
            assert!(message.contains(part), "{part:?} missing from {message:?}");
        }
    }

    #[test]
    fn resolve_env_opt_parses_valid_value() {
        assert_eq!(resolve_env_opt::<f32>("X", Some("1.5")), Some(1.5));
    }

    #[test]
    fn format_bytes_uses_the_largest_whole_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
    }
}

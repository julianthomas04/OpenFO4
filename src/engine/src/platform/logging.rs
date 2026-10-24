use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry};

pub const LOGS_DIR: &str = "logs";
pub const SESSIONS_KEPT: usize = 10;
const DEFAULT_LOG_FILTER: &str = "warn,openfo4=debug";
const FILE_PREFIX: &str = "openfo4-";
const FILE_EXTENSION: &str = ".log";
const LINE_TIMESTAMP_FORMAT: &[BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]");
const TIMESTAMP_LEN: usize = "YYYY-MM-DD_HH-MM-SS".len();

/// Keeps the non-blocking writer's worker alive. It lives in a global so the panic
/// hook can flush it too: with `panic = "abort"` no destructor runs after a panic.
static WORKER_GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("cannot create the log folder {}: {source}", path.display())]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot open the log file {}: {source}", path.display())]
    OpenFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot duplicate the handle of the log file {}: {source}", path.display())]
    CloneHandle {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Reported to the caller instead of stopping the program when file logging cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingWarning(String);

impl LoggingWarning {
    fn file_logging_disabled(cause: &LoggingError) -> Self {
        Self(format!("File logging is disabled: {cause}"))
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LoggingWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTimestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl SessionTimestamp {
    pub fn at(moment: OffsetDateTime) -> Self {
        Self {
            year: moment.year() as u16,
            month: moment.month() as u8,
            day: moment.day(),
            hour: moment.hour(),
            minute: moment.minute(),
            second: moment.second(),
        }
    }

    fn formatted(self) -> String {
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

pub fn session_log_file_name(timestamp: SessionTimestamp) -> String {
    format!("{FILE_PREFIX}{}{FILE_EXTENSION}", timestamp.formatted())
}

fn is_session_log_name(name: &str) -> bool {
    let Some(stamp) = name
        .strip_prefix(FILE_PREFIX)
        .and_then(|rest| rest.strip_suffix(FILE_EXTENSION))
    else {
        return false;
    };
    stamp.len() == TIMESTAMP_LEN
        && stamp.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 | 13 | 16 => byte == b'-',
            10 => byte == b'_',
            _ => byte.is_ascii_digit(),
        })
}

/// Deletes all but the `keep` newest session logs (names sort chronologically).
/// Returns one message per failure; cleanup problems are never fatal.
pub fn prune_session_logs(logs_dir: &Path, keep: usize) -> Vec<String> {
    let entries = match std::fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(error) => {
            return vec![format!(
                "cannot list {} for cleanup: {error}",
                logs_dir.display()
            )];
        }
    };
    let mut warnings = Vec::new();
    let mut names = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                if let Some(name) = entry
                    .file_name()
                    .to_str()
                    .filter(|n| is_session_log_name(n))
                {
                    names.push(name.to_owned());
                }
            }
            Err(error) => warnings.push(format!(
                "cannot read an entry of {}: {error}",
                logs_dir.display()
            )),
        }
    }
    names.sort_unstable_by(|a, b| b.cmp(a));
    for name in names.into_iter().skip(keep) {
        let path = logs_dir.join(&name);
        if let Err(error) = std::fs::remove_file(&path) {
            warnings.push(format!("cannot delete old log {}: {error}", path.display()));
        }
    }
    warnings
}

pub fn create_session_log(
    logs_dir: &Path,
    timestamp: SessionTimestamp,
) -> Result<(File, PathBuf), LoggingError> {
    std::fs::create_dir_all(logs_dir).map_err(|source| LoggingError::CreateDir {
        path: logs_dir.to_path_buf(),
        source,
    })?;
    let path = logs_dir.join(session_log_file_name(timestamp));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|source| LoggingError::OpenFile {
            path: path.clone(),
            source,
        })?;
    Ok((file, path))
}

fn log_filter() -> (EnvFilter, Option<String>) {
    match std::env::var("RUST_LOG") {
        Ok(directives) => match EnvFilter::try_new(&directives) {
            Ok(filter) => (filter, None),
            Err(error) => (
                EnvFilter::new(DEFAULT_LOG_FILTER),
                Some(format!("Ignoring invalid RUST_LOG {directives:?}: {error}")),
            ),
        },
        Err(std::env::VarError::NotPresent) => (EnvFilter::new(DEFAULT_LOG_FILTER), None),
        Err(std::env::VarError::NotUnicode(value)) => (
            EnvFilter::new(DEFAULT_LOG_FILTER),
            Some(format!(
                "Ignoring RUST_LOG, it is not valid unicode: {}",
                value.display()
            )),
        ),
    }
}

fn span_events() -> FmtSpan {
    if crate::utils::env_flag("OPENFO4_LOG_SPANS", false) {
        FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    }
}

fn flush_logs() {
    let mut guard = WORKER_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    drop(guard.take());
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(
            || "unknown location".to_owned(),
            |location| format!("{}:{}", location.file(), location.line()),
        );
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic at {location}: {message}\n{backtrace}");
        previous(info);
        flush_logs();
    }));
}

/// Flushes the file log when dropped; hold it for the whole process.
#[must_use = "dropping the session flushes and closes the log file"]
pub struct LogSession {
    log_file: Option<PathBuf>,
}

impl LogSession {
    pub fn log_file(&self) -> Option<&Path> {
        self.log_file.as_deref()
    }
}

impl Drop for LogSession {
    fn drop(&mut self) {
        flush_logs();
    }
}

struct FileSink {
    file: File,
    path: PathBuf,
}

/// The local UTC offset, read once so the file name and every line share one clock.
/// Reading it can fail on platforms where it is unsound once threads exist; the
/// failure is returned so the caller can report the fallback to UTC.
fn local_offset() -> (UtcOffset, Option<String>) {
    match UtcOffset::current_local_offset() {
        Ok(offset) => (offset, None),
        Err(error) => (
            UtcOffset::UTC,
            Some(format!(
                "Cannot determine the local time zone ({error}); log times are UTC"
            )),
        ),
    }
}

fn open_file_sink(logs_dir: &Path, offset: UtcOffset) -> Result<FileSink, LoggingError> {
    let started = SessionTimestamp::at(OffsetDateTime::now_utc().to_offset(offset));
    let (file, path) = create_session_log(logs_dir, started)?;
    Ok(FileSink { file, path })
}

/// Installs the global subscriber: the console in debug builds, and a per-session file
/// in `<install_dir>/logs`. A file-log failure never stops the program: logging
/// continues without the file, the failure is printed to stderr and returned for the
/// caller to surface in the UI.
pub fn init(install_dir: &Path) -> (LogSession, Option<LoggingWarning>) {
    let logs_dir = install_dir.join(LOGS_DIR);
    let (offset, offset_warning) = local_offset();
    let line_timer = OffsetTime::new(offset, LINE_TIMESTAMP_FORMAT);
    let (sink, failure) = match open_file_sink(&logs_dir, offset) {
        Ok(sink) => (Some(sink), None),
        Err(error) => (None, Some(error)),
    };
    let prune_warnings = if sink.is_some() {
        prune_session_logs(&logs_dir, SESSIONS_KEPT)
    } else {
        Vec::new()
    };

    let (file_filter, filter_warning) = log_filter();
    let mut guard = None;
    let mut failure = failure;
    let mut active_path = None;
    let file_layer = sink.and_then(|sink| match sink.file.try_clone() {
        Ok(file) => {
            let (writer, worker_guard) = tracing_appender::non_blocking(file);
            guard = Some(worker_guard);
            active_path = Some(sink.path);
            Some(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_timer(line_timer.clone())
                    .with_writer(writer)
                    .with_span_events(span_events())
                    .with_filter(file_filter),
            )
        }
        Err(source) => {
            failure = Some(LoggingError::CloneHandle {
                path: sink.path,
                source,
            });
            None
        }
    });
    let console_layer = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer()
            .with_timer(line_timer)
            .with_span_events(span_events())
            .with_filter(log_filter().0)
    });

    let registry = Registry::default().with(file_layer).with(console_layer);
    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());
    registry.init();

    *WORKER_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = guard;
    install_panic_hook();

    if let Some(warning) = offset_warning {
        tracing::warn!("{warning}");
    }
    if let Some(warning) = filter_warning {
        tracing::warn!("{warning}");
    }
    for warning in &prune_warnings {
        tracing::warn!("{warning}");
    }
    let warning = failure.as_ref().map(LoggingWarning::file_logging_disabled);
    if let Some(warning) = &warning {
        eprintln!("warning: {warning}");
        tracing::warn!("{warning}");
    }

    (
        LogSession {
            log_file: active_path,
        },
        warning,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(second: u8) -> SessionTimestamp {
        SessionTimestamp {
            year: 2026,
            month: 10,
            day: 5,
            hour: 7,
            minute: 3,
            second,
        }
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    #[test]
    fn the_file_name_is_the_zero_padded_session_start() {
        assert_eq!(
            session_log_file_name(stamp(9)),
            "openfo4-2026-10-05_07-03-09.log"
        );
        assert!(is_session_log_name(&session_log_file_name(stamp(9))));
    }

    #[test]
    fn pruning_keeps_the_newest_sessions_and_deletes_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        for second in 0..5 {
            touch(dir.path(), &session_log_file_name(stamp(second)));
        }
        let warnings = prune_session_logs(dir.path(), 3);
        assert_eq!(warnings, [] as [String; 0]);
        let mut left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [2, 3, 4].map(|second| session_log_file_name(stamp(second)))
        );
    }

    #[test]
    fn pruning_leaves_files_that_are_not_session_logs() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "notes.txt",
            "openfo4-latest.log",
            "openfo4-2026-10-05_07-03-0x.log",
            "other-2026-10-05_07-03-01.log",
        ] {
            touch(dir.path(), name);
        }
        touch(dir.path(), &session_log_file_name(stamp(1)));
        touch(dir.path(), &session_log_file_name(stamp(2)));
        prune_session_logs(dir.path(), 1);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 5);
        assert!(!dir.path().join(session_log_file_name(stamp(1))).exists());
    }

    #[test]
    fn pruning_a_missing_folder_is_a_warning_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(prune_session_logs(&dir.path().join("absent"), 10).len(), 1);
    }

    #[test]
    fn creating_the_session_log_creates_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join(LOGS_DIR);
        let (_, path) = create_session_log(&logs, stamp(1)).unwrap();
        assert_eq!(path, logs.join(session_log_file_name(stamp(1))));
        assert!(path.is_file());
    }

    #[test]
    fn an_unwritable_log_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("logs");
        std::fs::write(&blocker, b"a file, not a folder").unwrap();
        let error = create_session_log(&blocker, stamp(1)).unwrap_err();
        assert!(matches!(error, LoggingError::CreateDir { .. }));
        assert!(error.to_string().contains("cannot create the log folder"));
    }

    #[test]
    fn an_unopenable_log_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(session_log_file_name(stamp(1)))).unwrap();
        let error = create_session_log(dir.path(), stamp(1)).unwrap_err();
        assert!(matches!(error, LoggingError::OpenFile { .. }));
    }

    #[test]
    fn the_warning_names_the_cause() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("logs");
        std::fs::write(&blocker, b"x").unwrap();
        let cause = create_session_log(&blocker, stamp(1)).unwrap_err();
        let warning = LoggingWarning::file_logging_disabled(&cause);
        assert!(warning.message().starts_with("File logging is disabled: "));
        assert!(warning.message().contains("cannot create the log folder"));
    }

    #[test]
    fn the_timestamp_uses_the_given_offset() {
        let utc = time::macros::datetime!(2026-10-05 23:03:09 UTC);
        let local = utc.to_offset(time::macros::offset!(+2));
        let stamp = SessionTimestamp::at(local);
        assert_eq!(
            session_log_file_name(stamp),
            "openfo4-2026-10-06_01-03-09.log"
        );
        assert!(is_session_log_name(&session_log_file_name(stamp)));
    }

    #[test]
    fn line_timestamps_are_local_with_milliseconds() {
        let local = time::macros::datetime!(2026-10-05 23:03:09.0425 +2);
        assert_eq!(
            local.format(LINE_TIMESTAMP_FORMAT).unwrap(),
            "2026-10-05 23:03:09.042"
        );
    }
}

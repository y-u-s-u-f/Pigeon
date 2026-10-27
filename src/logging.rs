//! The log file, `<data dir>/pigeon.log` (sprint 2, W1). Launched from the
//! Dock, stderr goes nowhere, so it goes to the file then; from a terminal
//! it stays there and `info` lines go to both. gpui's `log` output comes
//! here too. The file is capped: past `CAP` it moves to `pigeon.log.1`.

use std::{
    fs::{File, OpenOptions},
    io::{IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    sync::Mutex,
};

const CAP: u64 = 2 * 1024 * 1024;

struct LogFile {
    path: PathBuf,
    file: File,
    /// stderr is the file, so `eprintln!` lands there too.
    redirected: bool,
}

static LOG: Mutex<Option<LogFile>> = Mutex::new(None);

unsafe extern "C" {
    fn dup2(from: i32, to: i32) -> i32;
}

/// The log file's path in `dir`.
pub fn path(dir: &Path) -> PathBuf {
    dir.join("pigeon.log")
}

/// Start logging to `dir`'s log file, and take gpui's `log` output.
pub fn init(dir: &Path) {
    let path = path(dir);
    let _ = std::fs::create_dir_all(dir);
    rotate_if_full(&path, CAP);
    let Ok(file) = open(&path) else {
        return;
    };
    let redirected = !std::io::stderr().is_terminal() && redirect_stderr(&file);
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(LogFile {
        path,
        file,
        redirected,
    });
    if log::set_logger(&GPUI_LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    info(&format!("Pigeon {} started", env!("CARGO_PKG_VERSION")));
}

fn open(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn redirect_stderr(file: &File) -> bool {
    use std::os::fd::AsRawFd as _;
    // SAFETY: both are open descriptors; fd 2 now refers to the file.
    unsafe { dup2(file.as_raw_fd(), 2) >= 0 }
}

/// Moves a log past `cap` bytes to `<name>.1`, replacing the older one.
/// True when it did.
pub fn rotate_if_full(path: &Path, cap: u64) -> bool {
    let full = std::fs::metadata(path).is_ok_and(|m| m.len() > cap);
    if full {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    full
}

/// One line, timestamped, in the log file (and on stderr from a terminal).
pub fn info(message: &str) {
    let line = format!(
        "{} {message}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
    );
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(log) = guard.as_mut() else {
        eprint!("{line}");
        return;
    };
    if rotate_if_full(&log.path, CAP)
        && let Ok(file) = open(&log.path)
    {
        if log.redirected {
            redirect_stderr(&file);
        }
        log.file = file;
    }
    let _ = log.file.write_all(line.as_bytes());
    if !log.redirected {
        eprint!("{line}");
    }
}

/// gpui's `log` records, warnings and up, plus its notification lines.
struct GpuiLog;
static GPUI_LOG: GpuiLog = GpuiLog;

impl log::Log for GpuiLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn || metadata.target().contains("notification")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            info(&format!(
                "{} {}: {}",
                record.level(),
                record.target(),
                record.args()
            ));
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_log_moves_aside() {
        let dir = std::env::temp_dir().join(format!("pigeon-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = path(&dir);
        std::fs::write(&log, vec![b'x'; 100]).unwrap();
        assert!(!rotate_if_full(&log, 100));
        std::fs::write(&log, vec![b'x'; 101]).unwrap();
        assert!(rotate_if_full(&log, 100));
        assert!(!log.exists());
        assert_eq!(
            std::fs::metadata(dir.join("pigeon.log.1")).unwrap().len(),
            101
        );
        // A missing log is not full.
        assert!(!rotate_if_full(&log, 100));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

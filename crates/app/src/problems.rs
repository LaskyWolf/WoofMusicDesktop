// Swift `DiagnosticsView`'s Recent Problems: the warnings and errors of the log.
// The log goes to stderr and to `woofmusic.log` next to the settings files.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

const CLEARED: &str = "-- Recent Problems cleared --";
const SHOWN: usize = 30;

pub fn log_path() -> Option<PathBuf> {
    IncrementalSyncService::default_defaults_path().map(|path| path.with_file_name("woofmusic.log"))
}

/// Writes every log line to stderr and to the log file.
struct Tee(Option<std::fs::File>);

impl Write for Tee {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(file) = &mut self.0 {
            let _ = file.write_all(bytes);
        }
        std::io::stderr().write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

/// `RUST_LOG` still decides what is logged; warnings and errors by default.
pub fn init() {
    let file = log_path().and_then(|path| {
        std::fs::create_dir_all(path.parent()?).ok()?;
        OpenOptions::new().create(true).append(true).open(path).ok()
    });
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Pipe(Box::new(Tee(file))))
        .init();
}

/// The last warnings and errors after the last Clear, newest last.
pub fn recent_from(log: &str) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    for line in log.lines() {
        if line == CLEARED {
            problems.clear();
        } else if line.contains(" WARN ") || line.contains(" ERROR ") {
            problems.push(line.to_owned());
        }
    }
    problems.split_off(problems.len().saturating_sub(SHOWN))
}

pub fn recent() -> Vec<String> {
    let log = log_path().and_then(|path| std::fs::read(path).ok());
    recent_from(&String::from_utf8_lossy(&log.unwrap_or_default()))
}

pub fn clear() {
    if let Some(mut file) =
        log_path().and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok())
    {
        let _ = writeln!(file, "{CLEARED}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_warnings_after_the_last_clear() {
        let log = format!(
            "[t INFO  a] fine\n[t WARN  a] old\n{CLEARED}\n[t INFO  a] fine\n[t ERROR a] new\n[t WARN  a] newer\n"
        );
        assert_eq!(recent_from(&log), ["[t ERROR a] new", "[t WARN  a] newer"]);
    }

    #[test]
    fn shows_only_the_last_thirty() {
        let log: String = (0..40).map(|i| format!("[t WARN  a] {i}\n")).collect();
        let shown = recent_from(&log);
        assert_eq!(shown.len(), SHOWN);
        assert_eq!(shown[0], "[t WARN  a] 10");
    }
}

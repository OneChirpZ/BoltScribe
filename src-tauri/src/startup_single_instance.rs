use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{plugin, Manager, Runtime};

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(2);
const NOTIFY_RETRY_INTERVAL: Duration = Duration::from_millis(20);

struct StartupInstanceLock {
    _file: File,
}

pub(crate) fn startup_race_guard_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    plugin::Builder::new("startup-single-instance-guard")
        .setup(|app, _api| {
            let identifier = &app.config().identifier;
            let lock_file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(lock_path(identifier))?;

            match lock_file.try_lock_exclusive() {
                Ok(()) => {
                    app.manage(StartupInstanceLock { _file: lock_file });
                    Ok(())
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    if let Err(err) = notify_existing_instance(identifier) {
                        eprintln!("failed to notify existing BoltScribe instance: {err}");
                    }
                    // Never continue into Tauri window creation without the
                    // startup lock, even if the primary instance is still
                    // initializing or is temporarily unable to accept events.
                    std::process::exit(0);
                }
                Err(err) => Err(err.into()),
            }
        })
        .build()
}

fn notify_existing_instance(identifier: &str) -> io::Result<()> {
    let socket = socket_path(identifier);
    let deadline = Instant::now() + NOTIFY_TIMEOUT;
    loop {
        match notify_socket(&socket) {
            Ok(()) => return Ok(()),
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) && Instant::now() < deadline =>
            {
                std::thread::sleep(NOTIFY_RETRY_INTERVAL);
            }
            Err(err) => return Err(err),
        }
    }
}

fn notify_socket(socket: &PathBuf) -> io::Result<()> {
    let stream = UnixStream::connect(socket)?;
    let mut writer = BufWriter::new(&stream);
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    writer.write_all(cwd.as_bytes())?;
    writer.write_all(b"\0\0")?;
    writer.write_all(std::env::args().collect::<Vec<_>>().join("\0").as_bytes())?;
    writer.flush()
}

fn lock_path(identifier: &str) -> PathBuf {
    runtime_directory().join(format!("{}_startup.lock", sanitized_identifier(identifier)))
}

fn socket_path(identifier: &str) -> PathBuf {
    runtime_directory().join(format!("{}_si.sock", sanitized_identifier(identifier)))
}

fn runtime_directory() -> PathBuf {
    // tauri-plugin-single-instance uses /tmp directly on macOS rather than
    // std::env::temp_dir(), which resolves to /var/folders/... for GUI apps.
    PathBuf::from("/tmp")
}

fn sanitized_identifier(identifier: &str) -> String {
    identifier.replace(['.', '-'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_paths_match_the_official_plugin_identifier_format() {
        assert_eq!(
            lock_path("cn.local.boltscribe"),
            PathBuf::from("/tmp/cn_local_boltscribe_startup.lock")
        );
        assert_eq!(
            socket_path("cn.local.boltscribe"),
            PathBuf::from("/tmp/cn_local_boltscribe_si.sock")
        );
    }
}

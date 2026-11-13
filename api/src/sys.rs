//! The NAS around the program: QTS settings, other programs (always run with an argv, never through a shell), the
//! install directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Runs `program` with `args` (no shell), returns its standard output when it exits with 0 within `limit`.
pub fn run(program: &Path, args: &[&str], limit: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(limit) {
        Ok(buf) => child.wait().ok()?.success().then_some(buf),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// A QTS setting through `/sbin/getcfg`; `None` when it is missing or empty.
pub fn getcfg(section: &str, key: &str, file: &str) -> Option<String> {
    let getcfg = Path::new("/sbin/getcfg");
    if !getcfg.is_file() {
        return None;
    }
    let out = run(getcfg, &[section, key, "-f", file], Duration::from_secs(5))?;
    let v = String::from_utf8_lossy(&out).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// The NAS time zone as an IANA name (`Asia/Taipei`); `UTC` when it cannot be read.
pub fn timezone() -> String {
    let tz = getcfg("System", "Time Zone", "/etc/config/uLinux.conf").unwrap_or_default();
    let ok = !tz.is_empty() && tz.len() <= 64 && tz.bytes().all(|c| c.is_ascii_alphanumeric() || b"/_+-".contains(&c));
    if ok { tz } else { "UTC".into() }
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname writes at most that many bytes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// The install directory: the parent of the directory holding the executable (`<Install_Path>/bin/hbmgr`).
pub fn install_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    Some(exe.parent()?.parent()?.to_path_buf())
}

//! Background jobs. The CGI takes the instance's lock (`flock`), writes the job file and starts a detached worker
//! (`hbmgr job-run <id>`) that inherits the locked file: the lock is never free between the answer and the worker.
//! The kernel releases it when the worker exits, however it exits, so a job file that still says `running` while its
//! lock is free (or held by another job) belongs to a dead worker and is reported as `failed`.

use crate::error::ApiError;
use crate::msg::Msg;
use crate::store::Store;
use crate::util::{self, now_rfc3339, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Jobs are kept this long.
const KEEP_SECS: i64 = 7 * 86_400;
/// The most of a job log returned by one poll.
const LOG_CHUNK: u64 = 256 * 1024;
/// The lock key of the update check (instances use their lower-case name).
pub const UPDATES_KEY: &str = "_updates";
const LOCK_FD_ENV: &str = "HBMGR_LOCK_FD";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Step {
    pub key: String,
    pub label: Msg,
    pub status: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Progress {
    pub percent: Option<u8>,
    pub detail: Option<Msg>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(from = "JobErrorRepr")]
pub struct JobError {
    pub code: String,
    /// The English sentence, for logs and the command line.
    pub message: String,
    pub msg: Msg,
}

/// A stored job error; one written by an earlier version has only `code` and `message`.
#[derive(Deserialize)]
struct JobErrorRepr {
    code: String,
    #[serde(default)]
    message: String,
    msg: Option<Msg>,
}

impl From<JobErrorRepr> for JobError {
    fn from(r: JobErrorRepr) -> JobError {
        let msg = r.msg.unwrap_or_else(|| Msg { key: r.message.clone(), args: Default::default() });
        let message = if r.message.is_empty() { msg.english() } else { r.message };
        JobError { code: r.code, message, msg }
    }
}

impl JobError {
    pub fn new(code: &str, msg: Msg) -> Self {
        JobError { code: code.into(), message: msg.english(), msg }
    }
}

/// A job as the API shows it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub instance: Option<String>,
    pub status: String,
    pub steps: Vec<Step>,
    pub progress: Progress,
    pub result: Value,
    pub error: Option<JobError>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// A job file: the job plus what the worker needs.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JobFile {
    #[serde(flatten)]
    pub job: Job,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub lock: String,
}

/// The steps of each kind of job, in order.
pub fn steps_of(kind: &str) -> Vec<Step> {
    let list: Vec<(&str, Msg)> = match kind {
        "create" | "apply" => vec![
            ("pull", msg!("Download the image")),
            ("stop", msg!("Stop the old container")),
            ("create", msg!("Create the new container")),
            ("start", msg!("Start the container")),
            ("verify", msg!("Make sure it keeps running")),
        ],
        "upgrade" => vec![
            ("pull", msg!("Download the image")),
            ("stop", msg!("Stop the old container")),
            ("create", msg!("Create the new container")),
            ("start", msg!("Start the container")),
            ("verify", msg!("Make sure it keeps running")),
            ("cleanup", msg!("Remove unused images")),
        ],
        "rollback" => vec![
            ("stop", msg!("Stop the current container")),
            ("restore", msg!("Switch back to the backup container")),
            ("start", msg!("Start the container")),
            ("verify", msg!("Make sure it keeps running")),
        ],
        "remove" => vec![
            ("stop", msg!("Stop the container")),
            ("remove", msg!("Remove the container")),
            ("backup", msg!("Remove the backup container")),
        ],
        "check_updates" => vec![("check", msg!("Check for image updates"))],
        _ => Vec::new(),
    };
    list.into_iter().map(|(k, label)| Step { key: k.to_string(), label, status: "pending".into() }).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Locks

/// An exclusive `flock` on `run/locks/<key>.lock`, released when dropped (or when the process dies).
#[derive(Debug)]
pub struct Lock {
    file: File,
    pub key: String,
}

impl Lock {
    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Records who holds the lock (a job id, or nothing for a short synchronous operation).
    pub fn set_holder(&mut self, holder: &str) {
        let _ = self.file.set_len(0);
        let _ = self.file.seek(SeekFrom::Start(0));
        let _ = self.file.write_all(holder.as_bytes());
        let _ = self.file.flush();
    }
}

fn lock_path(store: &Store, key: &str) -> PathBuf {
    store.locks_dir().join(format!("{key}.lock"))
}

/// The lock key of an instance: its name in lower case, so names that differ only in case exclude each other.
pub fn instance_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn open_lock(store: &Store, key: &str) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(lock_path(store, key))
}

fn flock_nb(f: &File) -> bool {
    // SAFETY: flock on a valid descriptor owned by `f`.
    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// Takes the lock without waiting for a holder, but retries for `patience` so that a reader that is only testing the
/// lock for a moment is not mistaken for a running job.
pub fn try_lock(store: &Store, key: &str, patience: Duration) -> std::io::Result<Option<Lock>> {
    let file = open_lock(store, key)?;
    let until = Instant::now() + patience;
    loop {
        if flock_nb(&file) {
            return Ok(Some(Lock { file, key: key.to_string() }));
        }
        if Instant::now() >= until {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Takes an instance's lock for a request, or answers 409 `busy` with the running job.
pub fn lock_or_busy(store: &Store, key: &str) -> Result<Lock, ApiError> {
    match try_lock(store, key, Duration::from_millis(400))? {
        Some(mut l) => {
            l.set_holder("");
            Ok(l)
        }
        None => Err(ApiError::busy(holder(store, key).as_deref())),
    }
}

/// The job holding a lock, if it is held by a job.
pub fn holder(store: &Store, key: &str) -> Option<String> {
    let mut f = open_lock(store, key).ok()?;
    if flock_nb(&f) {
        return None; // free (dropping `f` releases it)
    }
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

// ---------------------------------------------------------------------------------------------------------------
// Job files

fn job_path(store: &Store, id: &str) -> PathBuf {
    store.jobs_dir().join(format!("{id}.json"))
}

fn log_path(store: &Store, id: &str) -> PathBuf {
    store.jobs_dir().join(format!("{id}.log"))
}

fn valid_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
}

pub fn load(store: &Store, id: &str) -> Option<JobFile> {
    if !valid_id(id) {
        return None;
    }
    serde_json::from_slice(&fs::read(job_path(store, id)).ok()?).ok()
}

fn save(store: &Store, jf: &JobFile) -> std::io::Result<()> {
    write_atomic(&job_path(store, &jf.job.id), &serde_json::to_vec_pretty(jf).unwrap_or_default(), 0o600)
}

/// A job file, with a running job whose worker is gone marked as failed.
pub fn load_checked(store: &Store, id: &str) -> Option<JobFile> {
    let jf = load(store, id)?;
    if jf.job.status != "running" || holder(store, &jf.lock).as_deref() == Some(id) {
        return Some(jf);
    }
    // The lock is free or another job has it: read again (the worker may just have finished) and close it.
    let mut jf = load(store, id)?;
    if jf.job.status == "running" {
        jf.job.status = "failed".into();
        jf.job.error = Some(JobError::new("internal", msg!("The background task stopped unexpectedly")));
        jf.job.finished_at = Some(now_rfc3339());
        for s in jf.job.steps.iter_mut().filter(|s| s.status == "running") {
            s.status = "failed".into();
        }
        let _ = save(store, &jf);
    }
    Some(jf)
}

/// `GET /jobs/{id}`: the job and its log from `offset`.
pub fn view(store: &Store, id: &str, offset: u64) -> Option<Value> {
    let jf = load_checked(store, id)?;
    let mut v = serde_json::to_value(&jf.job).ok()?;
    let (log, next) = read_log(store, id, offset);
    v["log"] = json!(log);
    v["next_offset"] = json!(next);
    Some(v)
}

/// The log text from `offset` (at most `LOG_CHUNK` bytes, cut at a character boundary) and the next offset.
fn read_log(store: &Store, id: &str, offset: u64) -> (String, u64) {
    let Ok(mut f) = File::open(log_path(store, id)) else { return (String::new(), offset) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let offset = offset.min(len);
    if f.seek(SeekFrom::Start(offset)).is_err() {
        return (String::new(), offset);
    }
    let mut buf = Vec::new();
    let _ = f.take(LOG_CHUNK).read_to_end(&mut buf);
    let cut = match std::str::from_utf8(&buf) {
        Ok(_) => buf.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => buf.len(),
    };
    buf.truncate(cut);
    let next = offset + buf.len() as u64;
    (String::from_utf8_lossy(&buf).into_owned(), next)
}

/// `GET /jobs`: running jobs, or the last 50.
pub fn list(store: &Store, active_only: bool) -> Vec<Value> {
    let mut ids: Vec<String> = fs::read_dir(store.jobs_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string))
        .filter(|id| valid_id(id))
        .collect();
    ids.sort_by(|a, b| b.cmp(a));
    let mut out = Vec::new();
    for id in ids {
        let Some(jf) = load_checked(store, &id) else { continue };
        if active_only && jf.job.status != "running" {
            continue;
        }
        out.push(serde_json::to_value(&jf.job).unwrap_or(Value::Null));
        if !active_only && out.len() >= 50 {
            break;
        }
    }
    out
}

/// Removes finished jobs older than a week.
fn prune(store: &Store) {
    let now = util::now_secs();
    for e in fs::read_dir(store.jobs_dir()).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".json") else { continue };
        let Ok(meta) = e.metadata() else { continue };
        if now - meta.mtime() < KEEP_SECS {
            continue;
        }
        if load(store, id).is_some_and(|j| j.job.status == "running") {
            continue;
        }
        let _ = fs::remove_file(e.path());
        let _ = fs::remove_file(log_path(store, id));
    }
}

fn new_id() -> String {
    format!("{}-{}", util::compact_stamp(util::now_secs()), util::hex(&util::random_bytes(3)))
}

/// Starts a job: writes its file and starts the detached worker, which takes over `lock`. Returns the job id at once.
pub fn launch(store: &Store, mut lock: Lock, kind: &str, instance: Option<&str>, params: Value) -> Result<String, ApiError> {
    prune(store);
    let id = new_id();
    // The holder is written before the job file, so a reader never sees the new job with someone else's lock.
    lock.set_holder(&id);
    let jf = JobFile {
        job: Job {
            id: id.clone(),
            kind: kind.into(),
            instance: instance.map(str::to_string),
            status: "running".into(),
            steps: steps_of(kind),
            progress: Progress::default(),
            result: Value::Null,
            error: None,
            started_at: now_rfc3339(),
            finished_at: None,
        },
        params,
        lock: lock.key.clone(),
    };
    save(store, &jf)?;
    if let Err(e) = spawn_worker(store, &lock, &id) {
        let mut jf = jf;
        jf.job.status = "failed".into();
        jf.job.error = Some(JobError::new("internal", msg!("The background task could not be started")));
        jf.job.finished_at = Some(now_rfc3339());
        let _ = save(store, &jf);
        eprintln!("hbmgr: cannot start the worker for {id}: {e}");
        return Err(ApiError::internal(msg!("The background task could not be started")));
    }
    Ok(id)
}

/// Re-executes this program as `job-run <id>` in its own session, with standard input from /dev/null, output to
/// `run/worker.log`, a clean environment and no descriptor of ours except the lock.
fn spawn_worker(store: &Store, lock: &Lock, id: &str) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let out = OpenOptions::new().create(true).append(true).mode(0o600).open(store.run_dir().join("worker.log"))?;
    let err = out.try_clone()?;
    let lock_fd = lock.fd();
    let mut cmd = Command::new(exe);
    cmd.arg("job-run")
        .arg(id)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HBMGR_DATA", &store.root)
        .env(LOCK_FD_ENV, lock_fd.to_string());
    for k in ["HBMGR_DOCKER_SOCK", "HBMGR_CS_DOCKER_SOCK", "HBMGR_QNETWORK_TOOL"] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    // SAFETY: only async-signal-safe calls (setsid, fcntl, sysconf) between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            // Every descriptor above stderr closes on exec (whatever Apache left open), except the lock.
            let max = match libc::sysconf(libc::_SC_OPEN_MAX) {
                n if n > 0 => n.min(65_536) as RawFd,
                _ => 4096,
            };
            for fd in 3..max {
                if fd != lock_fd {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            if libc::fcntl(lock_fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // The child is not waited for: it outlives this process and is adopted by init.
    cmd.spawn().map(drop)
}

// ---------------------------------------------------------------------------------------------------------------
// Worker side

/// The worker's handle on its job: updates the job file and appends to the log.
pub struct Ctx<'a> {
    pub store: &'a Store,
    pub file: JobFile,
    log: Option<File>,
    last_save: Instant,
    /// What this worker has logged, to log an error once.
    logged: Vec<Msg>,
}

impl<'a> Ctx<'a> {
    pub fn new(store: &'a Store, file: JobFile) -> Self {
        let log = OpenOptions::new().create(true).append(true).mode(0o600).open(log_path(store, &file.job.id)).ok();
        Ctx { store, file, log, last_save: Instant::now(), logged: Vec::new() }
    }

    pub fn save(&mut self) {
        let _ = save(self.store, &self.file);
        self.last_save = Instant::now();
    }

    /// Appends `HH:MM:SS<TAB><Msg as compact JSON>` to the job log.
    pub fn log(&mut self, msg: Msg) {
        self.write_line(&msg.compact());
        self.logged.push(msg);
    }

    pub fn has_logged(&self, msg: &Msg) -> bool {
        self.logged.contains(msg)
    }

    /// Appends text that is not translated (a container's own output) as `HH:MM:SS<TAB>|<text>`, one line each.
    pub fn raw(&mut self, text: &str) {
        for line in text.lines() {
            self.write_line(&format!("|{}", line.replace('\t', " ")));
        }
    }

    fn write_line(&mut self, body: &str) {
        let stamp = util::rfc3339(util::now_secs());
        let time = &stamp[11..19];
        if let Some(f) = self.log.as_mut() {
            let _ = writeln!(f, "{time}\t{body}");
        }
    }

    pub fn step(&mut self, key: &str, status: &str) {
        if let Some(s) = self.file.job.steps.iter_mut().find(|s| s.key == key) {
            s.status = status.into();
        }
        if status == "running" {
            self.file.job.progress = Progress::default();
        }
        self.save();
    }

    /// Progress of the running step; written at most a few times a second.
    pub fn progress(&mut self, percent: Option<u8>, detail: Option<Msg>) {
        self.file.job.progress = Progress { percent, detail };
        if self.last_save.elapsed() >= Duration::from_millis(400) {
            self.save();
        }
    }

    /// Ends the job.
    pub fn finish(&mut self, status: &str, result: Value, error: Option<JobError>) {
        for s in self.file.job.steps.iter_mut() {
            if s.status == "running" {
                s.status = if status == "succeeded" { "done".into() } else { "failed".into() };
            }
            if s.status == "pending" {
                s.status = "skipped".into();
            }
        }
        self.file.job.status = status.into();
        self.file.job.result = result;
        self.file.job.error = error;
        self.file.job.finished_at = Some(now_rfc3339());
        self.file.job.progress = Progress::default();
        self.save();
    }
}

/// The worker's lock: the descriptor inherited from the CGI, checked to be that lock file; or, when the worker was
/// started by hand, the lock taken now.
pub fn worker_lock(store: &Store, key: &str) -> Option<Lock> {
    if let Some(fd) = std::env::var(LOCK_FD_ENV).ok().and_then(|v| v.parse::<RawFd>().ok()) {
        // SAFETY: the descriptor was passed to us for this purpose; it is checked before use and owned from here on.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } == 0 {
            let meta = fs::metadata(lock_path(store, key)).ok()?;
            if st.st_dev as u64 == meta.dev() && st.st_ino as u64 == meta.ino() {
                let file = unsafe { <File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
                if flock_nb(&file) {
                    return Some(Lock { file, key: key.to_string() });
                }
                return None;
            }
        }
        return None;
    }
    try_lock(store, key, Duration::ZERO).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;

    #[test]
    fn ids_and_steps() {
        let id = new_id();
        assert!(valid_id(&id), "{id}");
        assert!(!valid_id("../../etc/passwd"));
        let keys = |kind: &str| steps_of(kind).iter().map(|s| s.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys("apply"), ["pull", "stop", "create", "start", "verify"]);
        assert_eq!(keys("upgrade"), ["pull", "stop", "create", "start", "verify", "cleanup"]);
        assert!(steps_of("upgrade").iter().all(|s| s.status == "pending"));
    }

    #[test]
    fn a_lock_is_exclusive_and_names_its_holder() {
        let s = temp_store("lock");
        let mut a = try_lock(&s, "hb", Duration::ZERO).unwrap().unwrap();
        a.set_holder("20261112-112000-a1b2c3");
        assert!(try_lock(&s, "hb", Duration::from_millis(60)).unwrap().is_none());
        assert_eq!(holder(&s, "hb").as_deref(), Some("20261112-112000-a1b2c3"));
        assert_eq!(lock_or_busy(&s, "hb").unwrap_err().code, "busy");
        drop(a);
        assert_eq!(holder(&s, "hb"), None);
        assert!(try_lock(&s, "hb", Duration::ZERO).unwrap().is_some());
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn a_job_without_its_worker_is_failed() {
        let s = temp_store("dead");
        let id = "20261112-112000-a1b2c3".to_string();
        let jf = JobFile {
            job: Job {
                id: id.clone(),
                kind: "apply".into(),
                instance: Some("hb".into()),
                status: "running".into(),
                steps: steps_of("apply"),
                progress: Progress::default(),
                result: Value::Null,
                error: None,
                started_at: now_rfc3339(),
                finished_at: None,
            },
            params: Value::Null,
            lock: "hb".into(),
        };
        save(&s, &jf).unwrap();
        // While the lock names this job, it is running.
        let mut l = try_lock(&s, "hb", Duration::ZERO).unwrap().unwrap();
        l.set_holder(&id);
        assert_eq!(load_checked(&s, &id).unwrap().job.status, "running");
        assert_eq!(list(&s, true).len(), 1);
        // Another job holds the lock: this one is dead.
        l.set_holder("20261112-112100-ffffff");
        assert_eq!(load_checked(&s, &id).unwrap().job.status, "failed");
        drop(l);
        assert!(list(&s, true).is_empty());
        assert_eq!(list(&s, false).len(), 1);
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn logs_are_read_from_an_offset_at_character_boundaries() {
        let s = temp_store("log");
        let id = "20261112-112000-a1b2c3";
        fs::write(log_path(&s, id), "下載映像\nok\n").unwrap();
        let (all, next) = read_log(&s, id, 0);
        assert_eq!(all, "下載映像\nok\n", "multi-byte text from a container is kept whole");
        assert_eq!(next, all.len() as u64);
        let (none, same) = read_log(&s, id, next);
        assert_eq!((none.as_str(), same), ("", next));
        let (rest, _) = read_log(&s, id, 13);
        assert_eq!(rest, "ok\n");
        assert_eq!(read_log(&s, id, 10_000).1, next, "an offset past the end is clamped");
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn job_files_of_the_earlier_version_still_load() {
        let s = temp_store("oldjob");
        let id = "20261112-112000-a1b2c3";
        let old = r#"{"id":"20261112-112000-a1b2c3","kind":"apply","instance":"hb","status":"failed",
            "steps":[{"key":"pull","label":"old label","status":"done"}],
            "progress":{"percent":null,"detail":"old detail"},"result":null,
            "error":{"code":"docker_error","message":"old text"},
            "started_at":"2026-11-12T11:20:00Z","finished_at":"2026-11-12T11:21:00Z","params":null,"lock":"hb"}"#;
        fs::write(job_path(&s, id), old).unwrap();
        let jf = load(&s, id).expect("an old job file loads");
        let e = jf.job.error.unwrap();
        assert_eq!((e.code.as_str(), e.message.as_str(), e.msg.key.as_str()), ("docker_error", "old text", "old text"));
        assert_eq!(jf.job.steps[0].label.key, "old label");
        assert_eq!(jf.job.progress.detail.unwrap().key, "old detail");
        assert_eq!(list(&s, false).len(), 1);
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn log_lines_are_messages_or_raw_text() {
        let s = temp_store("logfmt");
        let jf = JobFile {
            job: Job {
                id: "20261112-112000-a1b2c3".into(),
                kind: "apply".into(),
                instance: None,
                status: "running".into(),
                steps: steps_of("apply"),
                progress: Progress::default(),
                result: Value::Null,
                error: None,
                started_at: now_rfc3339(),
                finished_at: None,
            },
            params: Value::Null,
            lock: "x".into(),
        };
        let mut c = Ctx::new(&s, jf);
        c.log(msg!("Using the network {name}", name = "qnet-static-bond0-3f9a2c"));
        c.raw("[11/12/2026] [Homebridge] ready\tnow\nsecond line");
        let (text, _) = read_log(&s, "20261112-112000-a1b2c3", 0);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        let (time, body) = lines[0].split_once('\t').unwrap();
        assert!(time.len() == 8 && time.as_bytes()[2] == b':', "{time}");
        assert_eq!(body, r#"{"key":"Using the network {name}","args":{"name":"qnet-static-bond0-3f9a2c"}}"#);
        assert_eq!(lines[1].split_once('\t').unwrap().1, "|[11/12/2026] [Homebridge] ready now");
        assert_eq!(lines[2].split_once('\t').unwrap().1, "|second line");
        assert!(c.has_logged(&msg!("Using the network {name}", name = "qnet-static-bond0-3f9a2c")));
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn finishing_closes_the_steps() {
        let s = temp_store("finish");
        let jf = JobFile {
            job: Job {
                id: "20261112-112000-a1b2c3".into(),
                kind: "apply".into(),
                instance: None,
                status: "running".into(),
                steps: steps_of("apply"),
                progress: Progress::default(),
                result: Value::Null,
                error: None,
                started_at: now_rfc3339(),
                finished_at: None,
            },
            params: Value::Null,
            lock: "x".into(),
        };
        let mut c = Ctx::new(&s, jf);
        c.step("pull", "done");
        c.step("stop", "running");
        c.finish("failed", Value::Null, Some(JobError::new("docker_error", msg!("Docker reported an error: {error}", error = "x"))));
        assert_eq!(c.file.job.error.as_ref().unwrap().message, "Docker reported an error: x");
        let st: Vec<&str> = c.file.job.steps.iter().map(|s| s.status.as_str()).collect();
        assert_eq!(st, ["done", "failed", "skipped", "skipped", "skipped"]);
        assert!(c.file.job.finished_at.is_some());
        let _ = fs::remove_dir_all(&s.root);
    }
}

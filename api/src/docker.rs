//! The Docker Engine API over its Unix socket. Requests go to a versioned path: 1.44 is the first version that takes a
//! MAC address in the endpoint settings when a container is created (older versions drop it silently).
//!
//! Two daemons are involved. New containers, images and networks live on Container Station's `system-docker`, which
//! Container Station's UI does not list (so nobody deletes a Homebridge there by mistake). Containers that already run
//! on Container Station's own daemon (the one its UI shows) are read and handled where they are, so they can be taken
//! over: the container list holds both daemons, and a call on a container id goes to the daemon that has it.

use crate::http::{self, Body, Head};
use crate::util::enc;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const API_PREFIX: &str = "/v1.44";
/// Where this package's containers live (`HBMGR_DOCKER_SOCK`).
const DEFAULT_SOCK: &str = "/var/run/system-docker.sock";
/// Container Station's own daemon, where existing containers are taken over from (`HBMGR_CS_DOCKER_SOCK`; empty: none).
const DEFAULT_CS_SOCK: &str = "/var/run/docker.sock";
/// The most of an answer that is read into memory.
const MAX_BODY: usize = 64 * 1024 * 1024;
/// Timeout of an ordinary call.
const CALL_TIME: Duration = Duration::from_secs(60);
/// Starting a container on a qnet network waits for its address (a DHCP lease can take half a minute).
const START_TIME: Duration = Duration::from_secs(180);
/// How long a stop may take: Docker waits `STOP_GRACE` before it kills the container.
pub const STOP_GRACE: u32 = 20;

#[derive(Debug, Clone, PartialEq)]
pub enum DockerError {
    /// The socket is missing or nobody listens: Container Station is not running.
    Unavailable(String),
    /// Docker answered with an error status.
    Api { status: u16, message: String },
    /// The connection broke or the answer made no sense.
    Broken(String),
}

impl DockerError {
    /// The message for the UI. Docker's own text is passed on as it is.
    pub fn msg(&self) -> crate::msg::Msg {
        match self {
            DockerError::Unavailable(_) => msg!("Cannot connect to Docker. Make sure Container Station is running."),
            DockerError::Api { message, .. } => msg!("Docker reported an error: {error}", error = message),
            DockerError::Broken(e) => msg!("The connection to Docker failed: {error}", error = e),
        }
    }

    /// The technical detail in English (what Docker said, or what went wrong with the connection).
    pub fn detail(&self) -> String {
        match self {
            DockerError::Unavailable(e) => format!("cannot connect to Docker: {e}"),
            DockerError::Api { message, .. } => message.clone(),
            DockerError::Broken(e) => format!("connection to Docker: {e}"),
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, DockerError::Api { status: 404, .. })
    }
}

impl std::fmt::Display for DockerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DockerError::Unavailable(e) => write!(f, "docker unavailable: {e}"),
            DockerError::Api { status, message } => write!(f, "docker {status}: {message}"),
            DockerError::Broken(e) => write!(f, "docker connection: {e}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, DockerError>;

pub struct Docker {
    sock: PathBuf,
    /// Container Station's own daemon, when it is another one.
    cs: Option<Box<Docker>>,
}

/// A container in the list (`GET /containers/json`).
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub image: String,
    pub image_id: String,
    pub state: String,
    pub labels: serde_json::Map<String, Value>,
    pub created: i64,
    /// It lives on Container Station's own daemon (Container Station lists it), not on system-docker.
    pub in_cs: bool,
    pub raw: Value,
}

impl Listed {
    pub fn label(&self, k: &str) -> Option<&str> {
        self.labels.get(k).and_then(Value::as_str)
    }

    /// The container's networks as (network name, configured IPv4, current IPv4, MAC).
    pub fn endpoints(&self) -> Vec<(String, String, String, String)> {
        endpoints(&self.raw["NetworkSettings"]["Networks"])
    }
}

/// (network, configured IPv4, current IPv4, MAC) of each endpoint in a `Networks` object.
pub fn endpoints(networks: &Value) -> Vec<(String, String, String, String)> {
    let Some(m) = networks.as_object() else { return Vec::new() };
    m.iter()
        .map(|(name, ep)| {
            let s = |v: &Value| v.as_str().unwrap_or("").to_string();
            (name.clone(), s(&ep["IPAMConfig"]["IPv4Address"]), s(&ep["IPAddress"]), s(&ep["MacAddress"]).to_ascii_lowercase())
        })
        .collect()
}

impl Docker {
    pub fn from_env() -> Self {
        let sock = std::env::var_os("HBMGR_DOCKER_SOCK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DEFAULT_SOCK));
        let cs = match std::env::var_os("HBMGR_CS_DOCKER_SOCK") {
            Some(v) if v.is_empty() => None,
            Some(v) => Some(PathBuf::from(v)),
            None => Some(PathBuf::from(DEFAULT_CS_SOCK)),
        };
        let cs = cs.filter(|c| *c != sock).map(|c| Box::new(Docker { sock: c, cs: None }));
        Docker { sock, cs }
    }

    /// Runs `f` on the daemon that has container `id`: system-docker first; on a 404 there, Container Station's own
    /// daemon when system-docker does not have the container (a 404 can also mean that something else, such as a
    /// network, is missing). When both fail, or Container Station's daemon is not there, system-docker's error stands.
    fn on_container<T>(&self, id: &str, f: impl Fn(&Docker) -> Result<T>) -> Result<T> {
        match f(self) {
            Err(e) if e.is_not_found() => {
                let Some(cs) = self.cs.as_deref() else { return Err(e) };
                if matches!(self.get_opt(&format!("/containers/{}/json", enc(id))), Ok(Some(_))) {
                    return Err(e);
                }
                match f(cs) {
                    Err(DockerError::Unavailable(_)) => Err(e),
                    r => r,
                }
            }
            r => r,
        }
    }

    /// Container Station's own daemon's answer, with "not running" as `None`.
    fn cs_opt<T>(&self, f: impl Fn(&Docker) -> Result<T>) -> Result<Option<T>> {
        match self.cs.as_deref().map(f) {
            None | Some(Err(DockerError::Unavailable(_))) => Ok(None),
            Some(r) => r.map(Some),
        }
    }

    fn connect(&self, timeout: Duration) -> Result<UnixStream> {
        let s = UnixStream::connect(&self.sock).map_err(|e| DockerError::Unavailable(e.to_string()))?;
        s.set_read_timeout(Some(timeout)).map_err(|e| DockerError::Broken(e.to_string()))?;
        s.set_write_timeout(Some(CALL_TIME)).map_err(|e| DockerError::Broken(e.to_string()))?;
        Ok(s)
    }

    /// Sends a request and returns the head and the body reader. Error statuses are returned, not turned into errors.
    pub fn send(&self, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<(Head, Body<BufReader<UnixStream>>)> {
        let mut s = self.connect(timeout)?;
        let payload = body.map(|b| serde_json::to_vec(b).unwrap_or_default());
        let headers: &[(&str, &str)] = if payload.is_some() { &[("Content-Type", "application/json")] } else { &[] };
        let req = http::request(method, &format!("{API_PREFIX}{path}"), "docker", headers, payload.as_deref(), false);
        s.write_all(&req).map_err(|e| DockerError::Broken(e.to_string()))?;
        let mut r = BufReader::new(s);
        let head = http::read_head(&mut r).map_err(|e| DockerError::Broken(e.to_string()))?;
        let body = Body::new(&head, r);
        Ok((head, body))
    }

    /// A call whose answer is read whole: JSON when there is any, `Null` for an empty body.
    pub fn call_with(&self, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
        let (head, body) = self.send(method, path, body, timeout)?;
        let bytes = body.read_all(MAX_BODY).map_err(|e| DockerError::Broken(e.to_string()))?;
        if head.status >= 400 {
            return Err(api_error(head.status, &bytes));
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|e| DockerError::Broken(format!("invalid JSON: {e}")))
    }

    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        self.call_with(method, path, body, CALL_TIME)
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.call("GET", path, None)
    }

    /// `GET`, with 404 as `None`.
    pub fn get_opt(&self, path: &str) -> Result<Option<Value>> {
        match self.get(path) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn version(&self) -> Result<Value> {
        self.get("/version")
    }

    fn list_here(&self) -> Result<Vec<Listed>> {
        let v = self.get("/containers/json?all=1")?;
        Ok(v.as_array().map(|a| a.iter().filter_map(listed).collect()).unwrap_or_default())
    }

    /// Every container of both daemons, system-docker's first. Container Station's own daemon not running only
    /// leaves its containers out.
    pub fn list_containers(&self) -> Result<Vec<Listed>> {
        let mut list = self.list_here()?;
        if let Some(more) = self.cs_opt(Docker::list_here)? {
            list.extend(more.into_iter().map(|c| Listed { in_cs: true, ..c }));
        }
        Ok(list)
    }

    /// The container whose name is exactly `name` (Docker's own lookup also matches id prefixes, and its name filter
    /// matches substrings), on system-docker before Container Station's own daemon.
    pub fn find(&self, name: &str) -> Result<Option<Listed>> {
        Ok(self.list_containers()?.into_iter().find(|c| c.name == name))
    }

    pub fn inspect(&self, id: &str) -> Result<Option<Value>> {
        let path = format!("/containers/{}/json", enc(id));
        match self.get_opt(&path)? {
            Some(v) => Ok(Some(v)),
            None => Ok(self.cs_opt(|d| d.get_opt(&path))?.flatten()),
        }
    }

    /// An image on system-docker, where containers are created.
    pub fn inspect_image(&self, image: &str) -> Result<Option<Value>> {
        self.get_opt(&format!("/images/{image}/json"))
    }

    /// An image on system-docker, else on Container Station's own daemon (the image of a container still there).
    pub fn inspect_image_anywhere(&self, image: &str) -> Result<Option<Value>> {
        match self.inspect_image(image)? {
            Some(v) => Ok(Some(v)),
            None => Ok(self.cs_opt(|d| d.inspect_image(image))?.flatten()),
        }
    }

    /// The images on system-docker (`GET /images/json`; its `Containers` count is not filled in).
    pub fn images(&self) -> Result<Vec<Value>> {
        Ok(self.get("/images/json")?.as_array().cloned().unwrap_or_default())
    }

    /// Removes an image of system-docker by id. Never forced: Docker refuses an image that a container uses.
    pub fn remove_image(&self, id: &str) -> Result<()> {
        self.call("DELETE", &format!("/images/{id}"), None).map(drop)
    }

    pub fn create(&self, name: &str, body: &Value) -> Result<String> {
        let v = self.call("POST", &format!("/containers/create?name={}", enc(name)), Some(body))?;
        v["Id"].as_str().map(str::to_string).ok_or_else(|| DockerError::Broken("no id in the answer".into()))
    }

    pub fn start(&self, id: &str) -> Result<()> {
        let path = format!("/containers/{}/start", enc(id));
        self.on_container(id, |d| d.call_with("POST", &path, None, START_TIME).map(drop))
    }

    pub fn stop(&self, id: &str) -> Result<()> {
        let path = format!("/containers/{}/stop?t={STOP_GRACE}", enc(id));
        self.on_container(id, |d| d.call_with("POST", &path, None, Duration::from_secs(u64::from(STOP_GRACE) + 30)).map(drop))
    }

    pub fn restart(&self, id: &str) -> Result<()> {
        let path = format!("/containers/{}/restart?t={STOP_GRACE}", enc(id));
        self.on_container(id, |d| d.call_with("POST", &path, None, START_TIME).map(drop))
    }

    pub fn set_restart(&self, id: &str, name: &str, max_retry: i64) -> Result<()> {
        let body = json!({ "RestartPolicy": { "Name": name, "MaximumRetryCount": max_retry } });
        let path = format!("/containers/{}/update", enc(id));
        self.on_container(id, |d| d.call("POST", &path, Some(&body)).map(drop))
    }

    pub fn rename(&self, id: &str, to: &str) -> Result<()> {
        let path = format!("/containers/{}/rename?name={}", enc(id), enc(to));
        self.on_container(id, |d| d.call("POST", &path, None).map(drop))
    }

    /// Removes a container (stopping it if needed). Named volumes and bind-mounted folders are never touched.
    pub fn remove(&self, id: &str) -> Result<()> {
        let path = format!("/containers/{}?force=1", enc(id));
        self.on_container(id, |d| d.call("DELETE", &path, None).map(drop))
    }

    /// system-docker's networks, where containers are created.
    pub fn networks(&self) -> Result<Vec<Value>> {
        Ok(self.get("/networks")?.as_array().cloned().unwrap_or_default())
    }

    /// The networks of both daemons, to read what a container of either one is connected to.
    pub fn all_networks(&self) -> Result<Vec<Value>> {
        let mut nets = self.networks()?;
        nets.extend(self.cs_opt(Docker::networks)?.unwrap_or_default());
        Ok(nets)
    }

    pub fn create_network(&self, body: &Value) -> Result<()> {
        self.call("POST", "/networks/create", Some(body)).map(drop)
    }

    /// The registry's manifest digest for `image` (Docker asks the registry; nothing is pulled).
    pub fn remote_digest(&self, image: &str) -> Result<String> {
        let v = self.call_with("GET", &format!("/distribution/{image}/json"), None, Duration::from_secs(20))?;
        v["Descriptor"]["digest"].as_str().map(str::to_string).ok_or_else(|| DockerError::Broken("no digest in the answer".into()))
    }

    /// Pulls `repo:tag`, calling `progress` with every JSON message of the stream. A failure reported inside the
    /// stream (Docker answers 200 and then sends `{"error": …}`) is an error.
    pub fn pull(&self, image: &str, mut progress: impl FnMut(&Value)) -> Result<()> {
        let (repo, tag) = crate::spec::split_image(image);
        let path = format!("/images/create?fromImage={}&tag={}", enc(repo), enc(tag));
        let (head, body) = self.send("POST", &path, None, Duration::from_secs(300))?;
        if head.status >= 400 {
            let bytes = body.read_all(1 << 20).unwrap_or_default();
            return Err(api_error(head.status, &bytes));
        }
        let mut r = BufReader::new(body);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = r.read_until(b'\n', &mut line).map_err(|e| DockerError::Broken(e.to_string()))?;
            if n == 0 {
                return Ok(());
            }
            let Ok(v) = serde_json::from_slice::<Value>(&line) else { continue };
            if let Some(err) = v.get("error").and_then(Value::as_str) {
                let msg = v["errorDetail"]["message"].as_str().unwrap_or(err);
                return Err(DockerError::Api { status: 500, message: msg.trim().to_string() });
            }
            progress(&v);
        }
    }

    /// The last `tail` lines of a container's output with time stamps, already demultiplexed.
    pub fn logs(&self, id: &str, tail: usize, tty: bool) -> Result<Vec<u8>> {
        let path = format!("/containers/{}/logs?stdout=1&stderr=1&timestamps=1&tail={tail}", enc(id));
        let bytes = self.on_container(id, |d| {
            let (head, body) = d.send("GET", &path, None, CALL_TIME)?;
            let bytes = body.read_all(16 * 1024 * 1024).map_err(|e| DockerError::Broken(e.to_string()))?;
            if head.status >= 400 {
                return Err(api_error(head.status, &bytes));
            }
            Ok(bytes)
        })?;
        Ok(demux(&bytes, tty))
    }

    /// One stats sample with the previous one filled in (takes about a second).
    pub fn stats(&self, id: &str) -> Result<Value> {
        let path = format!("/containers/{}/stats?stream=0", enc(id));
        self.on_container(id, |d| d.call_with("GET", &path, None, Duration::from_secs(15)))
    }
}

fn listed(v: &Value) -> Option<Listed> {
    let name = v["Names"].as_array()?.iter().filter_map(Value::as_str).find_map(|n| n.strip_prefix('/')).filter(|n| !n.contains('/'))?;
    Some(Listed {
        id: v["Id"].as_str()?.to_string(),
        name: name.to_string(),
        image: v["Image"].as_str().unwrap_or("").to_string(),
        image_id: v["ImageID"].as_str().unwrap_or("").to_string(),
        state: v["State"].as_str().unwrap_or("").to_string(),
        labels: v["Labels"].as_object().cloned().unwrap_or_default(),
        created: v["Created"].as_i64().unwrap_or(0),
        in_cs: false,
        raw: v.clone(),
    })
}

fn api_error(status: u16, body: &[u8]) -> DockerError {
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().chars().take(500).collect());
    DockerError::Api { status, message: message.trim().to_string() }
}

/// Docker's log stream: raw when the container has a TTY, else frames of `[stream, 0, 0, 0, size (4 bytes, big
/// endian)]` followed by `size` bytes. Something that does not look like a frame is taken as raw text from there on.
pub fn demux(raw: &[u8], tty: bool) -> Vec<u8> {
    if tty {
        return raw.to_vec();
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut rest = raw;
    while !rest.is_empty() {
        if rest.len() < 8 || rest[0] > 2 || rest[1..4] != [0, 0, 0] {
            out.extend_from_slice(rest);
            break;
        }
        let size = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let end = (8 + size).min(rest.len());
        out.extend_from_slice(&rest[8..end]);
        rest = &rest[end..];
    }
    out
}

/// Log text as lines: ANSI escapes removed, Docker's time stamps shortened to whole seconds.
pub fn log_lines(text: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(text)
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = crate::util::strip_ansi(l);
            match l.split_once(' ') {
                Some((ts, msg)) if ts.len() >= 20 && ts.ends_with('Z') => match crate::util::docker_time(ts) {
                    Some(t) => format!("{t} {}", msg.trim_end_matches('\r')),
                    None => l,
                },
                _ => l,
            }
        })
        .collect()
}

/// CPU share (percent of one CPU times the number of CPUs, as `docker stats` shows it) and memory use from a
/// stats sample.
pub fn stats_summary(s: &Value) -> (Option<f64>, Option<u64>, Option<u64>) {
    let cpu = (|| {
        let total = s["cpu_stats"]["cpu_usage"]["total_usage"].as_f64()?;
        let pre_total = s["precpu_stats"]["cpu_usage"]["total_usage"].as_f64()?;
        let sys = s["cpu_stats"]["system_cpu_usage"].as_f64()?;
        let pre_sys = s["precpu_stats"]["system_cpu_usage"].as_f64()?;
        let cpus = s["cpu_stats"]["online_cpus"]
            .as_f64()
            .or_else(|| s["cpu_stats"]["cpu_usage"]["percpu_usage"].as_array().map(|a| a.len() as f64))
            .filter(|n| *n > 0.0)
            .unwrap_or(1.0);
        let (d_cpu, d_sys) = (total - pre_total, sys - pre_sys);
        if d_sys <= 0.0 || d_cpu < 0.0 {
            return Some(0.0);
        }
        Some((d_cpu / d_sys * cpus * 1000.0).round() / 10.0)
    })();
    let m = &s["memory_stats"];
    let mem = m["usage"].as_u64().map(|usage| {
        let cache = ["inactive_file", "total_inactive_file"].iter().find_map(|k| m["stats"][*k].as_u64()).unwrap_or(0);
        usage.saturating_sub(cache)
    });
    let limit = m["limit"].as_u64();
    (cpu, mem, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(stream: u8, text: &str) -> Vec<u8> {
        let mut f = vec![stream, 0, 0, 0];
        f.extend_from_slice(&(text.len() as u32).to_be_bytes());
        f.extend_from_slice(text.as_bytes());
        f
    }

    #[test]
    fn demultiplexes_frames() {
        let mut raw = frame(1, "2026-11-12T11:20:00.123Z out\n");
        raw.extend(frame(2, "2026-11-12T11:20:01.5Z err\n"));
        raw.extend(frame(1, ""));
        assert_eq!(demux(&raw, false), b"2026-11-12T11:20:00.123Z out\n2026-11-12T11:20:01.5Z err\n");
    }

    #[test]
    fn tty_output_is_raw() {
        let raw = b"\x01\x00\x00\x00\x00\x00\x00\x02hi";
        assert_eq!(demux(raw, true), raw.to_vec());
    }

    #[test]
    fn a_cut_frame_keeps_what_arrived() {
        let mut raw = frame(1, "hello\n");
        raw.truncate(10);
        assert_eq!(demux(&raw, false), b"he");
        assert_eq!(demux(b"no frames here\n", false), b"no frames here\n");
    }

    #[test]
    fn log_lines_are_cleaned() {
        let text = b"2026-11-12T11:20:00.123456789Z \x1b[37m[11/12/2026, 7:20:00 PM]\x1b[39m \x1b[36m[homebridge]\x1b[39m ready\r\n\
2026-11-12T11:20:01.000000000Z second\r\n\nno stamp\n";
        assert_eq!(
            log_lines(text),
            vec![
                "2026-11-12T11:20:00Z [11/12/2026, 7:20:00 PM] [homebridge] ready".to_string(),
                "2026-11-12T11:20:01Z second".to_string(),
                "no stamp".to_string(),
            ]
        );
    }

    #[test]
    fn summarises_stats() {
        let s = serde_json::json!({
            "cpu_stats": { "cpu_usage": { "total_usage": 2_000_000u64 }, "system_cpu_usage": 100_000_000u64, "online_cpus": 4 },
            "precpu_stats": { "cpu_usage": { "total_usage": 1_000_000u64 }, "system_cpu_usage": 50_000_000u64 },
            "memory_stats": { "usage": 200_000_000u64, "limit": 33_440_000_000u64, "stats": { "inactive_file": 16_499_200u64 } }
        });
        assert_eq!(stats_summary(&s), (Some(8.0), Some(183_500_800), Some(33_440_000_000)));
        assert_eq!(stats_summary(&serde_json::json!({})), (None, None, None));
    }

    #[test]
    fn lists_names_exactly() {
        let v = serde_json::json!({ "Id": "abc", "Names": ["/homebridge-1F"], "Image": "oznu/homebridge:latest", "State": "running",
            "Labels": { "hbmgr.instance": "homebridge-1F" }, "NetworkSettings": { "Networks": { "qnet-static-bond0-3f9a2c": {
                "IPAMConfig": { "IPv4Address": "192.168.10.7" }, "IPAddress": "192.168.10.7", "MacAddress": "02:42:5E:7A:21:0C" } } } });
        let l = listed(&v).unwrap();
        assert_eq!(l.name, "homebridge-1F");
        assert_eq!(l.label("hbmgr.instance"), Some("homebridge-1F"));
        assert_eq!(
            l.endpoints(),
            vec![(
                "qnet-static-bond0-3f9a2c".to_string(),
                "192.168.10.7".to_string(),
                "192.168.10.7".to_string(),
                "02:42:5e:7a:21:0c".to_string()
            )]
        );
    }

    #[test]
    fn error_messages_come_from_docker() {
        assert_eq!(
            api_error(404, br#"{"message":"No such container: x"}"#),
            DockerError::Api { status: 404, message: "No such container: x".into() }
        );
        assert_eq!(api_error(500, b"plain text").msg().english(), "Docker reported an error: plain text");
    }
}

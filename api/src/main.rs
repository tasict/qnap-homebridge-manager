//! hbmgr: the backend of the Homebridge QPKG. One program, three uses:
//!
//! - CGI (when `GATEWAY_INTERFACE` is set): one request per process, published at `/Homebridge/api`.
//! - `hbmgr job-run <id>`: the detached worker of a background job.
//! - CLI for an administrator's shell: `hbmgr api <METHOD> <path> [json-body]`, `hbmgr check-updates`,
//!   `hbmgr --version`.

/// A Msg (see `msg.rs`): `msg!("Key with {name}", name = value)`. The key must be a string literal.
macro_rules! msg {
    ($key:literal $(,)?) => {
        $crate::msg::Msg::new($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::msg::Msg::new($key)$(.arg(stringify!($name), $value))+
    };
}

mod api;
mod auth;
mod cgi;
mod cleanup;
mod discover;
mod docker;
mod error;
mod hbinfo;
mod http;
mod instance;
mod jobs;
mod msg;
mod net;
mod ops;
mod paths;
mod pull;
mod spec;
mod store;
mod sys;
mod tasks;
mod updates;
mod util;

use std::io::Write;

pub const VERSION: &str = match option_env!("HBMGR_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};

const USAGE: &str = "usage: hbmgr api <METHOD> <path> [json-body]
       hbmgr check-updates
       hbmgr job-run <id>
       hbmgr --version";

fn main() {
    if std::env::var_os("GATEWAY_INTERFACE").is_some() {
        std::process::exit(cgi::run());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--version" | "-V" | "version"] => {
            println!("hbmgr {VERSION}");
            0
        }
        ["job-run", id] => tasks::worker(id),
        ["check-updates"] => check_updates(),
        ["api", method, path, rest @ ..] if rest.len() <= 1 => cli_api(method, path, rest.first().copied().unwrap_or("")),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

/// Runs the router like the CGI does, without the session and origin checks (an administrator's shell is trusted).
fn cli_api(method: &str, path: &str, body: &str) -> i32 {
    let store = store::Store::open();
    let req = api::Req::new(method, path, "", body.as_bytes().to_vec());
    let result = api::route(&req, &store, "cli");
    let out = match &result {
        Ok(v) => serde_json::json!({ "ok": true, "data": v }),
        Err(e) => e.to_json(),
    };
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    if result.is_ok() { 0 } else { 1 }
}

/// Checks every image now, in the foreground (nothing schedules this; the web page asks for checks itself).
fn check_updates() -> i32 {
    let store = store::Store::open();
    let Ok(Some(_lock)) = jobs::try_lock(&store, jobs::UPDATES_KEY, std::time::Duration::ZERO) else {
        eprintln!("hbmgr: an update check is already running");
        return 1;
    };
    let docker = docker::Docker::from_env();
    if let Err(e) = docker.version() {
        eprintln!("hbmgr: {e}");
        return 1;
    }
    let u = updates::check_all(&store, &docker, |_, _, _| {});
    for i in &u.images {
        let state = match (&i.error, i.available) {
            (Some(e), _) => e.clone(),
            (None, true) => "update available".into(),
            (None, false) => "up to date".into(),
        };
        println!("{}: {state}", i.image);
    }
    0
}

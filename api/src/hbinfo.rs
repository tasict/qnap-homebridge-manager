//! What the work directory says about the Homebridge inside: bridge name, PIN, the UI's port and scheme, plugins.
//! The files are the user's: they are only read, with size limits, and anything unreadable just gives `null`.

use serde_json::{Value, json};
use std::path::Path;

const MAX_CONFIG: u64 = 4 * 1024 * 1024;
const DEFAULT_UI_PORT: u64 = 8581;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiSettings {
    pub port: u64,
    pub ssl: bool,
}

/// The Homebridge UI's port and scheme from `config.json`: the `config` platform's `port` (8581 when absent) and
/// `ssl` (HTTPS when it names a key and certificate or a PFX file).
pub fn ui_settings(config: &Value) -> UiSettings {
    let platform = config["platforms"].as_array().and_then(|a| a.iter().find(|p| p["platform"].as_str() == Some("config")));
    let Some(p) = platform else { return UiSettings { port: DEFAULT_UI_PORT, ssl: false } };
    let port = p["port"].as_u64().filter(|n| (1..=65_535).contains(n)).unwrap_or(DEFAULT_UI_PORT);
    let ssl = &p["ssl"];
    let set = |k: &str| ssl[k].as_str().is_some_and(|s| !s.trim().is_empty());
    UiSettings { port, ssl: ssl.is_object() && ((set("key") && set("cert")) || set("pfx")) }
}

/// The address of the UI on `host`; the port is left out when it is the scheme's default.
pub fn ui_url(ui: &UiSettings, host: &str) -> String {
    let scheme = if ui.ssl { "https" } else { "http" };
    let default_port = if ui.ssl { 443 } else { 80 };
    if ui.port == default_port { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{}", ui.port) }
}

/// Plugin package names from `package.json`'s dependencies (Homebridge itself and its UI are not plugins).
fn plugins(pkg: &Value) -> Vec<String> {
    let Some(deps) = pkg["dependencies"].as_object() else { return Vec::new() };
    deps.keys()
        .filter(|k| {
            let base = k.rsplit('/').next().unwrap_or(k);
            base.starts_with("homebridge-") && base != "homebridge-config-ui-x"
        })
        .cloned()
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = crate::util::read_limited(path, MAX_CONFIG)?;
    serde_json::from_slice(&bytes).ok()
}

/// The `homebridge` object of an instance. `domain` is the spec's domain (may be empty); `ip` the container's current
/// address when it is running.
pub fn info(workdir: &str, domain: &str, ip: Option<&str>) -> Value {
    let dir = Path::new(workdir);
    let config = read_json(&dir.join("config.json"));
    let pkg = read_json(&dir.join("package.json"));
    // A new work directory has no config.json until Homebridge first starts: its UI then uses the defaults.
    let ui = config.as_ref().map(ui_settings).unwrap_or(UiSettings { port: DEFAULT_UI_PORT, ssl: false });
    let str_at = |v: Option<&Value>, a: &str, b: &str| v.and_then(|c| c[a][b].as_str()).map(str::to_string);
    let (ui_url_v, ui_url_ip) = match ip.filter(|i| !i.is_empty()) {
        Some(ip) => {
            let host = if domain.is_empty() { ip } else { domain };
            (json!(ui_url(&ui, host)), json!(ui_url(&ui, ip)))
        }
        None => (Value::Null, Value::Null),
    };
    json!({
        "bridge_name": str_at(config.as_ref(), "bridge", "name"),
        "pin": str_at(config.as_ref(), "bridge", "pin"),
        "ui_port": ui.port,
        "ui_ssl": ui.ssl,
        "ui_url": ui_url_v,
        "ui_url_ip": ui_url_ip,
        "plugins": pkg.as_ref().map(plugins).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ui_settings() {
        let c = json!({ "platforms": [ { "platform": "x" }, { "platform": "config", "port": 443, "ssl": { "key": "/stunnel/stunnel.pem", "cert": "/stunnel/stunnel.pem" } } ] });
        assert_eq!(ui_settings(&c), UiSettings { port: 443, ssl: true });
        let plain = json!({ "platforms": [ { "platform": "config", "port": 80 } ] });
        assert_eq!(ui_settings(&plain), UiSettings { port: 80, ssl: false });
        let pfx = json!({ "platforms": [ { "platform": "config", "ssl": { "pfx": "/x.pfx" } } ] });
        assert_eq!(ui_settings(&pfx), UiSettings { port: 8581, ssl: true });
        let empty_ssl = json!({ "platforms": [ { "platform": "config", "port": 8581, "ssl": {} } ] });
        assert!(!ui_settings(&empty_ssl).ssl);
        assert_eq!(ui_settings(&json!({})), UiSettings { port: 8581, ssl: false });
        assert_eq!(ui_settings(&json!({ "platforms": [ { "platform": "config", "port": "x" } ] })).port, 8581);
    }

    #[test]
    fn builds_ui_addresses() {
        let https = UiSettings { port: 443, ssl: true };
        assert_eq!(ui_url(&https, "1f.example.com"), "https://1f.example.com");
        let http80 = UiSettings { port: 80, ssl: false };
        assert_eq!(ui_url(&http80, "192.168.10.8"), "http://192.168.10.8");
        let default = UiSettings { port: 8581, ssl: false };
        assert_eq!(ui_url(&default, "192.168.10.7"), "http://192.168.10.7:8581");
        let https_other = UiSettings { port: 8443, ssl: true };
        assert_eq!(ui_url(&https_other, "h.example.com"), "https://h.example.com:8443");
        let http443 = UiSettings { port: 443, ssl: false };
        assert_eq!(ui_url(&http443, "h"), "http://h:443");
    }

    #[test]
    fn reads_a_work_directory() {
        let dir = std::env::temp_dir().join(format!("hbmgr-hbinfo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"bridge":{"name":"Homebridge 1F","pin":"031-45-154"},"platforms":[{"platform":"config","port":443,"ssl":{"key":"/k","cert":"/c"}}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dependencies":{"homebridge":"^2","homebridge-config-ui-x":"5","homebridge-mqttthing-ex":"1","@homebridge-plugins/homebridge-camera-ffmpeg":"4"}}"#,
        )
        .unwrap();
        let d = dir.to_string_lossy().to_string();
        let v = info(&d, "1f.example.com", Some("192.168.10.7"));
        assert_eq!(v["bridge_name"], "Homebridge 1F");
        assert_eq!(v["pin"], "031-45-154");
        assert_eq!(v["ui_port"], 443);
        assert_eq!(v["ui_ssl"], true);
        assert_eq!(v["ui_url"], "https://1f.example.com");
        assert_eq!(v["ui_url_ip"], "https://192.168.10.7");
        assert_eq!(v["plugins"], json!(["@homebridge-plugins/homebridge-camera-ffmpeg", "homebridge-mqttthing-ex"]));
        let no_domain = info(&d, "", Some("192.168.10.7"));
        assert_eq!(no_domain["ui_url"], "https://192.168.10.7");
        let stopped = info(&d, "1f.example.com", None);
        assert_eq!((stopped["ui_url"].clone(), stopped["ui_url_ip"].clone()), (Value::Null, Value::Null));
        std::fs::write(dir.join("config.json"), "{ not json").unwrap();
        let broken = info(&d, "", Some("192.168.10.7"));
        assert_eq!(broken["bridge_name"], Value::Null);
        assert_eq!(broken["ui_url"], "http://192.168.10.7:8581", "the UI's defaults");
        let missing = info("/nonexistent/dir", "", Some("192.168.10.7"));
        assert_eq!(missing["plugins"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

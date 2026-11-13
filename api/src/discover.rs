//! Existing Homebridge containers that this package does not manage yet, turned into a spec that takes them over.

use crate::msg::Msg;
use crate::net::Interface;
use crate::spec::{self, Mount, Network, Spec};
use serde_json::Value;
use std::collections::BTreeMap;

/// An image repository (without tag or digest) is Homebridge's when its last part ends in `homebridge`
/// (`homebridge/homebridge`, `oznu/homebridge`, `ghcr.io/homebridge/homebridge`).
pub fn homebridge_repository(repo: &str) -> bool {
    repo.rsplit('/').next().is_some_and(|last| last.ends_with("homebridge"))
}

/// A container looks like Homebridge when its image repository is Homebridge's or something is mounted at
/// `/homebridge`.
pub fn looks_like_homebridge(inspect: &Value) -> bool {
    let image = inspect["Config"]["Image"].as_str().unwrap_or("");
    let repo = spec::split_image(image.split('@').next().unwrap_or("")).0;
    homebridge_repository(repo) || mounts_of(inspect).iter().any(|m| m["Destination"].as_str() == Some(spec::WORKDIR_TARGET))
}

fn mounts_of(inspect: &Value) -> Vec<Value> {
    inspect["Mounts"].as_array().cloned().unwrap_or_default()
}

/// The qnet network a container is on: (network name, interface, static address or empty, MAC).
fn qnet_endpoint(inspect: &Value, networks: &[Value]) -> Option<(String, String, String, String)> {
    let eps = inspect["NetworkSettings"]["Networks"].as_object()?;
    for (name, ep) in eps {
        let Some(net) = networks.iter().find(|n| n["Name"].as_str() == Some(name.as_str())) else { continue };
        if net["Driver"].as_str() != Some("qnet") {
            continue;
        }
        let iface = net["IPAM"]["Options"]["iface"].as_str().or_else(|| net["Options"]["iface"].as_str()).unwrap_or("");
        let ip = ep["IPAMConfig"]["IPv4Address"].as_str().unwrap_or("");
        let mac = ep["MacAddress"].as_str().or_else(|| inspect["Config"]["MacAddress"].as_str()).unwrap_or("");
        return Some((name.clone(), iface.to_string(), ip.to_string(), mac.to_string()));
    }
    None
}

/// The prefilled spec for taking a container over, and notes on what cannot be carried over.
/// `image_env` is the image's own environment, which is left out.
pub fn to_spec(inspect: &Value, image_env: &[String], networks: &[Value], interfaces: &[Interface], tz: &str) -> (Spec, Vec<Msg>) {
    let mut notes = Vec::new();
    let name = inspect["Name"].as_str().unwrap_or("").trim_start_matches('/').to_string();
    if !spec::valid_name(&name) {
        notes.push(msg!(
            "The container name {name} does not follow the naming rules. Rename the container before importing it.",
            name = &name
        ));
    }

    let raw_image = inspect["Config"]["Image"].as_str().unwrap_or("");
    let image = match spec::normalize_image(raw_image.split('@').next().unwrap_or("")) {
        Ok(i) => {
            if raw_image.contains('@') {
                notes.push(msg!("The image is pinned by digest (@sha256); its tag will be used instead"));
            }
            i
        }
        Err(_) => {
            notes.push(msg!("The image {image} cannot be used; the default image will be used instead", image = raw_image));
            spec::DEFAULT_IMAGE.to_string()
        }
    };

    let id_prefix = inspect["Id"].as_str().unwrap_or("").get(..12).unwrap_or("").to_string();
    let host_raw = inspect["Config"]["Hostname"].as_str().unwrap_or("").to_ascii_lowercase();
    let hostname = if host_raw.is_empty() || host_raw == id_prefix || !spec::valid_hostname(&host_raw) {
        spec::default_hostname(&name)
    } else {
        host_raw
    };
    let domain = if hostname.contains('.') && spec::valid_domain(&hostname) { hostname.clone() } else { String::new() };

    let default_iface = interfaces.iter().find(|i| i.default).or(interfaces.first()).map(|i| i.iface.clone()).unwrap_or_default();
    let network = match qnet_endpoint(inspect, networks) {
        Some((_, iface, ip, mac)) => {
            let iface = if interfaces.iter().any(|i| i.iface == iface) {
                iface
            } else {
                notes.push(msg!("The network interface {iface} was not found. Choose one again.", iface = &iface));
                default_iface.clone()
            };
            let mac = spec::normalize_mac(&mac).unwrap_or_default();
            if ip.is_empty() {
                Network { mode: "dhcp".into(), iface, ip: String::new(), mac }
            } else {
                Network { mode: "static".into(), iface, ip, mac }
            }
        }
        None => {
            let mode = inspect["HostConfig"]["NetworkMode"].as_str().unwrap_or("");
            notes.push(msg!(
                "The container uses the {mode} network, which cannot be kept; it will get a LAN address by DHCP instead",
                mode = mode
            ));
            Network { mode: "dhcp".into(), iface: default_iface, ip: String::new(), mac: String::new() }
        }
    };
    if inspect["NetworkSettings"]["Networks"].as_object().is_some_and(|m| m.len() > 1) {
        notes.push(msg!("The container is connected to several networks; only one of them is kept"));
    }

    let mut workdir = String::new();
    let mut mounts = Vec::new();
    for m in mounts_of(inspect) {
        let dest = m["Destination"].as_str().unwrap_or("");
        let src = m["Source"].as_str().unwrap_or("");
        let kind = m["Type"].as_str().unwrap_or("");
        if kind != "bind" {
            let label = m["Name"].as_str().unwrap_or(dest);
            notes.push(msg!("The volume {name} ({path}) is not a folder on the NAS and cannot be kept", name = label, path = dest));
            continue;
        }
        if dest == spec::WORKDIR_TARGET {
            workdir = src.to_string();
            continue;
        }
        if !src.starts_with("/share/") {
            notes.push(msg!("The folder {path} is not under /share and cannot be mounted", path = src));
            continue;
        }
        mounts.push(Mount { host: src.to_string(), container: dest.to_string(), ro: !m["RW"].as_bool().unwrap_or(true) });
    }
    mounts.sort_by(|a, b| a.container.cmp(&b.container));
    if workdir.is_empty() {
        notes.push(msg!("No work folder is mounted at /homebridge. Choose one."));
    }

    let mut env = BTreeMap::new();
    for e in inspect["Config"]["Env"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if image_env.iter().any(|i| i == e) {
            continue;
        }
        if let Some((k, v)) = e.split_once('=') {
            env.insert(k.to_string(), v.to_string());
        }
    }
    if !env.contains_key("TZ") && !tz.is_empty() {
        env.insert("TZ".into(), tz.to_string());
    }

    let restart = match inspect["HostConfig"]["RestartPolicy"]["Name"].as_str().unwrap_or("") {
        "always" => "always",
        "unless-stopped" => "unless-stopped",
        "on-failure" => {
            notes.push(msg!("The restart policy on-failure becomes unless-stopped"));
            "unless-stopped"
        }
        _ => "no",
    };

    let hc = &inspect["HostConfig"];
    if hc["PortBindings"].as_object().is_some_and(|m| !m.is_empty()) {
        notes.push(msg!("Port mappings are not kept: the container has its own LAN address and does not need them"));
    }
    if hc["Devices"].as_array().is_some_and(|a| !a.is_empty()) {
        notes.push(msg!("Device mappings (such as USB devices) are not kept"));
    }
    if hc["Privileged"].as_bool() == Some(true) {
        notes.push(msg!("Privileged mode is not kept"));
    }
    if hc["CapAdd"].as_array().is_some_and(|a| !a.is_empty()) {
        notes.push(msg!("Extra capabilities (CapAdd) are not kept"));
    }
    if inspect["Config"]["Cmd"].as_array().is_some_and(|a| !a.is_empty()) {
        notes.push(msg!("The custom start command is not kept"));
    }

    let spec = Spec {
        name: name.clone(),
        display_name: name,
        emoji: spec::DEFAULT_EMOJI.into(),
        color: spec::COLORS[0].into(),
        image,
        domain,
        hostname,
        network,
        workdir,
        mounts,
        env,
        restart: restart.into(),
    };
    (spec, notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inspect() -> Value {
        serde_json::from_str(include_str!("../testdata/inspect-homebridge.json")).unwrap()
    }

    fn networks() -> Vec<Value> {
        vec![json!({ "Name": "qnet-static-bond0-3f9a2c", "Driver": "qnet", "IPAM": { "Driver": "qnet", "Options": { "iface": "bond0" },
            "Config": [ { "Subnet": "192.168.8.0/22", "Gateway": "192.168.8.1" } ] }, "Options": {} })]
    }

    fn image_env() -> Vec<String> {
        inspect()["Config"]["Env"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|e| !e.starts_with("TZ="))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn recognises_homebridge() {
        assert!(looks_like_homebridge(&inspect()));
        assert!(looks_like_homebridge(&json!({ "Config": { "Image": "ghcr.io/homebridge/homebridge:latest" } })));
        assert!(looks_like_homebridge(&json!({ "Config": { "Image": "custom:1" }, "Mounts": [ { "Destination": "/homebridge" } ] })));
        assert!(!looks_like_homebridge(&json!({ "Config": { "Image": "eclipse-mosquitto:openssl" }, "Mounts": [] })));
        assert!(!looks_like_homebridge(&json!({ "Config": { "Image": "homebridge-tools/other:1" } })));
    }

    #[test]
    fn converts_a_captured_container() {
        let ifs = [crate::spec::tests::iface()];
        let (s, notes) = to_spec(&inspect(), &image_env(), &networks(), &ifs, "Asia/Taipei");
        assert_eq!(s.name, "homebridge-1F");
        assert_eq!(s.image, "oznu/homebridge:latest");
        assert_eq!(s.hostname, "1f.example.com");
        assert_eq!(s.domain, "1f.example.com", "a host name with a dot is also the domain");
        assert_eq!(
            s.network,
            Network { mode: "static".into(), iface: "bond0".into(), ip: "192.168.10.7".into(), mac: "02:42:5e:7a:21:0c".into() }
        );
        assert_eq!(s.workdir, "/share/Docker/homebridge-1F");
        assert_eq!(
            s.mounts,
            vec![
                Mount { host: "/share/Develop".into(), container: "/Develop".into(), ro: false },
                Mount { host: "/share/Docker/stunnel".into(), container: "/stunnel".into(), ro: false },
            ]
        );
        assert_eq!(s.env.len(), 1, "only what the image does not set: {:?}", s.env);
        assert_eq!(s.env["TZ"], "Asia/Taipei");
        assert_eq!(s.restart, "always");
        assert!(notes.is_empty(), "{notes:?}");
        // The spec is accepted as it is by the create endpoint's validation.
        let v = serde_json::to_value(&s).unwrap();
        let parsed = crate::spec::parse(&v, &crate::spec::tests::ctx_with(&ifs)).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn notes_what_cannot_be_kept() {
        let mut i = inspect();
        i["Config"]["Hostname"] = json!("b190e2fb6016");
        i["HostConfig"]["NetworkMode"] = json!("bridge");
        i["NetworkSettings"]["Networks"] =
            json!({ "bridge": { "IPAMConfig": null, "IPAddress": "10.0.3.2", "MacAddress": "02:42:0a:00:03:02" } });
        i["HostConfig"]["PortBindings"] = json!({ "8581/tcp": [ { "HostPort": "8581" } ] });
        i["HostConfig"]["Devices"] = json!([ { "PathOnHost": "/dev/ttyUSB0" } ]);
        i["HostConfig"]["RestartPolicy"]["Name"] = json!("on-failure");
        i["Mounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "Type": "volume", "Name": "hbdata", "Source": "/x", "Destination": "/data", "RW": true }));
        let ifs = [crate::spec::tests::iface()];
        let (s, notes) = to_spec(&i, &image_env(), &networks(), &ifs, "Asia/Taipei");
        assert_eq!(s.hostname, "homebridge-1f", "Docker's generated host name is not kept");
        assert_eq!(s.domain, "");
        assert_eq!(s.network.mode, "dhcp");
        assert_eq!(s.network.iface, "bond0");
        assert_eq!(s.restart, "unless-stopped");
        assert_eq!(notes.len(), 5, "{notes:?}");
    }
}

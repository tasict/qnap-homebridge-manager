//! The instance spec: what the user configures, how it is checked, and what the container is created from.

use crate::error::Fields;
use crate::msg::Msg;
use crate::net::{Interface, Subnet};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

pub const BACKUP_SUFFIX: &str = ".hbmgr-prev";
pub const LABEL_INSTANCE: &str = "hbmgr.instance";
pub const LABEL_SPEC: &str = "hbmgr.spec";
/// The hashed part of the spec as JSON, so a rollback knows exactly what a container was created from.
pub const LABEL_CONFIG: &str = "hbmgr.config";
pub const COLORS: [&str; 8] = ["violet", "orange", "teal", "pink", "blue", "green", "amber", "slate"];
pub const DEFAULT_EMOJI: &str = "🏠";
pub const DEFAULT_IMAGE: &str = "homebridge/homebridge:latest";
pub const WORKDIR_TARGET: &str = "/homebridge";
const SYSTEM_DIRS: [&str; 11] = ["/proc", "/sys", "/dev", "/etc", "/bin", "/sbin", "/lib", "/usr", "/var/run", "/run", "/init"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Spec {
    pub name: String,
    pub display_name: String,
    pub emoji: String,
    pub color: String,
    pub image: String,
    pub domain: String,
    pub hostname: String,
    pub network: Network,
    pub workdir: String,
    pub mounts: Vec<Mount>,
    pub env: BTreeMap<String, String>,
    pub restart: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Network {
    pub mode: String,
    pub iface: String,
    pub ip: String,
    pub mac: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Mount {
    pub host: String,
    pub container: String,
    pub ro: bool,
}

pub fn backup_name(name: &str) -> String {
    format!("{name}{BACKUP_SUFFIX}")
}

/// What spec validation needs from outside.
pub struct Ctx<'a> {
    pub interfaces: &'a [Interface],
    pub tz: &'a str,
    /// Checks a host folder (`may_be_missing` for a work directory that is created later) and returns its normalised
    /// path, or a message.
    pub check_path: &'a dyn Fn(&str, bool) -> Result<String, Msg>,
}

/// Field errors gathered while reading a request body.
struct Errs {
    fields: Fields,
}

impl Errs {
    fn add(&mut self, path: &str, msg: Msg) {
        self.fields.entry(path.to_string()).or_insert(msg);
    }
}

fn opt_str<'a>(v: &'a Value, key: &str, path: &str, e: &mut Errs) -> Option<&'a str> {
    match v.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => {
            e.add(path, msg!("This value must be text"));
            None
        }
    }
}

/// Reads and checks a spec from a request body. Defaults are filled in; the MAC address stays empty when none was
/// given (it is generated when the instance is created).
pub fn parse(v: &Value, ctx: &Ctx) -> Result<Spec, Fields> {
    let mut e = Errs { fields: Fields::new() };
    if !v.is_object() {
        e.add("spec", msg!("The instance settings are missing"));
        return Err(e.fields);
    }
    let mut s = Spec::default();

    let name = opt_str(v, "name", "name", &mut e).unwrap_or("").trim();
    if name.is_empty() {
        e.add("name", msg!("Enter a name"));
    } else if !valid_name(name) {
        e.add("name", msg!("Use up to 63 letters, digits, underscores, dots and hyphens, starting with a letter or digit"));
    } else if name.to_ascii_lowercase().ends_with(BACKUP_SUFFIX) {
        e.add("name", msg!("The name cannot end in .hbmgr-prev"));
    }
    s.name = name.to_string();

    let display = opt_str(v, "display_name", "display_name", &mut e).unwrap_or("").trim();
    if display.chars().count() > 40 {
        e.add("display_name", msg!("The display name can be at most 40 characters long"));
    } else if display.chars().any(char::is_control) {
        e.add("display_name", msg!("The display name contains characters that are not allowed"));
    }
    s.display_name = if display.is_empty() { s.name.clone() } else { display.to_string() };

    let emoji = opt_str(v, "emoji", "emoji", &mut e).unwrap_or("").trim();
    if emoji.len() > 16 || emoji.chars().any(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || c.is_control()) {
        e.add("emoji", msg!("Choose a single emoji"));
    }
    s.emoji = if emoji.is_empty() { DEFAULT_EMOJI.into() } else { emoji.to_string() };

    let color = opt_str(v, "color", "color", &mut e).unwrap_or("").trim();
    if !color.is_empty() && !COLORS.contains(&color) {
        e.add("color", msg!("This color is not available"));
    }
    s.color = if color.is_empty() { COLORS[0].into() } else { color.to_string() };

    let image = opt_str(v, "image", "image", &mut e).unwrap_or("").trim();
    match normalize_image(image) {
        Ok(i) => s.image = i,
        Err(m) => e.add("image", m),
    }

    let domain = opt_str(v, "domain", "domain", &mut e).unwrap_or("").trim().trim_end_matches('.').to_ascii_lowercase();
    if !domain.is_empty() && !valid_domain(&domain) {
        e.add("domain", msg!("The domain name is not valid (for example hb.example.com)"));
    }
    s.domain = domain;

    let hostname = opt_str(v, "hostname", "hostname", &mut e).unwrap_or("").trim().trim_end_matches('.').to_ascii_lowercase();
    if !hostname.is_empty() && !valid_hostname(&hostname) {
        e.add("hostname", msg!("Use letters, digits, hyphens and dots, with at most 63 characters between dots"));
    }
    s.hostname = if !hostname.is_empty() {
        hostname
    } else if !s.domain.is_empty() {
        s.domain.clone()
    } else {
        default_hostname(&s.name)
    };

    parse_network(v.get("network"), ctx, &mut s.network, &mut e);

    match opt_str(v, "workdir", "workdir", &mut e).map(str::trim) {
        None | Some("") => e.add("workdir", msg!("Choose a work folder")),
        Some(w) => match (ctx.check_path)(w, true) {
            Ok(p) => s.workdir = p,
            Err(m) => e.add("workdir", m),
        },
    }

    s.mounts = parse_mounts(v.get("mounts"), "mounts", ctx, &mut e);

    match v.get("env") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            if m.len() > 64 {
                e.add("env", msg!("You can set at most 64 environment variables"));
            }
            for (k, val) in m {
                let path = format!("env.{k}");
                if !valid_env_name(k) {
                    e.add(&path, msg!("Variable names use letters, digits and underscores and cannot start with a digit"));
                    continue;
                }
                match val {
                    Value::String(x) if x.len() > 4096 => e.add(&path, msg!("The value can be at most 4096 bytes long")),
                    Value::String(x) if x.contains(['\0', '\n', '\r']) => e.add(&path, msg!("The value cannot contain line breaks")),
                    Value::String(x) => {
                        s.env.insert(k.clone(), x.clone());
                    }
                    _ => e.add(&path, msg!("This value must be text")),
                }
            }
        }
        Some(_) => e.add("env", msg!("This value is not in the right format")),
    }
    if !s.env.contains_key("TZ") && !ctx.tz.is_empty() {
        s.env.insert("TZ".into(), ctx.tz.to_string());
    }

    let restart = opt_str(v, "restart", "restart", &mut e).unwrap_or("").trim();
    s.restart = match restart {
        "" => "always".into(),
        "always" | "unless-stopped" | "no" => restart.into(),
        _ => {
            e.add("restart", msg!("The restart policy must be always, unless-stopped or no"));
            String::new()
        }
    };

    if e.fields.is_empty() { Ok(s) } else { Err(e.fields) }
}

fn parse_network(v: Option<&Value>, ctx: &Ctx, out: &mut Network, e: &mut Errs) {
    let Some(v) = v.filter(|v| v.is_object()) else {
        e.add("network", msg!("Set up the network"));
        return;
    };
    let mode = opt_str(v, "mode", "network.mode", e).unwrap_or("").trim();
    if mode != "static" && mode != "dhcp" {
        e.add("network.mode", msg!("Choose a fixed IP address or DHCP"));
    }
    out.mode = mode.to_string();
    let iface_name = opt_str(v, "iface", "network.iface", e).unwrap_or("").trim();
    let iface = ctx.interfaces.iter().find(|i| i.iface == iface_name);
    if iface.is_none() {
        e.add("network.iface", msg!("Choose a network interface"));
    }
    out.iface = iface_name.to_string();
    let ip = opt_str(v, "ip", "network.ip", e).unwrap_or("").trim();
    if mode == "static" {
        if ip.is_empty() {
            e.add("network.ip", msg!("Enter an IP address"));
        } else if let Some(iface) = iface {
            match ip_problem(ip, iface) {
                Some(m) => e.add("network.ip", m),
                None => out.ip = ip.parse::<Ipv4Addr>().map(|a| a.to_string()).unwrap_or_default(),
            }
        }
    } else if mode == "dhcp" && !ip.is_empty() {
        e.add("network.ip", msg!("Leave the IP address empty when using DHCP"));
    }
    let mac = opt_str(v, "mac", "network.mac", e).unwrap_or("").trim();
    if !mac.is_empty() {
        match normalize_mac(mac) {
            Some(m) => out.mac = m,
            None => e.add("network.mac", msg!("Enter a unicast MAC address such as 02:42:0a:01:02:03")),
        }
    }
}

/// Why `ip` cannot be used on `iface`, or `None` when it can.
pub fn ip_problem(ip: &str, iface: &Interface) -> Option<Msg> {
    let Ok(addr) = ip.trim().parse::<Ipv4Addr>() else { return Some(msg!("The IP address is not valid")) };
    let Some(subnet) = iface.subnet.as_deref().and_then(Subnet::parse) else {
        return Some(msg!("This network interface has no usable subnet. Use DHCP instead."));
    };
    if !subnet.contains(addr) {
        return Some(msg!("The IP address is outside this interface's subnet {subnet}", subnet = subnet.to_string()));
    }
    if subnet.prefix < 31 && (addr == subnet.network() || addr == subnet.broadcast()) {
        return Some(msg!("The network and broadcast addresses cannot be used"));
    }
    if iface.gateway.as_deref().and_then(|g| g.parse::<Ipv4Addr>().ok()) == Some(addr) {
        return Some(msg!("This is the gateway's address"));
    }
    if iface.host_ip.as_deref().and_then(|g| g.parse::<Ipv4Addr>().ok()) == Some(addr) {
        return Some(msg!("This is the NAS's own address"));
    }
    None
}

pub fn parse_mounts(v: Option<&Value>, prefix: &str, ctx: &Ctx, e_out: &mut impl MountErrs) -> Vec<Mount> {
    let mut out = Vec::new();
    let list = match v {
        None | Some(Value::Null) => return out,
        Some(Value::Array(a)) => a,
        Some(_) => {
            e_out.add_err(prefix, msg!("This value is not in the right format"));
            return out;
        }
    };
    if list.len() > 32 {
        e_out.add_err(prefix, msg!("You can add at most 32 folders"));
    }
    for (i, m) in list.iter().enumerate() {
        let p = |k: &str| format!("{prefix}.{i}.{k}");
        if !m.is_object() {
            e_out.add_err(&format!("{prefix}.{i}"), msg!("This value is not in the right format"));
            continue;
        }
        let mut mount = Mount::default();
        match m.get("host").and_then(Value::as_str).map(str::trim) {
            None | Some("") => e_out.add_err(&p("host"), msg!("Choose a folder on the NAS")),
            Some(h) => match (ctx.check_path)(h, false) {
                Ok(n) => mount.host = n,
                Err(msg) => e_out.add_err(&p("host"), msg),
            },
        }
        match m.get("container").and_then(Value::as_str).map(str::trim) {
            None | Some("") => e_out.add_err(&p("container"), msg!("Enter the path inside the container")),
            Some(c) => match container_path_problem(c) {
                Some(msg) => e_out.add_err(&p("container"), msg),
                None if out.iter().any(|o: &Mount| o.container == c) => {
                    e_out.add_err(&p("container"), msg!("This path inside the container is already used"))
                }
                None => mount.container = c.to_string(),
            },
        }
        match m.get("ro") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => mount.ro = *b,
            Some(_) => e_out.add_err(&p("ro"), msg!("This value is not in the right format")),
        }
        out.push(mount);
    }
    out
}

/// Where mount errors go (spec fields, settings fields).
pub trait MountErrs {
    fn add_err(&mut self, path: &str, msg: Msg);
}

impl MountErrs for Errs {
    fn add_err(&mut self, path: &str, msg: Msg) {
        self.add(path, msg);
    }
}

impl MountErrs for Fields {
    fn add_err(&mut self, path: &str, msg: Msg) {
        self.entry(path.to_string()).or_insert(msg);
    }
}

fn container_path_problem(c: &str) -> Option<Msg> {
    if !c.starts_with('/') {
        return Some(msg!("Use an absolute path that starts with /"));
    }
    if c.len() > 1024 || c.contains(['\0', ':', ',', '\n']) {
        return Some(msg!("The path contains characters that are not allowed"));
    }
    if c != "/" && (c.ends_with('/') || c.contains("//") || c.split('/').any(|p| p == "." || p == "..")) {
        return Some(msg!("Write the path without ., .. or extra slashes"));
    }
    if c == "/" {
        return Some(msg!("A folder cannot be mounted at /"));
    }
    if c == WORKDIR_TARGET || c.starts_with("/homebridge/") {
        return Some(msg!("/homebridge is where the work folder is mounted"));
    }
    if SYSTEM_DIRS.iter().any(|s| c == *s || c.starts_with(&format!("{s}/"))) {
        return Some(msg!("Folders cannot be mounted at system locations"));
    }
    None
}

pub fn valid_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty() && b.len() <= 63 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(c))
}

fn valid_env_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty()
        && b.len() <= 256
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// An RFC 1123 host name in lower case: dot-separated labels of letters, digits and inner hyphens, ≤ 63 each, ≤ 253
/// in all.
pub fn valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.split('.').all(|l| {
            let b = l.as_bytes();
            !b.is_empty()
                && b.len() <= 63
                && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
                && b[0] != b'-'
                && b[b.len() - 1] != b'-'
        })
}

/// A domain name: a host name with at least two labels whose last label is not a number (so not an IP address).
pub fn valid_domain(d: &str) -> bool {
    valid_hostname(d) && d.contains('.') && d.rsplit('.').next().is_some_and(|tld| !tld.bytes().all(|c| c.is_ascii_digit()))
}

/// The container host name used when none is given: the name in lower case, with `_` (not allowed in host names)
/// as `-`.
pub fn default_hostname(name: &str) -> String {
    let h: String = name.to_ascii_lowercase().chars().map(|c| if c == '_' { '-' } else { c }).collect();
    let h = h.trim_matches(|c| c == '-' || c == '.').to_string();
    if valid_hostname(&h) { h } else { "homebridge".into() }
}

/// `xx:xx:xx:xx:xx:xx` in lower case, unicast and not all zeros; `-` separators are accepted.
pub fn normalize_mac(m: &str) -> Option<String> {
    let parts: Vec<&str> = m.split([':', '-']).collect();
    if parts.len() != 6 || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    let bytes: Vec<u8> = parts.iter().map(|p| u8::from_str_radix(p, 16).ok()).collect::<Option<_>>()?;
    if bytes[0] & 1 == 1 || bytes.iter().all(|b| *b == 0) {
        return None;
    }
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"))
}

/// A random locally administered unicast MAC in Docker's `02:42:…` range.
pub fn random_mac(taken: &dyn Fn(&str) -> bool) -> String {
    loop {
        let r = crate::util::random_bytes(4);
        let mac = format!("02:42:{:02x}:{:02x}:{:02x}:{:02x}", r[0], r[1], r[2], r[3]);
        if !taken(&mac) && r.iter().any(|b| *b != 0) {
            return mac;
        }
    }
}

/// Splits `repo[:tag]` into the repository and the tag (`latest` when there is none). A `:` before the last `/` is a
/// registry port, not a tag.
pub fn split_image(image: &str) -> (&str, &str) {
    let slash = image.rfind('/').map(|i| i + 1).unwrap_or(0);
    match image[slash..].rfind(':') {
        Some(i) => (&image[..slash + i], &image[slash + i + 1..]),
        None => (image, "latest"),
    }
}

/// Checks an image reference and adds `:latest` when it has no tag. Digests are refused.
pub fn normalize_image(image: &str) -> Result<String, Msg> {
    if image.is_empty() {
        return Err(msg!("Enter an image name"));
    }
    if image.contains('@') {
        return Err(msg!("Use a tag such as :latest; @sha256 digests are not supported"));
    }
    if image.len() > 255 || image.contains(char::is_whitespace) {
        return Err(msg!("The image name is not valid (for example homebridge/homebridge:latest, in lowercase)"));
    }
    let (repo, tag) = split_image(image);
    let tag_ok = !tag.is_empty()
        && tag.len() <= 128
        && tag.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        && !tag.starts_with(['.', '-']);
    let mut parts = repo.split('/').collect::<Vec<_>>();
    // A first part with a dot or a port, or "localhost", is a registry host.
    if parts.len() > 1 && (parts[0].contains(['.', ':']) || parts[0] == "localhost") {
        let host = parts.remove(0);
        let (h, port) = host.split_once(':').unwrap_or((host, "1"));
        if !valid_hostname(&h.to_ascii_lowercase()) || port.is_empty() || !port.bytes().all(|c| c.is_ascii_digit()) {
            return Err(msg!("The image name is not valid (for example homebridge/homebridge:latest, in lowercase)"));
        }
    }
    let path_ok = !parts.is_empty()
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
                && p.as_bytes()[0].is_ascii_alphanumeric()
                && p.as_bytes()[p.len() - 1].is_ascii_alphanumeric()
        });
    if !tag_ok || !path_ok {
        return Err(msg!("The image name is not valid (for example homebridge/homebridge:latest, in lowercase)"));
    }
    Ok(format!("{repo}:{tag}"))
}

/// The repository of an image without Docker Hub's implicit prefixes (`docker.io/library/alpine` → `alpine`).
pub fn canonical_repo(image: &str) -> String {
    let (repo, _) = split_image(image);
    let r = repo.strip_prefix("docker.io/").or_else(|| repo.strip_prefix("index.docker.io/")).unwrap_or(repo);
    r.strip_prefix("library/").unwrap_or(r).to_string()
}

/// Images known to be unmaintained, with what to use instead.
pub fn legacy_image(image: &str) -> Option<&'static str> {
    (canonical_repo(image) == "oznu/homebridge").then_some(DEFAULT_IMAGE)
}

impl Spec {
    /// The part of the spec that the container is created from, as canonical JSON (keys sorted, mounts sorted).
    /// `display_name`, `emoji`, `color` and `domain` are not in it.
    pub fn config_json(&self) -> String {
        let mut mounts = self.mounts.clone();
        mounts.sort_by(|a, b| a.container.cmp(&b.container));
        let v = json!({
            "env": self.env,
            "hostname": self.hostname,
            "image": self.image,
            "mounts": mounts.iter().map(|m| json!({ "container": m.container, "host": m.host, "ro": m.ro })).collect::<Vec<_>>(),
            "network": { "iface": self.network.iface, "ip": self.network.ip, "mac": self.network.mac, "mode": self.network.mode },
            "restart": self.restart,
            "workdir": self.workdir,
        });
        // serde_json's maps are ordered by key, so this is canonical.
        v.to_string()
    }

    /// The spec hash: what the container label `hbmgr.spec` holds.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"hbmgr-spec-v1\n");
        h.update(self.config_json().as_bytes());
        crate::util::hex(&h.finalize()[..16])
    }

    /// Takes the container-related fields from the hashed config JSON of a container label.
    pub fn with_config(&self, config: &str) -> Option<Spec> {
        let v: Value = serde_json::from_str(config).ok()?;
        let mut s = self.clone();
        s.image = v["image"].as_str()?.to_string();
        s.hostname = v["hostname"].as_str()?.to_string();
        s.network = serde_json::from_value(v["network"].clone()).ok()?;
        s.workdir = v["workdir"].as_str()?.to_string();
        s.mounts = serde_json::from_value(v["mounts"].clone()).ok()?;
        s.env = serde_json::from_value(v["env"].clone()).ok()?;
        s.restart = v["restart"].as_str()?.to_string();
        Some(s)
    }

    /// The body of `POST /containers/create` for this spec on the qnet network `network`.
    pub fn create_body(&self, network: &str) -> Value {
        let mut mounts = vec![json!({ "Type": "bind", "Source": self.workdir, "Target": WORKDIR_TARGET, "ReadOnly": false })];
        for m in &self.mounts {
            mounts.push(json!({ "Type": "bind", "Source": m.host, "Target": m.container, "ReadOnly": m.ro }));
        }
        let mut endpoint = Map::new();
        if self.network.mode == "static" {
            endpoint.insert("IPAMConfig".into(), json!({ "IPv4Address": self.network.ip }));
        }
        if !self.network.mac.is_empty() {
            endpoint.insert("MacAddress".into(), json!(self.network.mac));
        }
        let mut endpoints = Map::new();
        endpoints.insert(network.to_string(), Value::Object(endpoint));
        json!({
            "Image": self.image,
            "Hostname": self.hostname,
            "Env": self.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
            "Labels": {
                LABEL_INSTANCE: self.name,
                LABEL_SPEC: self.hash(),
                LABEL_CONFIG: self.config_json(),
            },
            "HostConfig": {
                "NetworkMode": network,
                "RestartPolicy": { "Name": self.restart, "MaximumRetryCount": 0 },
                "Mounts": mounts,
            },
            "NetworkingConfig": { "EndpointsConfig": endpoints },
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn iface() -> Interface {
        Interface {
            iface: "bond0".into(),
            display: "Adapter 1+2 (Virtual Switch 3)".into(),
            subnet: Some("192.168.8.0/22".into()),
            gateway: Some("192.168.8.1".into()),
            host_ip: Some("192.168.11.1".into()),
            default: true,
            arp_iface: "qvs0".into(),
        }
    }

    pub fn fake_paths(p: &str, may_be_missing: bool) -> Result<String, Msg> {
        if !p.starts_with("/share/") {
            return Err(msg!("Only folders under /share can be used"));
        }
        if p.contains("missing") && !may_be_missing {
            return Err(msg!("The folder does not exist"));
        }
        Ok(p.trim_end_matches('/').to_string())
    }

    pub fn ctx_with(ifaces: &[Interface]) -> Ctx<'_> {
        Ctx { interfaces: ifaces, tz: "Asia/Taipei", check_path: &fake_paths }
    }

    fn sample() -> Value {
        json!({
            "name": "homebridge-1F",
            "display_name": "一樓",
            "emoji": "🏠",
            "color": "violet",
            "image": "homebridge/homebridge",
            "domain": "1F.Example.com.",
            "network": { "mode": "static", "iface": "bond0", "ip": "192.168.10.7", "mac": "02:42:5E:7A:21:0C" },
            "workdir": "/share/Docker/homebridge-1F",
            "mounts": [ { "host": "/share/Docker/stunnel", "container": "/stunnel", "ro": false } ],
            "env": { "TZ": "Asia/Taipei" },
            "restart": "always"
        })
    }

    #[test]
    fn parses_and_normalises_a_spec() {
        let ifs = [iface()];
        let s = parse(&sample(), &ctx_with(&ifs)).unwrap();
        assert_eq!(s.image, "homebridge/homebridge:latest");
        assert_eq!(s.domain, "1f.example.com");
        assert_eq!(s.hostname, "1f.example.com", "the host name follows the domain");
        assert_eq!(s.network.mac, "02:42:5e:7a:21:0c");
        assert_eq!(s.mounts.len(), 1);
    }

    #[test]
    fn fills_defaults() {
        let ifs = [iface()];
        let v = json!({ "name": "Bridge_2", "image": "oznu/homebridge:latest", "network": { "mode": "dhcp", "iface": "bond0" },
            "workdir": "/share/Docker/missing-new" });
        let s = parse(&v, &ctx_with(&ifs)).unwrap();
        assert_eq!(s.display_name, "Bridge_2");
        assert_eq!(s.emoji, DEFAULT_EMOJI);
        assert_eq!(s.color, "violet");
        assert_eq!(s.hostname, "bridge-2");
        assert_eq!(s.restart, "always");
        assert_eq!(s.env.get("TZ").map(String::as_str), Some("Asia/Taipei"));
        assert_eq!(s.network.ip, "");
        assert_eq!(s.network.mac, "", "generated later");
    }

    #[test]
    fn reports_every_bad_field() {
        let ifs = [iface()];
        let v = json!({
            "name": "-bad", "color": "red", "image": "Homebridge@sha256:00", "domain": "not a domain", "hostname": "-x",
            "network": { "mode": "static", "iface": "eth9", "ip": "192.168.10.7", "mac": "01:00:5e:00:00:01" },
            "workdir": "/tmp/x",
            "mounts": [ { "host": "/share/a", "container": "/homebridge/x" }, { "host": "/share/b", "container": "/data" },
                        { "host": "/share/missing", "container": "/data" }, { "host": "/share/c", "container": "/usr/lib" },
                        { "host": "/share/d", "container": "relative" }, { "host": "/share/e", "container": "/a/../b" } ],
            "env": { "1X": "y", "OK": "line\nbreak", "N": 5 },
            "restart": "sometimes"
        });
        let f = parse(&v, &ctx_with(&ifs)).unwrap_err();
        for k in [
            "name",
            "color",
            "image",
            "domain",
            "hostname",
            "network.iface",
            "network.mac",
            "workdir",
            "mounts.0.container",
            "mounts.2.host",
            "mounts.3.container",
            "mounts.4.container",
            "mounts.5.container",
            "env.1X",
            "env.OK",
            "env.N",
            "restart",
        ] {
            assert!(f.contains_key(k), "missing error for {k}: {f:?}");
        }
        assert!(!f.contains_key("mounts.1.container"));
        let dup = json!({ "name": "a", "image": "x", "network": { "mode": "dhcp", "iface": "bond0" }, "workdir": "/share/x",
            "mounts": [ { "host": "/share/a", "container": "/data" }, { "host": "/share/b", "container": "/data" } ] });
        assert!(parse(&dup, &ctx_with(&ifs)).unwrap_err().contains_key("mounts.1.container"));
    }

    #[test]
    fn name_rules() {
        assert!(valid_name("homebridge-1F"));
        assert!(valid_name("a"));
        assert!(valid_name("my.bridge_2"));
        assert!(!valid_name("_x"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(64)));
        assert!(!valid_name("a/b"));
        let ifs = [iface()];
        let v = json!({ "name": "x.HBMGR-PREV", "image": "x", "network": { "mode": "dhcp", "iface": "bond0" }, "workdir": "/share/x" });
        assert!(parse(&v, &ctx_with(&ifs)).unwrap_err().contains_key("name"));
    }

    #[test]
    fn network_rules() {
        let i = iface();
        assert_eq!(ip_problem("192.168.10.7", &i), None);
        assert!(ip_problem("192.168.12.1", &i).is_some(), "outside the subnet");
        assert!(ip_problem("192.168.8.0", &i).is_some(), "network address");
        assert!(ip_problem("192.168.11.255", &i).is_some(), "broadcast");
        assert!(ip_problem("192.168.8.1", &i).is_some(), "gateway");
        assert!(ip_problem("192.168.11.1", &i).is_some(), "the NAS");
        assert!(ip_problem("192.168.10.256", &i).is_some());
        let ifs = [i];
        let dhcp_with_ip = json!({ "name": "a", "image": "x", "network": { "mode": "dhcp", "iface": "bond0", "ip": "192.168.10.9" }, "workdir": "/share/x" });
        assert!(parse(&dhcp_with_ip, &ctx_with(&ifs)).unwrap_err().contains_key("network.ip"));
        let static_no_ip = json!({ "name": "a", "image": "x", "network": { "mode": "static", "iface": "bond0" }, "workdir": "/share/x" });
        assert!(parse(&static_no_ip, &ctx_with(&ifs)).unwrap_err().contains_key("network.ip"));
    }

    #[test]
    fn domain_and_hostname_rules() {
        assert!(valid_domain("1f.example.com"));
        assert!(valid_domain("a-b.c"));
        assert!(!valid_domain("localhost"), "needs a dot");
        assert!(!valid_domain("192.168.10.7"), "not an address");
        assert!(!valid_domain("-a.example.com"));
        assert!(!valid_domain("a..b"));
        assert!(!valid_domain("a_b.example.com"));
        assert!(!valid_domain(&format!("{}.com", "a".repeat(64))));
        assert!(!valid_domain(&format!("{}com", "abcdefghi.".repeat(26))), "longer than 253");
        assert!(valid_hostname("homebridge"));
        assert!(!valid_hostname("Upper"), "checked after lowercasing");
        assert_eq!(default_hostname("Homebridge_1F"), "homebridge-1f");
        assert_eq!(default_hostname("x_"), "x");
    }

    #[test]
    fn image_rules() {
        assert_eq!(normalize_image("homebridge/homebridge"), Ok("homebridge/homebridge:latest".into()));
        assert_eq!(normalize_image("alpine:3.24"), Ok("alpine:3.24".into()));
        assert_eq!(normalize_image("ghcr.io/homebridge/homebridge:2024-01-01"), Ok("ghcr.io/homebridge/homebridge:2024-01-01".into()));
        assert_eq!(normalize_image("registry.local:5000/hb"), Ok("registry.local:5000/hb:latest".into()));
        assert!(normalize_image("Homebridge/homebridge").is_err());
        assert!(normalize_image("homebridge@sha256:abc").is_err());
        assert!(normalize_image("a b").is_err());
        assert!(normalize_image("x:").is_err());
        assert_eq!(split_image("registry.local:5000/hb"), ("registry.local:5000/hb", "latest"));
        assert_eq!(canonical_repo("docker.io/library/alpine:3"), "alpine");
        assert_eq!(legacy_image("oznu/homebridge:latest"), Some(DEFAULT_IMAGE));
        assert_eq!(legacy_image("homebridge/homebridge:latest"), None);
    }

    #[test]
    fn mac_rules() {
        assert_eq!(normalize_mac("02-42-0A-BC-DE-01").as_deref(), Some("02:42:0a:bc:de:01"));
        assert_eq!(normalize_mac("01:00:5e:00:00:01"), None, "multicast");
        assert_eq!(normalize_mac("00:00:00:00:00:00"), None);
        assert_eq!(normalize_mac("02:42:0a:bc:de"), None);
        let m = random_mac(&|_| false);
        assert!(m.starts_with("02:42:") && normalize_mac(&m).as_deref() == Some(m.as_str()));
    }

    #[test]
    fn hash_is_stable_and_covers_only_container_fields() {
        let ifs = [iface()];
        let s = parse(&sample(), &ctx_with(&ifs)).unwrap();
        // Pinned: a change here means every managed container shows pending changes after an upgrade of the package.
        assert_eq!(
            s.config_json(),
            r#"{"env":{"TZ":"Asia/Taipei"},"hostname":"1f.example.com","image":"homebridge/homebridge:latest","mounts":[{"container":"/stunnel","host":"/share/Docker/stunnel","ro":false}],"network":{"iface":"bond0","ip":"192.168.10.7","mac":"02:42:5e:7a:21:0c","mode":"static"},"restart":"always","workdir":"/share/Docker/homebridge-1F"}"#
        );
        let h = s.hash();
        assert_eq!(h.len(), 32);
        let mut cosmetic = s.clone();
        cosmetic.display_name = "二樓".into();
        cosmetic.emoji = "🌙".into();
        cosmetic.color = "teal".into();
        cosmetic.domain = "other.example.com".into();
        assert_eq!(cosmetic.hash(), h, "display fields and the domain do not count");
        let mut reordered = s.clone();
        reordered.mounts.push(Mount { host: "/share/a".into(), container: "/a".into(), ro: true });
        let mut other_order = s.clone();
        other_order.mounts.insert(0, Mount { host: "/share/a".into(), container: "/a".into(), ro: true });
        assert_eq!(reordered.hash(), other_order.hash(), "mount order does not count");
        let mut changed = s.clone();
        changed.hostname = "other".into();
        assert_ne!(changed.hash(), h);
        let mut env = s.clone();
        env.env.insert("DEBUG".into(), "1".into());
        assert_ne!(env.hash(), h);
        // Parsing the same body twice gives the same hash (defaults are deterministic).
        assert_eq!(parse(&sample(), &ctx_with(&ifs)).unwrap().hash(), h);
    }

    #[test]
    fn config_label_round_trips() {
        let ifs = [iface()];
        let s = parse(&sample(), &ctx_with(&ifs)).unwrap();
        let mut current = s.clone();
        current.image = "oznu/homebridge:latest".into();
        current.env.clear();
        let back = current.with_config(&s.config_json()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.hash(), s.hash());
    }

    #[test]
    fn builds_the_create_body() {
        let ifs = [iface()];
        let s = parse(&sample(), &ctx_with(&ifs)).unwrap();
        let b = s.create_body("qnet-static-bond0-3f9a2c");
        assert_eq!(b["Image"], "homebridge/homebridge:latest");
        assert_eq!(b["Hostname"], "1f.example.com");
        assert_eq!(b["Env"], json!(["TZ=Asia/Taipei"]));
        assert_eq!(b["Labels"][LABEL_INSTANCE], "homebridge-1F");
        assert_eq!(b["Labels"][LABEL_SPEC], s.hash());
        assert_eq!(b["HostConfig"]["NetworkMode"], "qnet-static-bond0-3f9a2c");
        assert_eq!(b["HostConfig"]["RestartPolicy"]["Name"], "always");
        assert_eq!(
            b["HostConfig"]["Mounts"][0],
            json!({ "Type": "bind", "Source": "/share/Docker/homebridge-1F", "Target": "/homebridge", "ReadOnly": false })
        );
        let ep = &b["NetworkingConfig"]["EndpointsConfig"]["qnet-static-bond0-3f9a2c"];
        assert_eq!(ep["IPAMConfig"]["IPv4Address"], "192.168.10.7");
        assert_eq!(ep["MacAddress"], "02:42:5e:7a:21:0c");
        let mut d = s.clone();
        d.network = Network { mode: "dhcp".into(), iface: "bond0".into(), ip: String::new(), mac: "02:42:00:00:00:01".into() };
        let ep = d.create_body("qnet-dhcp-bond0-hbmgr")["NetworkingConfig"]["EndpointsConfig"]["qnet-dhcp-bond0-hbmgr"].clone();
        assert_eq!(ep, json!({ "MacAddress": "02:42:00:00:00:01" }));
    }
}

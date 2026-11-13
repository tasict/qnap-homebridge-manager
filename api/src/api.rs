//! The router and the request handlers. The same code serves the CGI and `hbmgr api`.

use crate::docker::{Docker, Listed};
use crate::error::{ApiError, Fields};
use crate::instance::{self, Images};
use crate::jobs::{self, instance_key};
use crate::msg::Msg;
use crate::net::{self, Interface, IpUser};
use crate::spec::{self, Spec};
use crate::store::{Record, Settings, Store};
use crate::{discover, paths, sys, tasks, util};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::time::Duration;

/// A request after the CGI checks.
pub struct Req {
    pub method: String,
    pub segs: Vec<String>,
    pub query: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn new(method: &str, path: &str, query: &str, body: Vec<u8>) -> Req {
        let (path, inline_query) = path.split_once('?').unwrap_or((path, ""));
        let q = if query.is_empty() { inline_query } else { query };
        Req {
            method: method.to_ascii_uppercase(),
            segs: path.split('/').filter(|s| !s.is_empty()).map(|s| util::dec(s, false)).collect(),
            query: util::parse_query(q).into_iter().collect(),
            body,
        }
    }

    fn json(&self) -> Result<Value, ApiError> {
        if self.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&self.body).map_err(|_| ApiError::bad_request(msg!("The request body is not valid JSON")))
    }

    fn json_object(&self) -> Result<Value, ApiError> {
        match self.json()? {
            v @ Value::Object(_) => Ok(v),
            _ => Err(ApiError::bad_request(msg!("The request body must be a JSON object"))),
        }
    }
}

/// Routes a request. `user` is the QTS user (or `cli`).
pub fn route(req: &Req, store: &Store, user: &str) -> Result<Value, ApiError> {
    let s: Vec<&str> = req.segs.iter().map(String::as_str).collect();
    let m = req.method.as_str();
    let docker = Docker::from_env();
    let d = &docker;
    match (m, s.as_slice()) {
        ("GET", ["session"]) => Ok(session(store, d, user)),
        ("GET", ["instances"]) => list_instances(store, d),
        ("POST", ["instances"]) => create(req, store, d),
        ("GET", ["instances", name]) => instance::view_one(store, d, name),
        ("PUT", ["instances", name]) => update(req, store, d, name),
        ("DELETE", ["instances", name]) => delete(req, store, name),
        ("POST", ["instances", name, action @ ("start" | "stop" | "restart")]) => power(store, d, name, action),
        ("POST", ["instances", name, "apply"]) => start_job(store, name, "apply", Value::Null),
        ("POST", ["instances", name, "upgrade"]) => upgrade(req, store, name),
        ("POST", ["instances", name, "rollback"]) => rollback(store, d, name),
        ("DELETE", ["instances", name, "backup"]) => delete_backup(store, d, name),
        ("GET", ["instances", name, "logs"]) => logs(req, store, d, name),
        ("GET", ["instances", name, "stats"]) => stats(store, d, name),
        ("GET", ["discover"]) => discover_list(store, d),
        ("GET", ["networks"]) => Ok(json!({ "interfaces": net::interfaces()? })),
        ("POST", ["check", "ip"]) => check_ip(req, store, d),
        ("POST", ["check", "domain"]) => check_domain(req),
        ("GET", ["fs"]) => fs_list(req, store),
        ("POST", ["fs", "mkdir"]) => fs_mkdir(req, store),
        ("GET", ["updates"]) => {
            let u = store.updates();
            Ok(json!({ "checked_at": u.checked_at, "images": u.images }))
        }
        ("POST", ["updates", "check"]) => {
            let lock = jobs::lock_or_busy(store, jobs::UPDATES_KEY)?;
            Ok(json!({ "job_id": jobs::launch(store, lock, "check_updates", None, Value::Null)? }))
        }
        ("GET", ["jobs"]) => Ok(json!({ "jobs": jobs::list(store, req.query.get("active").is_some_and(|v| v == "1" || v == "true")) })),
        ("GET", ["jobs", id]) => {
            let offset = req.query.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
            jobs::view(store, id, offset).ok_or_else(|| ApiError::not_found(msg!("This task could not be found")))
        }
        ("GET", ["settings"]) => Ok(settings_json(&store.settings())),
        ("PUT", ["settings"]) => put_settings(req, store),
        _ if known_path(&s) => Err(ApiError::bad_request(msg!("This method is not supported here"))),
        _ => Err(ApiError::not_found(msg!("There is no such API"))),
    }
}

fn known_path(s: &[&str]) -> bool {
    matches!(
        s,
        ["session"]
            | ["instances"]
            | ["instances", _]
            | ["instances", _, "start" | "stop" | "restart" | "apply" | "upgrade" | "rollback" | "backup" | "logs" | "stats"]
            | ["discover"]
            | ["networks"]
            | ["check", "ip" | "domain"]
            | ["fs"]
            | ["fs", "mkdir"]
            | ["updates"]
            | ["updates", "check"]
            | ["jobs"]
            | ["jobs", _]
            | ["settings"]
    )
}

fn session(store: &Store, d: &Docker, user: &str) -> Value {
    let (ok, version, error) = match d.version() {
        Ok(v) => (true, v["Version"].as_str().map(str::to_string), None),
        Err(e) => (false, None, Some(e.msg())),
    };
    let tz = sys::timezone();
    json!({
        "user": user,
        "version": crate::VERSION,
        "docker": { "ok": ok, "version": version, "error": error.as_ref().map(Msg::english), "error_msg": error },
        "host": { "hostname": sys::hostname(), "timezone": tz },
        "defaults": { "image": store.settings().default_image, "restart": "always", "env": { "TZ": tz } },
    })
}

fn record_or_404(store: &Store, name: &str) -> Result<Record, ApiError> {
    store.record(name).ok_or_else(|| ApiError::not_found(msg!("This instance could not be found")))
}

fn list_instances(store: &Store, d: &Docker) -> Result<Value, ApiError> {
    let list = d.list_containers()?;
    let upd = store.updates();
    let mut images = Images::default();
    let out: Vec<Value> = store.records().iter().map(|r| instance::view(store, d, r, &list, &upd, &mut images)).collect();
    Ok(json!({ "instances": out }))
}

fn spec_ctx<'a>(interfaces: &'a [Interface], tz: &'a str, check: &'a dyn Fn(&str, bool) -> Result<String, Msg>) -> spec::Ctx<'a> {
    spec::Ctx { interfaces, tz, check_path: check }
}

/// MAC addresses in use by containers and instances other than `excluded`.
fn mac_taken(mac: &str, list: &[Listed], records: &[Record], excluded: &[String]) -> bool {
    list.iter().filter(|c| !excluded.contains(&c.name)).any(|c| c.endpoints().iter().any(|e| e.3 == mac))
        || records.iter().filter(|r| !excluded.contains(&r.spec.name)).any(|r| r.spec.network.mac == mac)
}

/// Names that do not count when an instance's address is checked: itself, its backup and the swap name.
fn own_names(name: &str) -> Vec<String> {
    vec![name.to_string(), spec::backup_name(name), format!("{name}.hbmgr-swap")]
}

/// MAC addresses that belong to the instance (its spec, its containers): an ARP answer from them is not a conflict.
fn own_macs(name: &str, spec_mac: &str, list: &[Listed]) -> Vec<String> {
    let names = own_names(name);
    let mut v: Vec<String> = list.iter().filter(|c| names.contains(&c.name)).flat_map(|c| c.endpoints().into_iter().map(|e| e.3)).collect();
    if !spec_mac.is_empty() {
        v.push(spec_mac.to_string());
    }
    v.retain(|m| !m.is_empty());
    v
}

fn ips_of(c: &Listed) -> Vec<String> {
    c.endpoints().into_iter().flat_map(|(_, conf, cur, _)| [conf, cur]).filter(|i| !i.is_empty()).collect()
}

/// The address checks for a static IP: another instance or container (refused), something on the LAN (refused
/// unless confirmed).
fn used_by_instance(ip: &str, name: &str) -> Msg {
    msg!("The address {ip} is already used by instance {name}", ip = ip, name = name)
}

fn used_by_container(ip: &str, name: &str) -> Msg {
    msg!("The address {ip} is already used by container {name}", ip = ip, name = name)
}

fn answers_on_lan(ip: &str, mac: &str) -> Msg {
    msg!("A device on the network already answers at {ip} ({mac})", ip = ip, mac = mac)
}

fn check_address(store: &Store, list: &[Listed], spec: &Spec, iface: &Interface, confirm_lan: bool) -> Result<(), ApiError> {
    let ip = &spec.network.ip;
    let excluded = own_names(&spec.name);
    let instances: Vec<(String, String)> =
        store.records().into_iter().filter(|r| r.spec.network.mode == "static").map(|r| (r.spec.name, r.spec.network.ip)).collect();
    let containers: Vec<(String, Vec<String>)> = list.iter().map(|c| (c.name.clone(), ips_of(c))).collect();
    let in_use = |by: &str, who: &str, msg: Msg| {
        ApiError::conflict(msg).with_details(json!({ "kind": "ip_in_use", "mac": null, "by": by, "name": who }))
    };
    match net::ip_user(ip, &excluded, &instances, &containers) {
        Some(IpUser::Instance(n)) => return Err(in_use("instance", &n, used_by_instance(ip, &n))),
        Some(IpUser::Container(n)) => return Err(in_use("container", &n, used_by_container(ip, &n))),
        None => {}
    }
    if confirm_lan {
        return Ok(());
    }
    if let Some(mac) = net::arp(&iface.arp_iface, ip)
        && !own_macs(&spec.name, &spec.network.mac, list).contains(&mac)
    {
        return Err(ApiError::conflict(answers_on_lan(ip, &mac))
            .with_details(json!({ "kind": "ip_in_use", "mac": mac, "by": "lan", "name": null })));
    }
    Ok(())
}

fn create(req: &Req, store: &Store, d: &Docker) -> Result<Value, ApiError> {
    let b = req.json_object()?;
    let replace = b["replace_existing"].as_bool().unwrap_or(false);
    let confirm = b["confirm_ip_in_use"].as_bool().unwrap_or(false);
    let ifaces = net::interfaces()?;
    let tz = sys::timezone();
    let own = paths::own_dirs(&store.root);
    let check = |p: &str, m: bool| paths::check_dir(p, m, &own);
    let mut spec = spec::parse(b.get("spec").unwrap_or(&Value::Null), &spec_ctx(&ifaces, &tz, &check)).map_err(ApiError::validation)?;
    let name = spec.name.clone();
    let lock = jobs::lock_or_busy(store, &instance_key(&name))?;
    if store.record_ignoring_case(&name).is_some() {
        return Err(ApiError::field("name", msg!("An instance with this name already exists")));
    }
    let list = d.list_containers()?;
    if let Some(c) = list.iter().find(|c| c.name == name)
        && !replace
    {
        return Err(ApiError::conflict(msg!("A container named {name} already exists", name = &name)).with_details(json!({
            "kind": "container_exists",
            "managed": instance::is_managed(c, &name),
            "image": c.image,
            "status": instance::status_name(&c.state),
        })));
    }
    let records = store.records();
    let excluded = own_names(&name);
    if spec.network.mac.is_empty() {
        spec.network.mac = spec::random_mac(&|m| mac_taken(m, &list, &records, &excluded));
    } else if mac_taken(&spec.network.mac, &list, &records, &excluded) {
        return Err(ApiError::field("network.mac", msg!("This MAC address is already used by another container")));
    }
    if spec.network.mode == "static" {
        let iface = ifaces.iter().find(|i| i.iface == spec.network.iface).expect("checked by parse");
        check_address(store, &list, &spec, iface, confirm)?;
    }
    let mut rec = Record { spec, ..Default::default() };
    store.save_record(&mut rec)?;
    match jobs::launch(store, lock, "create", Some(&name), json!({ "new_record": true, "replace_existing": replace })) {
        Ok(id) => Ok(json!({ "job_id": id })),
        Err(e) => {
            let _ = store.delete_record(&name);
            Err(e)
        }
    }
}

fn update(req: &Req, store: &Store, d: &Docker, name: &str) -> Result<Value, ApiError> {
    let mut rec = record_or_404(store, name)?;
    let b = req.json_object()?;
    let confirm = b["confirm_ip_in_use"].as_bool().unwrap_or(false);
    let spec_v = b.get("spec").unwrap_or(&Value::Null);
    if spec_v.get("name").and_then(Value::as_str).is_some_and(|n| n.trim() != name) {
        return Err(ApiError::field("name", msg!("The name cannot be changed after the instance is created")));
    }
    let mut spec_v = spec_v.clone();
    if let Some(o) = spec_v.as_object_mut() {
        o.insert("name".into(), json!(name));
        // The host name defaults to the domain only for a new instance: an edit without one keeps the container's,
        // so changing the domain never needs a recreate.
        if o.get("hostname").and_then(Value::as_str).is_none_or(|h| h.trim().is_empty()) {
            o.insert("hostname".into(), json!(rec.spec.hostname));
        }
    }
    let ifaces = net::interfaces()?;
    let tz = sys::timezone();
    let own = paths::own_dirs(&store.root);
    let check = |p: &str, m: bool| paths::check_dir(p, m, &own);
    let mut spec = spec::parse(&spec_v, &spec_ctx(&ifaces, &tz, &check)).map_err(ApiError::validation)?;
    let _lock = jobs::lock_or_busy(store, &instance_key(name))?;
    let list = d.list_containers()?;
    if spec.network.mac.is_empty() {
        spec.network.mac = rec.spec.network.mac.clone();
    } else if spec.network.mac != rec.spec.network.mac && mac_taken(&spec.network.mac, &list, &store.records(), &own_names(name)) {
        return Err(ApiError::field("network.mac", msg!("This MAC address is already used by another container")));
    }
    let old = &rec.spec.network;
    let address_changed = spec.network.mode != old.mode || spec.network.ip != old.ip || spec.network.iface != old.iface;
    if spec.network.mode == "static" && address_changed {
        let iface = ifaces.iter().find(|i| i.iface == spec.network.iface).expect("checked by parse");
        check_address(store, &list, &spec, iface, confirm)?;
    }
    rec.spec = spec;
    store.save_record(&mut rec)?;
    Ok(instance::view(store, d, &rec, &list, &store.updates(), &mut Images::default()))
}

fn delete(req: &Req, store: &Store, name: &str) -> Result<Value, ApiError> {
    record_or_404(store, name)?;
    match req.query.get("container").map(String::as_str).unwrap_or("keep") {
        "keep" => {
            let _lock = jobs::lock_or_busy(store, &instance_key(name))?;
            store.delete_record(name)?;
            Ok(json!({ "removed": true }))
        }
        "remove" => start_job(store, name, "remove", Value::Null),
        _ => Err(ApiError::field("container", msg!("The container option must be keep or remove"))),
    }
}

fn start_job(store: &Store, name: &str, kind: &str, params: Value) -> Result<Value, ApiError> {
    record_or_404(store, name)?;
    let lock = jobs::lock_or_busy(store, &instance_key(name))?;
    Ok(json!({ "job_id": jobs::launch(store, lock, kind, Some(name), params)? }))
}

fn upgrade(req: &Req, store: &Store, name: &str) -> Result<Value, ApiError> {
    let b = req.json()?;
    let image = match b.get("image") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => Some(spec::normalize_image(s.trim()).map_err(|m| ApiError::field("image", m))?),
        Some(_) => return Err(ApiError::field("image", msg!("This value must be text"))),
    };
    start_job(store, name, "upgrade", json!({ "image": image }))
}

fn rollback(store: &Store, d: &Docker, name: &str) -> Result<Value, ApiError> {
    record_or_404(store, name)?;
    if d.find(&spec::backup_name(name))?.is_none() {
        return Err(ApiError::conflict(msg!("There is no backup container to roll back to")));
    }
    start_job(store, name, "rollback", Value::Null)
}

fn power(store: &Store, d: &Docker, name: &str, action: &str) -> Result<Value, ApiError> {
    record_or_404(store, name)?;
    let _lock = jobs::lock_or_busy(store, &instance_key(name))?;
    let c = d
        .find(name)?
        .filter(|c| instance::is_managed(c, name))
        .ok_or_else(|| ApiError::conflict(msg!("This instance has no container of its own yet. Apply its settings first.")))?;
    match action {
        "start" => d.start(&c.id)?,
        "stop" => d.stop(&c.id)?,
        _ => d.restart(&c.id)?,
    }
    instance::view_one(store, d, name)
}

fn delete_backup(store: &Store, d: &Docker, name: &str) -> Result<Value, ApiError> {
    let mut rec = record_or_404(store, name)?;
    let _lock = jobs::lock_or_busy(store, &instance_key(name))?;
    if let Some(b) = d.find(&spec::backup_name(name))? {
        d.remove(&b.id)?;
    }
    rec.backup = None;
    store.save_record(&mut rec)?;
    instance::view_one(store, d, name)
}

fn logs(req: &Req, store: &Store, d: &Docker, name: &str) -> Result<Value, ApiError> {
    record_or_404(store, name)?;
    let tail = match req.query.get("tail") {
        None => 200,
        Some(t) => t
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=2000).contains(n))
            .ok_or_else(|| ApiError::field("tail", msg!("The number of lines must be between 1 and 2000")))?,
    };
    let Some(c) = d.find(name)? else { return Ok(json!({ "lines": [], "truncated": false })) };
    let tty = d.inspect(&c.id)?.is_some_and(|i| i["Config"]["Tty"].as_bool() == Some(true));
    let mut lines = crate::docker::log_lines(&d.logs(&c.id, tail + 1, tty)?);
    let truncated = lines.len() > tail;
    if truncated {
        lines.drain(..lines.len() - tail);
    }
    Ok(json!({ "lines": lines, "truncated": truncated }))
}

fn stats(store: &Store, d: &Docker, name: &str) -> Result<Value, ApiError> {
    record_or_404(store, name)?;
    let none = json!({ "cpu_percent": null, "mem_bytes": null, "mem_limit_bytes": null });
    let Some(c) = d.find(name)?.filter(|c| instance::is_managed(c, name) && c.state == "running") else { return Ok(none) };
    let (cpu, mem, limit) = crate::docker::stats_summary(&d.stats(&c.id)?);
    Ok(json!({ "cpu_percent": cpu, "mem_bytes": mem, "mem_limit_bytes": limit }))
}

fn discover_list(store: &Store, d: &Docker) -> Result<Value, ApiError> {
    let list = d.list_containers()?;
    let records = store.records();
    let nets = d.all_networks()?;
    let ifaces = net::interfaces().unwrap_or_default();
    let tz = sys::timezone();
    let mut out = Vec::new();
    for c in &list {
        if c.name.contains(".hbmgr-") || records.iter().any(|r| r.spec.name.eq_ignore_ascii_case(&c.name)) {
            continue;
        }
        let Some(i) = d.inspect(&c.id)? else { continue };
        if !discover::looks_like_homebridge(&i) {
            continue;
        }
        let (spec, notes) = discover::to_spec(&i, &tasks::image_env(d, &i), &nets, &ifaces, &tz);
        let ip = instance::endpoint(&i).map(|e| e.1).filter(|ip| !ip.is_empty());
        out.push(json!({
            "name": c.name,
            "image": i["Config"]["Image"],
            "status": instance::status_name(i["State"]["Status"].as_str().unwrap_or("")),
            "ip": ip,
            "spec": spec,
            "notes": notes,
        }));
    }
    Ok(json!({ "containers": out }))
}

fn check_ip(req: &Req, store: &Store, d: &Docker) -> Result<Value, ApiError> {
    let b = req.json_object()?;
    let ifaces = net::interfaces()?;
    let iface_name = b["iface"].as_str().unwrap_or("");
    let iface =
        ifaces.iter().find(|i| i.iface == iface_name).ok_or_else(|| ApiError::field("iface", msg!("Choose a network interface")))?;
    let ip = b["ip"].as_str().unwrap_or("").trim().to_string();
    let exclude = b["exclude"].as_str().unwrap_or("").trim().to_string();
    let answer = |valid: bool, reason: Option<&str>, message: Msg, mac: Option<String>, by: Option<String>| json!({ "valid": valid, "available": valid && reason.is_none(), "reason": reason, "message": message, "mac": mac, "by": by });
    if let Some(problem) = spec::ip_problem(&ip, iface) {
        return Ok(answer(false, None, problem, None, None));
    }
    let ip = ip.parse::<Ipv4Addr>().map(|a| a.to_string()).unwrap_or(ip);
    let list = d.list_containers()?;
    let records = store.records();
    let excluded = if exclude.is_empty() { Vec::new() } else { own_names(&exclude) };
    let instances: Vec<(String, String)> =
        records.iter().filter(|r| r.spec.network.mode == "static").map(|r| (r.spec.name.clone(), r.spec.network.ip.clone())).collect();
    let containers: Vec<(String, Vec<String>)> = list.iter().map(|c| (c.name.clone(), ips_of(c))).collect();
    match net::ip_user(&ip, &excluded, &instances, &containers) {
        Some(IpUser::Instance(n)) => {
            let mac = records.iter().find(|r| r.spec.name == n).map(|r| r.spec.network.mac.clone()).filter(|m| !m.is_empty());
            return Ok(answer(true, Some("instance"), used_by_instance(&ip, &n), mac, Some(n)));
        }
        Some(IpUser::Container(n)) => {
            let mac = list.iter().find(|c| c.name == n).and_then(|c| c.endpoints().into_iter().map(|e| e.3).find(|m| !m.is_empty()));
            return Ok(answer(true, Some("container"), used_by_container(&ip, &n), mac, Some(n)));
        }
        None => {}
    }
    if let Some(mac) = net::arp(&iface.arp_iface, &ip) {
        let spec_mac = records.iter().find(|r| r.spec.name == exclude).map(|r| r.spec.network.mac.clone()).unwrap_or_default();
        let mine = !exclude.is_empty() && own_macs(&exclude, &spec_mac, &list).contains(&mac);
        if !mine {
            let by = list
                .iter()
                .find(|c| c.endpoints().iter().any(|e| e.3 == mac))
                .map(|c| c.name.clone())
                .or_else(|| records.iter().find(|r| r.spec.network.mac == mac).map(|r| r.spec.name.clone()));
            return Ok(answer(true, Some("lan"), answers_on_lan(&ip, &mac), Some(mac), by));
        }
    }
    Ok(answer(true, None, msg!("This address is available"), None, None))
}

fn check_domain(req: &Req) -> Result<Value, ApiError> {
    let b = req.json_object()?;
    let domain = b["domain"].as_str().unwrap_or("").trim().trim_end_matches('.').to_ascii_lowercase();
    let ip = match b.get("ip") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => {
            Some(s.trim().parse::<Ipv4Addr>().map_err(|_| ApiError::field("ip", msg!("The IP address is not valid")))?)
        }
        Some(_) => return Err(ApiError::field("ip", msg!("The IP address is not valid"))),
    };
    if !spec::valid_domain(&domain) {
        let message = msg!("The domain name is not valid (for example hb.example.com)");
        return Ok(json!({ "valid": false, "resolves": [], "matches": null, "message": message }));
    }
    Ok(net::domain_answer(ip, net::resolve_v4(&domain, Duration::from_secs(3))))
}

fn fs_list(req: &Req, store: &Store) -> Result<Value, ApiError> {
    let path = req.query.get("path").map(String::as_str).unwrap_or("");
    paths::list(path, &paths::own_dirs(&store.root)).map_err(|m| ApiError::field("path", m))
}

fn fs_mkdir(req: &Req, store: &Store) -> Result<Value, ApiError> {
    let b = req.json_object()?;
    let path = b["path"].as_str().unwrap_or("");
    let p = paths::mkdir(path, &paths::own_dirs(&store.root)).map_err(|m| ApiError::field("path", m))?;
    Ok(json!({ "path": p }))
}

fn settings_json(s: &Settings) -> Value {
    json!({ "default_image": s.default_image, "default_mounts": s.default_mounts })
}

fn put_settings(req: &Req, store: &Store) -> Result<Value, ApiError> {
    let b = req.json_object()?;
    let mut fields = Fields::new();
    let image = b["default_image"].as_str().unwrap_or("").trim();
    let image = match spec::normalize_image(if image.is_empty() { spec::DEFAULT_IMAGE } else { image }) {
        Ok(i) => i,
        Err(m) => {
            fields.insert("default_image".into(), m);
            String::new()
        }
    };
    let own = paths::own_dirs(&store.root);
    let check = |p: &str, m: bool| paths::check_dir(p, m, &own);
    let ctx = spec_ctx(&[], "", &check);
    let mounts = spec::parse_mounts(b.get("default_mounts"), "default_mounts", &ctx, &mut fields);
    if !fields.is_empty() {
        return Err(ApiError::validation(fields));
    }
    let s = Settings { default_image: image, default_mounts: mounts };
    store.save_settings(&s)?;
    Ok(settings_json(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;

    fn req(method: &str, path: &str, query: &str, body: &str) -> Req {
        Req::new(method, path, query, body.as_bytes().to_vec())
    }

    #[test]
    fn parses_requests() {
        let r = req("get", "/instances/homebridge-1F/logs", "tail=20", "");
        assert_eq!(r.method, "GET");
        assert_eq!(r.segs, ["instances", "homebridge-1F", "logs"]);
        assert_eq!(r.query["tail"], "20");
        let inline = req("GET", "/fs?path=%2Fshare%2FDocker", "", "");
        assert_eq!(inline.segs, ["fs"]);
        assert_eq!(inline.query["path"], "/share/Docker");
        assert_eq!(req("POST", "/x", "", "  ").json().unwrap(), Value::Null);
        assert_eq!(req("POST", "/x", "", "{bad").json().unwrap_err().code, "bad_request");
        assert_eq!(req("POST", "/x", "", "[1]").json_object().unwrap_err().code, "bad_request");
    }

    #[test]
    fn routes_without_docker() {
        let store = temp_store("route");
        // SAFETY: tests that touch the environment run in this module only and use a socket that does not exist.
        unsafe {
            std::env::set_var("HBMGR_DOCKER_SOCK", store.root.join("no.sock"));
            std::env::set_var("HBMGR_CS_DOCKER_SOCK", "");
        }
        assert_eq!(route(&req("GET", "/nothing", "", ""), &store, "cli").unwrap_err().code, "not_found");
        assert_eq!(route(&req("DELETE", "/session", "", ""), &store, "cli").unwrap_err().code, "bad_request");
        assert_eq!(route(&req("GET", "/instances", "", ""), &store, "cli").unwrap_err().code, "docker_unavailable");
        assert_eq!(route(&req("GET", "/instances/none", "", ""), &store, "cli").unwrap_err().code, "not_found");
        assert_eq!(route(&req("PUT", "/instances/none", "", "{}"), &store, "cli").unwrap_err().code, "not_found");
        let s = route(&req("GET", "/session", "", ""), &store, "admin").unwrap();
        assert_eq!(s["user"], "admin");
        assert_eq!(s["docker"]["ok"], false);
        assert_eq!(s["defaults"]["image"], spec::DEFAULT_IMAGE);
        assert_eq!(
            route(&req("GET", "/settings", "", ""), &store, "cli").unwrap(),
            json!({ "default_image": spec::DEFAULT_IMAGE, "default_mounts": [] })
        );
        let bad = route(
            &req("PUT", "/settings", "", r#"{"default_image":"Bad Image","default_mounts":[{"host":"/tmp","container":"x"}]}"#),
            &store,
            "cli",
        )
        .unwrap_err();
        let f = bad.fields.unwrap();
        assert!(f.contains_key("default_image") && f.contains_key("default_mounts.0.host") && f.contains_key("default_mounts.0.container"));
        let saved = route(&req("PUT", "/settings", "", r#"{"default_image":"oznu/homebridge"}"#), &store, "cli").unwrap();
        assert_eq!(saved["default_image"], "oznu/homebridge:latest");
        assert_eq!(route(&req("GET", "/updates", "", ""), &store, "cli").unwrap(), json!({ "checked_at": null, "images": [] }));
        assert_eq!(route(&req("GET", "/jobs", "active=1", ""), &store, "cli").unwrap(), json!({ "jobs": [] }));
        assert_eq!(route(&req("GET", "/jobs/20261112-112000-a1b2c3", "", ""), &store, "cli").unwrap_err().code, "not_found");
        let d = route(&req("POST", "/check/domain", "", r#"{"domain":"not a domain"}"#), &store, "cli").unwrap();
        assert_eq!(d["valid"], false);
        assert_eq!(
            route(&req("POST", "/check/domain", "", r#"{"domain":"a.example.com","ip":"x"}"#), &store, "cli").unwrap_err().code,
            "validation"
        );
        assert_eq!(route(&req("GET", "/fs", "path=/etc", ""), &store, "cli").unwrap_err().code, "validation");
        let _ = std::fs::remove_dir_all(&store.root);
    }
}

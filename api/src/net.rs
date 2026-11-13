//! Networks: the NAS interfaces (from Container Station's `qnetwork-tool`), IPv4 subnet maths, the qnet networks the
//! containers join, and the address and domain checks.

use crate::error::ApiError;
use serde::Serialize;
use serde_json::{Value, json};
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// A NAS interface a container can be put on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Interface {
    /// The interface qnet attaches to (`bond0`, `eth0`).
    pub iface: String,
    pub display: String,
    pub subnet: Option<String>,
    pub gateway: Option<String>,
    pub host_ip: Option<String>,
    pub default: bool,
    /// Where the NAS itself is on that LAN (the virtual switch `qvs0`, or the interface): used for ARP probes.
    #[serde(skip)]
    pub arp_iface: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Subnet {
    pub addr: u32,
    pub prefix: u8,
}

impl Subnet {
    /// `192.168.8.0/22`; host bits are cleared.
    pub fn parse(s: &str) -> Option<Subnet> {
        let (a, p) = s.trim().split_once('/')?;
        let addr: Ipv4Addr = a.parse().ok()?;
        let prefix: u8 = p.parse().ok().filter(|p| *p <= 32)?;
        Some(Subnet { addr: u32::from(addr) & mask(prefix), prefix })
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & mask(self.prefix) == self.addr
    }

    pub fn network(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.addr)
    }

    pub fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.addr | !mask(self.prefix))
    }
}

impl std::fmt::Display for Subnet {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}/{}", self.network(), self.prefix)
    }
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix)) }
}

/// `192.168.8.1/22` → `192.168.8.1`; empty → `None`.
fn addr_part(s: Option<&str>) -> Option<String> {
    let a = s?.split('/').next()?.trim();
    a.parse::<Ipv4Addr>().ok().map(|a| a.to_string())
}

/// `192.168.8.0/22` normalised (host bits cleared), or `None`.
fn subnet_part(s: Option<&str>) -> Option<String> {
    Subnet::parse(s?).map(|s| s.to_string())
}

/// Interfaces from `qnetwork-tool vswitch list`: one per physical interface of a virtual switch.
pub fn parse_vswitch(v: &Value) -> Vec<Interface> {
    let mut out = Vec::new();
    for sw in v["switches"].as_array().into_iter().flatten() {
        let Some(ifaces) = sw["ifaces"].as_array().filter(|a| !a.is_empty()) else { continue };
        let sw_name = sw["name"].as_str().unwrap_or("");
        let sw_display = sw["display_name"].as_str().unwrap_or(sw_name);
        for i in ifaces {
            let Some(name) = i["name"].as_str().filter(|n| !n.is_empty()) else { continue };
            let i_display = i["display_name"].as_str().unwrap_or(name);
            out.push(Interface {
                iface: name.to_string(),
                display: format!("{i_display} ({sw_display})"),
                subnet: subnet_part(sw["subnet"].as_str()),
                gateway: addr_part(sw["gateway"].as_str()),
                host_ip: addr_part(sw["ipv4"].as_str()),
                default: sw["default_gateway"].as_bool().unwrap_or(false),
                arp_iface: if sw_name.is_empty() { name.to_string() } else { sw_name.to_string() },
            });
        }
    }
    mark_default(out)
}

/// Interfaces from `qnetwork-tool ethernet list` (models without virtual switches).
pub fn parse_ethernet(v: &Value) -> Vec<Interface> {
    let mut out = Vec::new();
    for i in v["ifaces"].as_array().into_iter().flatten() {
        let Some(name) = i["name"].as_str().filter(|n| !n.is_empty()) else { continue };
        let Some(host_ip) = addr_part(i["ipv4"].as_str()) else { continue };
        let display = i["display_name"].as_str().unwrap_or(name);
        let sw = i["switch_name"].as_str().unwrap_or("");
        let display = match i["switch_display_name"].as_str().filter(|s| !s.is_empty()) {
            Some(sd) => format!("{display} ({sd})"),
            None => display.to_string(),
        };
        out.push(Interface {
            iface: name.to_string(),
            display,
            subnet: subnet_part(i["subnet"].as_str()),
            gateway: addr_part(i["gateway"].as_str()),
            host_ip: Some(host_ip),
            default: i["default_gateway"].as_bool().unwrap_or(false),
            arp_iface: if sw.is_empty() { name.to_string() } else { sw.to_string() },
        });
    }
    mark_default(out)
}

fn mark_default(mut v: Vec<Interface>) -> Vec<Interface> {
    let mut seen = false;
    for i in v.iter_mut() {
        if i.default && !seen {
            seen = true;
        } else {
            i.default = false;
        }
    }
    if !seen && let Some(first) = v.first_mut() {
        first.default = true;
    }
    v
}

/// Container Station's `qnetwork-tool`.
fn qnetwork_tool() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HBMGR_QNETWORK_TOOL") {
        return Some(PathBuf::from(p));
    }
    let root = crate::sys::getcfg("container-station", "Install_Path", "/etc/config/qpkg.conf")?;
    let p = PathBuf::from(root).join("bin/qnetwork-tool");
    p.is_file().then_some(p)
}

/// Runs `qnetwork-tool` with `args` and parses its JSON output; `None` when it is missing or fails.
fn qnetwork(args: &[&str], limit: Duration) -> Option<Value> {
    let tool = qnetwork_tool()?;
    let out = crate::sys::run(&tool, args, limit)?;
    serde_json::from_slice(&out).ok()
}

/// The interfaces containers can use.
pub fn interfaces() -> Result<Vec<Interface>, ApiError> {
    if qnetwork_tool().is_none() {
        return Err(ApiError::new(
            503,
            "docker_unavailable",
            msg!("Container Station's network tool was not found. Make sure Container Station is installed and running."),
        ));
    }
    let limit = Duration::from_secs(10);
    let mut list = qnetwork(&["vswitch", "list"], limit).map(|v| parse_vswitch(&v)).unwrap_or_default();
    if list.is_empty() {
        list = qnetwork(&["ethernet", "list"], limit).map(|v| parse_ethernet(&v)).unwrap_or_default();
    }
    if list.is_empty() {
        return Err(ApiError::internal(msg!("The NAS network interfaces could not be read")));
    }
    Ok(list)
}

/// Asks the LAN who has `ip` (an ARP probe from the NAS, up to about 2 s). The MAC address of the answer, if any.
pub fn arp(arp_iface: &str, ip: &str) -> Option<String> {
    let v = qnetwork(&["arp", "--iface", arp_iface, "--ip", ip, "--format", "json", "--timeout", "2"], Duration::from_secs(6))?;
    parse_arp(&v, ip)
}

pub fn parse_arp(v: &Value, ip: &str) -> Option<String> {
    v["machines"]
        .as_array()?
        .iter()
        .find(|m| m["ip"].as_str() == Some(ip))
        .and_then(|m| m["mac"].as_str())
        .and_then(crate::spec::normalize_mac)
}

/// The qnet network a container joins: an existing one on the same interface (and, for static addresses, the same
/// subnet), else `None`. The IPAM options carry the interface (networks created by older Container Stations have no
/// driver options).
pub fn pick_network(nets: &[Value], mode: &str, iface: &str, subnet: Option<&str>) -> Option<String> {
    let want = if mode == "static" { subnet.and_then(Subnet::parse) } else { None };
    if mode == "static" && want.is_none() {
        return None;
    }
    let mut found: Vec<String> = nets
        .iter()
        .filter(|n| n["Driver"].as_str() == Some("qnet"))
        .filter(|n| {
            let on = n["IPAM"]["Options"]["iface"].as_str().or_else(|| n["Options"]["iface"].as_str());
            on == Some(iface)
        })
        .filter(|n| {
            let subnets: Vec<Option<Subnet>> = n["IPAM"]["Config"]
                .as_array()
                .map(|a| a.iter().map(|c| c["Subnet"].as_str().and_then(Subnet::parse)).collect())
                .unwrap_or_default();
            match want {
                Some(w) => subnets.contains(&Some(w)),
                None => subnets.is_empty() || subnets.iter().any(|s| s.is_some_and(|s| s.prefix == 0)),
            }
        })
        .filter_map(|n| n["Name"].as_str().map(str::to_string))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Name and `POST /networks/create` body of the qnet network this package creates when none fits.
pub fn network_create(mode: &str, iface: &Interface) -> (String, Value) {
    let name = format!("qnet-{}-{}-hbmgr", if mode == "static" { "static" } else { "dhcp" }, iface.iface);
    let config = if mode == "static" {
        let mut c = serde_json::Map::new();
        c.insert("Subnet".into(), json!(iface.subnet.clone().unwrap_or_default()));
        if let Some(g) = &iface.gateway {
            c.insert("Gateway".into(), json!(g));
        }
        json!([c])
    } else {
        json!([])
    };
    let body = json!({
        "Name": name,
        "Driver": "qnet",
        "CheckDuplicate": true,
        "Options": { "iface": iface.iface },
        "IPAM": { "Driver": "qnet", "Options": { "iface": iface.iface }, "Config": config },
        "Labels": { "hbmgr.created": "1" },
    });
    (name, body)
}

/// Who already uses an address.
#[derive(Debug, Clone, PartialEq)]
pub enum IpUser {
    Instance(String),
    Container(String),
}

/// Another instance whose spec has `ip`, or a container configured with or holding `ip`. Names in `exclude` (the
/// instance being edited, the container being replaced and its backup) do not count.
pub fn ip_user(ip: &str, exclude: &[String], instances: &[(String, String)], containers: &[(String, Vec<String>)]) -> Option<IpUser> {
    if let Some((n, _)) = instances.iter().find(|(n, i)| i == ip && !exclude.contains(n)) {
        return Some(IpUser::Instance(n.clone()));
    }
    containers.iter().find(|(n, ips)| !exclude.contains(n) && ips.iter().any(|i| i == ip)).map(|(n, _)| IpUser::Container(n.clone()))
}

/// Resolves a host name's IPv4 addresses, giving up after `limit` (the lookup runs on its own thread, so a stuck
/// resolver cannot hold the request). `None` when the time ran out.
pub fn resolve_v4(name: &str, limit: Duration) -> Option<Vec<Ipv4Addr>> {
    let (tx, rx) = mpsc::channel();
    let host = name.to_string();
    std::thread::spawn(move || {
        let addrs: Vec<Ipv4Addr> = (host.as_str(), 0u16)
            .to_socket_addrs()
            .map(|it| {
                it.filter_map(|a| match a {
                    SocketAddr::V4(v4) => Some(*v4.ip()),
                    SocketAddr::V6(_) => None,
                })
                .collect()
            })
            .unwrap_or_default();
        let _ = tx.send(addrs);
    });
    let mut v = rx.recv_timeout(limit).ok()?;
    v.sort();
    v.dedup();
    Some(v)
}

/// The answer of `POST /check/domain`. `resolved` is `None` when the lookup timed out.
pub fn domain_answer(ip: Option<Ipv4Addr>, resolved: Option<Vec<Ipv4Addr>>) -> Value {
    let Some(addrs) = resolved else {
        let message = msg!("Looking up the domain name took too long. Check the DNS settings of the NAS and try again.");
        return json!({ "valid": true, "resolves": [], "matches": ip.map(|_| false), "message": message });
    };
    let list = addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let matches = ip.map(|ip| addrs.contains(&ip));
    let addresses = list.join(", ");
    let message = match (addrs.is_empty(), ip, matches) {
        (true, Some(ip), _) => {
            msg!("The domain name does not resolve. Add a DNS A record that points to {ip}.", ip = ip.to_string())
        }
        (true, None, _) => msg!("The domain name does not resolve. Add a DNS A record that points to the address of this Homebridge."),
        (false, Some(_), Some(true)) => msg!("The domain name points to this Homebridge"),
        (false, Some(ip), _) => msg!(
            "The domain name points to {addresses}, not to this Homebridge ({ip}). Change its DNS A record.",
            addresses = &addresses,
            ip = ip.to_string()
        ),
        (false, None, _) => msg!("The domain name points to {addresses}", addresses = &addresses),
    };
    json!({ "valid": true, "resolves": list, "matches": matches, "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_maths() {
        let s = Subnet::parse("192.168.8.0/22").unwrap();
        assert_eq!(s.network(), Ipv4Addr::new(192, 168, 8, 0));
        assert_eq!(s.broadcast(), Ipv4Addr::new(192, 168, 11, 255));
        assert!(s.contains(Ipv4Addr::new(192, 168, 10, 7)));
        assert!(!s.contains(Ipv4Addr::new(192, 168, 12, 0)));
        assert_eq!(Subnet::parse("192.168.10.7/22").unwrap().to_string(), "192.168.8.0/22");
        assert_eq!(Subnet::parse("0.0.0.0/0").unwrap().prefix, 0);
        assert!(Subnet::parse("0.0.0.0/0").unwrap().contains(Ipv4Addr::new(8, 8, 8, 8)));
        assert_eq!(Subnet::parse("10.0.0.0/33"), None);
        assert_eq!(Subnet::parse("10.0.0.0"), None);
    }

    const VSWITCH: &str = r#"{"switches":[{"name":"docker0","display_name":"Container Network (docker0)","gateway":"10.0.5.254/24","subnet":"10.0.5.0/24","ipv4":"10.0.5.1/24","default_gateway":false,"dhcp":true},{"name":"qvs0","display_name":"Virtual Switch 3","gateway":"192.168.8.1/22","subnet":"192.168.8.0/22","ipv4":"192.168.11.1/22","default_gateway":true,"ifaces":[{"name":"bond0","display_name":"Adapter 1+2","speed":[2500,2500]}],"dhcp":false}]}"#;

    #[test]
    fn reads_virtual_switches() {
        let list = parse_vswitch(&serde_json::from_str(VSWITCH).unwrap());
        assert_eq!(
            list,
            vec![Interface {
                iface: "bond0".into(),
                display: "Adapter 1+2 (Virtual Switch 3)".into(),
                subnet: Some("192.168.8.0/22".into()),
                gateway: Some("192.168.8.1".into()),
                host_ip: Some("192.168.11.1".into()),
                default: true,
                arp_iface: "qvs0".into(),
            }]
        );
        assert_eq!(serde_json::to_value(&list[0]).unwrap().get("arp_iface"), None);
    }

    #[test]
    fn reads_physical_interfaces() {
        let v = serde_json::json!({"ifaces":[{"name":"eth0","display_name":"Adapter 1","gateway":"192.168.1.1/24","subnet":"192.168.1.0/24","ipv4":"192.168.1.20/24","switch_name":"","default_gateway":false},{"name":"eth1","display_name":"Adapter 2","ipv4":""}]});
        let list = parse_ethernet(&v);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].arp_iface, "eth0");
        assert!(list[0].default, "the first one is the default when none is marked");
    }

    #[test]
    fn reads_arp_answers() {
        let v = serde_json::json!({"machines":[{"ip":"192.168.10.5","mac":"02:42:3C:91:D4:58"}]});
        assert_eq!(parse_arp(&v, "192.168.10.5").as_deref(), Some("02:42:3c:91:d4:58"));
        let silent = serde_json::json!({"machines":[{"ip":"192.168.11.233","mac":""}]});
        assert_eq!(parse_arp(&silent, "192.168.11.233"), None);
    }

    fn nets() -> Vec<Value> {
        serde_json::from_str(
            r#"[
            {"Name":"bridge","Driver":"bridge","IPAM":{"Driver":"default","Options":null,"Config":[{"Subnet":"10.0.3.0/24"}]},"Options":{}},
            {"Name":"qnet-static-bond0-3f9a2c","Driver":"qnet","IPAM":{"Driver":"qnet","Options":{"iface":"bond0"},"Config":[{"Subnet":"192.168.8.0/22","Gateway":"192.168.8.1"}]},"Options":{}},
            {"Name":"qnet-dhcp-bond0-hbmgr","Driver":"qnet","IPAM":{"Driver":"qnet","Options":{"iface":"bond0"},"Config":[{"Subnet":"0.0.0.0/0","Gateway":"192.168.8.1"}]},"Options":{"iface":"bond0"}},
            {"Name":"qnet-static-eth1-x","Driver":"qnet","IPAM":{"Driver":"qnet","Options":{"iface":"eth1"},"Config":[{"Subnet":"192.168.8.0/22"}]},"Options":{}}
        ]"#,
        )
        .unwrap()
    }

    #[test]
    fn picks_qnet_networks() {
        let n = nets();
        assert_eq!(pick_network(&n, "static", "bond0", Some("192.168.8.0/22")).as_deref(), Some("qnet-static-bond0-3f9a2c"));
        assert_eq!(pick_network(&n, "static", "bond0", Some("192.168.0.0/24")), None);
        assert_eq!(pick_network(&n, "dhcp", "bond0", None).as_deref(), Some("qnet-dhcp-bond0-hbmgr"));
        assert_eq!(pick_network(&n, "dhcp", "eth1", None), None);
        assert_eq!(pick_network(&n, "static", "eth9", Some("192.168.8.0/22")), None);
    }

    #[test]
    fn builds_qnet_networks() {
        let i = crate::spec::tests::iface();
        let (name, body) = network_create("dhcp", &i);
        assert_eq!(name, "qnet-dhcp-bond0-hbmgr");
        assert_eq!(body["IPAM"]["Options"]["iface"], "bond0");
        assert_eq!(body["Options"]["iface"], "bond0");
        assert_eq!(body["IPAM"]["Config"], serde_json::json!([]));
        let (name, body) = network_create("static", &i);
        assert_eq!(name, "qnet-static-bond0-hbmgr");
        assert_eq!(body["IPAM"]["Config"], serde_json::json!([{ "Subnet": "192.168.8.0/22", "Gateway": "192.168.8.1" }]));
    }

    #[test]
    fn finds_who_uses_an_address() {
        let instances = vec![("hb-a".to_string(), "192.168.10.20".to_string())];
        let containers =
            vec![("homebridge".to_string(), vec!["192.168.10.5".to_string()]), ("hb-a".to_string(), vec!["192.168.10.20".to_string()])];
        assert_eq!(ip_user("192.168.10.20", &[], &instances, &containers), Some(IpUser::Instance("hb-a".into())));
        assert_eq!(ip_user("192.168.10.20", &["hb-a".into()], &instances, &containers), None);
        assert_eq!(ip_user("192.168.10.5", &[], &instances, &containers), Some(IpUser::Container("homebridge".into())));
        assert_eq!(ip_user("192.168.10.5", &["homebridge".into()], &instances, &containers), None);
        assert_eq!(ip_user("192.168.10.99", &[], &instances, &containers), None);
    }

    #[test]
    fn domain_answers() {
        let ip = Some(Ipv4Addr::new(192, 168, 10, 7));
        let a = domain_answer(ip, Some(vec![Ipv4Addr::new(192, 168, 10, 7)]));
        assert_eq!(
            a,
            serde_json::json!({ "valid": true, "resolves": ["192.168.10.7"], "matches": true, "message": { "key": "The domain name points to this Homebridge" } })
        );
        let a = domain_answer(ip, Some(vec![]));
        assert_eq!(a["matches"], false);
        assert_eq!(
            a["message"],
            serde_json::json!({ "key": "The domain name does not resolve. Add a DNS A record that points to {ip}.", "args": { "ip": "192.168.10.7" } })
        );
        let a = domain_answer(ip, Some(vec![Ipv4Addr::new(192, 168, 10, 8)]));
        assert_eq!(a["matches"], false);
        assert_eq!(a["message"]["args"]["addresses"], "192.168.10.8");
        let a = domain_answer(None, Some(vec![Ipv4Addr::new(192, 168, 10, 8)]));
        assert_eq!(a["matches"], Value::Null);
        let a = domain_answer(None, None);
        assert_eq!(a["resolves"], serde_json::json!([]));
    }

    #[test]
    fn resolves_names_with_a_time_limit() {
        let v = resolve_v4("localhost", Duration::from_secs(3)).unwrap();
        assert!(v.contains(&Ipv4Addr::LOCALHOST));
        assert_eq!(resolve_v4("name.invalid", Duration::from_secs(3)).unwrap_or_default(), Vec::<Ipv4Addr>::new());
    }
}

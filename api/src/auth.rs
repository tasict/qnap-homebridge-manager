//! QTS sessions: the `NAS_SID` cookie is checked with QTS's `authLogin.cgi` on the loopback CGI server (`_thttpd_`
//! on 127.0.0.1:58080, which never redirects to HTTPS). Passes are cached for 30 s in `run/sessions/`, under a hash
//! of the sid, readable by root only.

use crate::error::ApiError;
use crate::http::{self, Body};
use crate::store::Store;
use crate::util::{self, write_atomic};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const QTS: &str = "127.0.0.1:58080";
const CACHE_SECS: i64 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Pass(User),
    /// QTS says the session is not valid.
    Denied,
    /// QTS gave no usable answer.
    Unavailable,
}

/// The text of the first `<tag>` element, with a CDATA wrapper removed.
fn xml_leaf(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    let mut v = xml[start..end].trim();
    if let Some(inner) = v.strip_prefix("<![CDATA[").and_then(|s| s.strip_suffix("]]>")) {
        v = inner.trim();
    }
    Some(v.to_string())
}

/// Reads authLogin.cgi's answer to `?sid=`. Only an explicit `authPassed` other than 1 ends a session.
pub fn verdict(body: &str) -> Verdict {
    match xml_leaf(body, "authPassed").as_deref() {
        Some("1") => {
            let name = xml_leaf(body, "username").filter(|s| !s.is_empty()).or_else(|| xml_leaf(body, "user")).unwrap_or_default();
            if name.is_empty() {
                return Verdict::Unavailable;
            }
            Verdict::Pass(User { name, admin: xml_leaf(body, "isAdmin").as_deref() == Some("1") })
        }
        Some(_) => Verdict::Denied,
        None => Verdict::Unavailable,
    }
}

/// `NAS_SID` from a `Cookie` header; only letters and digits are accepted.
pub fn cookie_sid(cookie: &str) -> Option<String> {
    cookie
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == "NAS_SID")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty() && v.len() <= 128 && v.bytes().all(|c| c.is_ascii_alphanumeric()))
}

fn ask_qts(sid: &str, client_ip: &str) -> Verdict {
    let addr: SocketAddr = QTS.parse().expect("valid address");
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(3)) else { return Verdict::Unavailable };
    let _ = s.set_read_timeout(Some(Duration::from_secs(8)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    let path = format!("/cgi-bin/authLogin.cgi?sid={}", util::enc(sid));
    let xff = if client_ip.parse::<std::net::IpAddr>().is_ok() { client_ip } else { "127.0.0.1" };
    let req = http::request("GET", &path, "127.0.0.1", &[("X-Forwarded-For", xff)], None, true);
    if s.write_all(&req).is_err() {
        return Verdict::Unavailable;
    }
    let mut r = BufReader::new(s);
    let Ok(head) = http::read_head(&mut r) else { return Verdict::Unavailable };
    if !(200..300).contains(&head.status) {
        return Verdict::Unavailable;
    }
    match Body::new(&head, r).read_all(1 << 20) {
        Ok(b) => verdict(&String::from_utf8_lossy(&b)),
        Err(_) => Verdict::Unavailable,
    }
}

fn cache_file(store: &Store, sid: &str) -> std::path::PathBuf {
    let mut h = Sha256::new();
    h.update(b"hbmgr-sid\n");
    h.update(sid.as_bytes());
    store.sessions_dir().join(format!("{}.json", util::hex(&h.finalize())))
}

fn cached(store: &Store, sid: &str, now: i64) -> Option<User> {
    let v: Value = serde_json::from_slice(&std::fs::read(cache_file(store, sid)).ok()?).ok()?;
    let at = v["at"].as_i64()?;
    if at > now || now - at >= CACHE_SECS {
        return None;
    }
    Some(User { name: v["user"].as_str()?.to_string(), admin: v["admin"].as_bool()? })
}

/// Drops cached passes older than ten minutes.
fn prune(store: &Store) {
    for e in std::fs::read_dir(store.sessions_dir()).into_iter().flatten().flatten() {
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 600);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The administrator behind `cookie`, or the error to answer with.
pub fn check(store: &Store, cookie: &str, client_ip: &str) -> Result<User, ApiError> {
    let Some(sid) = cookie_sid(cookie) else {
        return Err(ApiError::new(401, "auth_required", msg!("Sign in to QTS to continue")));
    };
    let now = util::now_secs();
    let user = match cached(store, &sid, now) {
        Some(u) => u,
        None => match ask_qts(&sid, client_ip) {
            Verdict::Pass(u) => {
                let entry = json!({ "user": u.name, "admin": u.admin, "at": now });
                let _ = write_atomic(&cache_file(store, &sid), entry.to_string().as_bytes(), 0o600);
                if now % 16 == 0 {
                    prune(store);
                }
                u
            }
            Verdict::Denied => return Err(ApiError::new(401, "auth_required", msg!("Your QTS session has expired. Sign in again."))),
            Verdict::Unavailable => {
                return Err(ApiError::new(503, "qts_unavailable", msg!("Could not check your QTS session. Try again in a moment.")));
            }
        },
    };
    if !user.admin {
        return Err(ApiError::new(403, "admin_required", msg!("Only administrators can use this app")));
    }
    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;

    #[test]
    fn session_verdicts() {
        assert_eq!(
            verdict(
                "<QDocRoot><authPassed><![CDATA[1]]></authPassed><user>u</user><username><![CDATA[alice]]></username><isAdmin>0</isAdmin></QDocRoot>"
            ),
            Verdict::Pass(User { name: "alice".into(), admin: false })
        );
        assert_eq!(
            verdict("<QDocRoot><authPassed>1</authPassed><username>admin</username><isAdmin><![CDATA[1]]></isAdmin></QDocRoot>"),
            Verdict::Pass(User { name: "admin".into(), admin: true })
        );
        assert_eq!(
            verdict(r#"<?xml version="1.0"?><QDocRoot version="1.0"><authPassed><![CDATA[0]]></authPassed></QDocRoot>"#),
            Verdict::Denied
        );
        assert_eq!(verdict("<QDocRoot><authPassed>1</authPassed></QDocRoot>"), Verdict::Unavailable, "no user name");
        assert_eq!(verdict("<html>Service Temporarily Overloaded</html>"), Verdict::Unavailable);
    }

    #[test]
    fn reads_the_sid_cookie_only() {
        assert_eq!(cookie_sid("a=b; NAS_SID=abc123XY; c=d").as_deref(), Some("abc123XY"));
        assert_eq!(cookie_sid("NAS_SID=abc;x"), Some("abc".into()));
        assert_eq!(cookie_sid("XNAS_SID=abc"), None);
        assert_eq!(cookie_sid("NAS_SID="), None);
        assert_eq!(cookie_sid("NAS_SID=a%20b"), None);
        assert_eq!(cookie_sid(""), None);
    }

    #[test]
    fn passes_are_cached_for_thirty_seconds() {
        let s = temp_store("auth");
        let now = util::now_secs();
        assert_eq!(check(&s, "", "1.2.3.4").unwrap_err().status, 401);
        let entry = json!({ "user": "admin", "admin": true, "at": now - 5 });
        write_atomic(&cache_file(&s, "abc"), entry.to_string().as_bytes(), 0o600).unwrap();
        assert_eq!(check(&s, "NAS_SID=abc", "1.2.3.4").unwrap().name, "admin");
        assert_eq!(cached(&s, "abc", now + 40), None, "expired");
        assert_eq!(cached(&s, "abd", now), None, "another sid");
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(cache_file(&s, "abc")).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600);
        let entry = json!({ "user": "bob", "admin": false, "at": now });
        write_atomic(&cache_file(&s, "bob1"), entry.to_string().as_bytes(), 0o600).unwrap();
        assert_eq!(check(&s, "NAS_SID=bob1", "1.2.3.4").unwrap_err().code, "admin_required");
        let _ = std::fs::remove_dir_all(&s.root);
    }
}

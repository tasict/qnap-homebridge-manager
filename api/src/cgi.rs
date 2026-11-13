//! The CGI front: reads the request from the environment and standard input, checks where it comes from and who
//! sends it, routes it and writes the JSON answer.

use crate::api::{self, Req};
use crate::error::ApiError;
use crate::store::Store;
use serde_json::{Value, json};
use std::io::{Read, Write};

pub const MAX_BODY: usize = 256 * 1024;

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}

/// The method after `X-HB-Method` (only `POST` may be turned into `PUT` or `DELETE`).
pub fn effective_method(method: &str, override_header: &str) -> Result<String, ApiError> {
    let m = method.to_ascii_uppercase();
    let o = override_header.trim().to_ascii_uppercase();
    if o.is_empty() {
        return Ok(m);
    }
    if m == "POST" && (o == "PUT" || o == "DELETE") {
        return Ok(o);
    }
    Err(ApiError::bad_request(msg!("The X-HB-Method header has a value that is not supported")))
}

/// `host[:port]` of an `Origin`, with the scheme's default port left out.
fn origin_authority(origin: &str) -> Option<(String, String)> {
    let (scheme, rest) = origin.split_once("://")?;
    let auth = rest.split('/').next()?.to_ascii_lowercase();
    Some((scheme.to_ascii_lowercase(), auth))
}

fn strip_default_port(auth: &str, scheme: &str) -> String {
    match (scheme, auth.rsplit_once(':')) {
        ("https", Some((h, "443"))) | ("http", Some((h, "80"))) => h.to_string(),
        _ => auth.to_string(),
    }
}

/// When there is an `Origin`, its host must be the `Host` the request was sent to.
pub fn origin_ok(origin: &str, host: &str, https: bool) -> bool {
    if origin.is_empty() {
        return true;
    }
    let Some((scheme, auth)) = origin_authority(origin) else { return false };
    let host_scheme = if https { "https" } else { "http" };
    strip_default_port(&auth, &scheme) == strip_default_port(&host.to_ascii_lowercase(), host_scheme)
}

/// The checks that come before the session: request forgery and size.
pub fn precheck(method: &str, hb_request: &str, origin: &str, host: &str, https: bool) -> Result<(), ApiError> {
    if method != "GET" && hb_request.trim() != "1" {
        return Err(ApiError::new(403, "bad_origin", msg!("The request is missing the X-HB-Request header")));
    }
    if !origin_ok(origin, host, https) {
        return Err(ApiError::new(403, "bad_origin", msg!("The request comes from a different site")));
    }
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// The CGI answer: status line, headers and the JSON envelope.
pub fn response(result: &Result<Value, ApiError>) -> Vec<u8> {
    let (status, body) = match result {
        Ok(v) => (200, json!({ "ok": true, "data": v })),
        Err(e) => (e.status, e.to_json()),
    };
    let mut out = format!(
        "Status: {status} {}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        reason(status)
    )
    .into_bytes();
    out.extend_from_slice(body.to_string().as_bytes());
    out
}

fn handle() -> Result<Value, ApiError> {
    let method = effective_method(&env("REQUEST_METHOD"), &env("HTTP_X_HB_METHOD"))?;
    let https = env("HTTPS").eq_ignore_ascii_case("on") || env("REQUEST_SCHEME") == "https";
    precheck(&method, &env("HTTP_X_HB_REQUEST"), &env("HTTP_ORIGIN"), &env("HTTP_HOST"), https)?;
    let len: usize = match env("CONTENT_LENGTH").trim() {
        "" => 0,
        n => n.parse().map_err(|_| ApiError::bad_request(msg!("The Content-Length header is not valid")))?,
    };
    if len > MAX_BODY {
        return Err(ApiError::bad_request(msg!("The request is too large (the limit is 256 KiB)")));
    }
    let mut body = Vec::with_capacity(len);
    std::io::stdin().take(len as u64).read_to_end(&mut body)?;
    let store = Store::open();
    let user = crate::auth::check(&store, &env("HTTP_COOKIE"), &env("REMOTE_ADDR"))?;
    let req = Req::new(&method, &env("PATH_INFO"), &env("QUERY_STRING"), body);
    api::route(&req, &store, &user.name)
}

pub fn run() -> i32 {
    let result = match std::panic::catch_unwind(handle) {
        Ok(r) => r,
        Err(_) => Err(ApiError::internal(msg!("An unexpected error occurred on the server"))),
    };
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&response(&result));
    let _ = out.flush();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_override() {
        assert_eq!(effective_method("POST", "PUT").unwrap(), "PUT");
        assert_eq!(effective_method("post", "delete").unwrap(), "DELETE");
        assert_eq!(effective_method("POST", "").unwrap(), "POST");
        assert_eq!(effective_method("GET", "").unwrap(), "GET");
        assert!(effective_method("GET", "DELETE").is_err(), "a GET cannot become a DELETE");
        assert!(effective_method("POST", "PATCH").is_err());
    }

    #[test]
    fn origin_rules() {
        assert!(origin_ok("", "nas:8080", false));
        assert!(origin_ok("http://NAS:8080", "nas:8080", false));
        assert!(origin_ok("https://nas.example.com", "nas.example.com:443", true));
        assert!(origin_ok("https://nas.example.com:443", "nas.example.com", true));
        assert!(!origin_ok("http://evil.example", "nas:8080", false));
        assert!(!origin_ok("http://nas:8081", "nas:8080", false));
        assert!(!origin_ok("null", "nas:8080", false));
    }

    #[test]
    fn forgery_checks() {
        assert!(precheck("GET", "", "", "nas", false).is_ok());
        assert_eq!(precheck("POST", "", "", "nas", false).unwrap_err().code, "bad_origin");
        assert!(precheck("POST", "1", "http://nas", "nas", false).is_ok());
        assert_eq!(precheck("DELETE", "1", "http://other", "nas", false).unwrap_err().status, 403);
    }

    #[test]
    fn answers_with_the_envelope() {
        let ok = String::from_utf8(response(&Ok(json!({ "a": 1 })))).unwrap();
        assert!(ok.starts_with("Status: 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\n"));
        assert!(ok.ends_with("\r\n\r\n{\"data\":{\"a\":1},\"ok\":true}"));
        let err = String::from_utf8(response(&Err(ApiError::field("network.ip", msg!("Enter an IP address"))))).unwrap();
        assert!(err.starts_with("Status: 400 Bad Request\r\n"));
        let body: Value = serde_json::from_str(err.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "validation");
        assert_eq!(body["error"]["fields"]["network.ip"], json!({ "key": "Enter an IP address" }));
        assert_eq!(body["error"]["msg"], json!({ "key": "Enter an IP address" }));
        assert_eq!(body["error"]["message"], "Enter an IP address");
    }
}

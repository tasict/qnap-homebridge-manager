//! Text for people. The backend never writes a sentence in a particular language: it sends a `Msg`, an English
//! sentence with `{placeholder}` names (the key of the UI's catalogs) and the values to put in them. The UI renders it
//! in the viewer's language; `english()` fills in the English sentence for logs and the command line.
//!
//! Every Msg is made with the `msg!` macro and a string literal, so the keys can be checked against the catalogs in
//! `shared/web/i18n/<lang>.backend.json` (see the tests below).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "Repr")]
pub struct Msg {
    pub key: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
}

/// What a stored Msg may look like: the object, or plain text written by an earlier version.
#[derive(Deserialize)]
#[serde(untagged)]
enum Repr {
    Full {
        key: String,
        #[serde(default)]
        args: Map<String, Value>,
    },
    Text(String),
}

impl From<Repr> for Msg {
    fn from(r: Repr) -> Msg {
        match r {
            Repr::Full { key, args } => Msg { key, args },
            Repr::Text(key) => Msg { key, args: Map::new() },
        }
    }
}

/// Values a placeholder can take: text or a whole number.
pub trait IntoArg {
    fn into_arg(self) -> Value;
}

impl IntoArg for &str {
    fn into_arg(self) -> Value {
        Value::String(self.to_string())
    }
}

impl IntoArg for String {
    fn into_arg(self) -> Value {
        Value::String(self)
    }
}

impl IntoArg for &String {
    fn into_arg(self) -> Value {
        Value::String(self.clone())
    }
}

macro_rules! number_arg {
    ($($t:ty),*) => {
        $(impl IntoArg for $t {
            fn into_arg(self) -> Value {
                Value::from(self)
            }
        })*
    };
}

number_arg!(u8, u16, u32, u64, usize, i32, i64);

/// The `{name}` placeholders of a key, in order of appearance.
pub fn placeholders(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = key;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if is_name(&after[..end]) => {
                out.push(after[..end].to_string());
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out
}

fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && (b[0].is_ascii_lowercase() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

impl Msg {
    /// Only for the `msg!` macro: keys are always string literals.
    #[doc(hidden)]
    pub fn new(key: &str) -> Msg {
        Msg { key: key.to_string(), args: Map::new() }
    }

    pub fn arg(mut self, name: &str, value: impl IntoArg) -> Msg {
        self.args.insert(name.to_string(), value.into_arg());
        self
    }

    /// The English sentence with the arguments filled in.
    pub fn english(&self) -> String {
        let mut out = self.key.clone();
        for p in placeholders(&self.key) {
            let value = match self.args.get(&p) {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => {
                    debug_assert!(false, "no value for {{{p}}} in {:?}", self.key);
                    continue;
                }
            };
            out = out.replace(&format!("{{{p}}}"), &value);
        }
        out
    }

    /// The Msg as one line of compact JSON (job logs).
    pub fn compact(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    #[test]
    fn fills_in_and_serialises() {
        let m = msg!("The address {ip} is already used by container {name}", ip = "192.168.10.7", name = "homebridge-1F");
        assert_eq!(m.english(), "The address 192.168.10.7 is already used by container homebridge-1F");
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            serde_json::json!({ "key": "The address {ip} is already used by container {name}", "args": { "ip": "192.168.10.7", "name": "homebridge-1F" } })
        );
        let plain = msg!("Download the image");
        assert_eq!(serde_json::to_string(&plain).unwrap(), r#"{"key":"Download the image"}"#);
        let n = msg!("Checking {image} ({done} of {count})", image = "alpine:3.24", done = 2usize, count = 3usize);
        assert_eq!(n.english(), "Checking alpine:3.24 (2 of 3)");
        assert_eq!(n.args["count"], 3);
        // Earlier versions stored plain text.
        let old: Msg = serde_json::from_str("\"plain\"").unwrap();
        assert_eq!(old, Msg { key: "plain".into(), args: Map::new() });
        let back: Msg = serde_json::from_str(&m.compact()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn finds_placeholders() {
        assert_eq!(placeholders("Checking {image} ({done} of {count})"), ["image", "done", "count"]);
        assert!(placeholders("No {Upper} or { spaced } or {}").is_empty());
    }

    // ---- The catalogs ----------------------------------------------------------------------------------------

    /// One `msg!` call in the source: its key and the names of its arguments.
    #[derive(Debug)]
    struct Use {
        file: String,
        key: String,
        args: BTreeSet<String>,
    }

    /// The part of a source file before its tests, without line comments.
    fn product_code(text: &str) -> String {
        let text = text.split("#[cfg(test)]").next().unwrap_or("");
        text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
    }

    /// Reads a string literal starting at `s[0] == '"'`; returns its value and the rest after it.
    fn literal(s: &str) -> Option<(String, &str)> {
        let mut out = String::new();
        let mut it = s.char_indices().skip(1);
        while let Some((i, c)) = it.next() {
            match c {
                '"' => return Some((out, &s[i + 1..])),
                '\\' => match it.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// The argument names after the key, up to the macro's closing parenthesis.
    fn arg_names(rest: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let (mut depth, mut seg, mut in_str) = (1i32, String::new(), false);
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            if in_str {
                if c == '\\' {
                    chars.next();
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 || (depth == 1 && c == ',') {
                let s = seg.trim();
                if let Some((name, value)) = s.split_once('=')
                    && !value.starts_with('=')
                    && name.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !name.trim().is_empty()
                {
                    names.insert(name.trim().to_string());
                }
                seg.clear();
                if depth == 0 {
                    break;
                }
                continue;
            }
            seg.push(c);
        }
        names
    }

    fn uses() -> Vec<Use> {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        for e in std::fs::read_dir(&src).unwrap().flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if !file.ends_with(".rs") {
                continue;
            }
            let code = product_code(&std::fs::read_to_string(e.path()).unwrap());
            let mut rest = code.as_str();
            while let Some(at) = rest.find("msg!(") {
                let after = rest[at + 5..].trim_start();
                rest = &rest[at + 5..];
                if !after.starts_with('"') {
                    continue;
                }
                let (key, tail) = literal(after).expect("a closed string literal");
                out.push(Use { file: file.clone(), key, args: arg_names(tail) });
            }
        }
        out
    }

    /// A catalog file in its own order: (key, translation) pairs.
    struct Ordered(Vec<(String, String)>);

    impl<'de> Deserialize<'de> for Ordered {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> serde::de::Visitor<'de> for V {
                type Value = Ordered;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("an object of strings")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Ordered, A::Error> {
                    let mut v = Vec::new();
                    while let Some(entry) = m.next_entry::<String, String>()? {
                        v.push(entry);
                    }
                    Ok(Ordered(v))
                }
            }
            d.deserialize_map(V)
        }
    }

    /// `HBMGR_I18N_DIR`, else `shared/web/i18n` next to `api/` (in the build container `shared/` is mounted beside
    /// the sources).
    fn catalog_dir() -> PathBuf {
        match std::env::var_os("HBMGR_I18N_DIR") {
            Some(d) => PathBuf::from(d),
            None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/web/i18n"),
        }
    }

    #[test]
    fn every_msg_names_its_placeholders() {
        let uses = uses();
        assert!(uses.len() > 100, "the source scan found only {} msg! calls", uses.len());
        let mut problems = Vec::new();
        for u in &uses {
            let want: BTreeSet<String> = placeholders(&u.key).into_iter().collect();
            if want != u.args {
                problems.push(format!("{}: {:?} has arguments {:?}", u.file, u.key, u.args));
            }
            if u.key.trim() != u.key || u.key.is_empty() || u.key.contains('\n') {
                problems.push(format!("{}: {:?} is not a clean key", u.file, u.key));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn catalogs_cover_exactly_the_source_keys() {
        let mut keys: BTreeMap<String, String> = BTreeMap::new();
        for u in uses() {
            keys.entry(u.key).or_insert(u.file);
        }
        let dir = catalog_dir();
        let mut problems = Vec::new();
        for lang in ["zh-TW", "ja"] {
            let path = dir.join(format!("{lang}.backend.json"));
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let Ordered(entries) = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut seen = BTreeSet::new();
            for (i, (k, v)) in entries.iter().enumerate() {
                if !seen.insert(k.clone()) {
                    problems.push(format!("{lang}: {k:?} appears twice"));
                }
                if i > 0 && entries[i - 1].0 >= *k {
                    problems.push(format!("{lang}: {k:?} is out of order"));
                }
                if !keys.contains_key(k) {
                    problems.push(format!("{lang}: {k:?} is not used in the source any more"));
                }
                if v.trim().is_empty() {
                    problems.push(format!("{lang}: {k:?} has no translation"));
                }
                let want: BTreeSet<String> = placeholders(k).into_iter().collect();
                let got: BTreeSet<String> = placeholders(v).into_iter().collect();
                if want != got {
                    problems.push(format!("{lang}: {k:?} has placeholders {got:?} in its translation"));
                }
            }
            for (k, file) in &keys {
                if !seen.contains(k) {
                    problems.push(format!("{lang}: {k:?} ({file}) is missing"));
                }
            }
        }
        assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
    }

    #[test]
    fn no_sentences_in_other_languages_in_the_product_code() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for e in std::fs::read_dir(&src).unwrap().flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            for (n, line) in code.lines().enumerate() {
                let cjk = line.chars().any(|c| matches!(c as u32, 0x3000..=0x9fff | 0xf900..=0xfaff | 0xff00..=0xffef));
                assert!(!cjk, "{}:{}: {line}", e.file_name().to_string_lossy(), n + 1);
            }
        }
    }
}

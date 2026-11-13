//! The data directory (`<Install_Path>/data`, or `HBMGR_DATA`): instance records, settings, the update-check
//! cache, jobs and run-time files. It is not part of the package, so it survives upgrades.

use crate::spec::{self, Mount, Spec};
use crate::util::{now_rfc3339, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// What is kept about an instance.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Record {
    pub spec: Spec,
    pub created_at: String,
    pub updated_at: String,
    pub backup: Option<BackupRecord>,
}

/// The container kept as `<name>.hbmgr-prev` by the last recreate.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct BackupRecord {
    pub container_id: String,
    pub image: String,
    pub image_id: String,
    pub created_at: String,
    /// Its restart policy before it was set to `no`, restored when it is swapped back in.
    pub restart: Restart,
    /// Whether it was running before it was replaced.
    pub was_running: bool,
    /// The spec it was created from (for a container this package did not create: what was read from it).
    pub spec: Option<Spec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Restart {
    pub name: String,
    pub max_retry: i64,
}

impl Default for Restart {
    fn default() -> Self {
        Restart { name: "always".into(), max_retry: 0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub default_image: String,
    pub default_mounts: Vec<Mount>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { default_image: spec::DEFAULT_IMAGE.into(), default_mounts: Vec::new() }
    }
}

/// `state/updates.json`: the last answer of the registry for every image.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Updates {
    pub checked_at: Option<String>,
    pub images: Vec<ImageUpdate>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ImageUpdate {
    pub image: String,
    pub local_digest: Option<String>,
    pub remote_digest: Option<String>,
    pub available: bool,
    /// The last check's failure in English (logs, the command line) …
    pub error: Option<String>,
    /// … and as a Msg for the UI.
    pub error_msg: Option<crate::msg::Msg>,
}

impl Updates {
    pub fn entry(&self, image: &str) -> Option<&ImageUpdate> {
        self.images.iter().find(|i| i.image == image)
    }
}

pub struct Store {
    pub root: PathBuf,
}

fn mkdir_private(p: &Path) {
    if !p.exists() {
        let _ = fs::DirBuilder::new().recursive(true).mode(0o700).create(p);
    }
}

impl Store {
    /// `HBMGR_DATA`, else `data/` next to the executable's directory.
    pub fn open() -> Store {
        let root = std::env::var_os("HBMGR_DATA").map(PathBuf::from).unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.canonicalize().ok())
                .and_then(|e| e.parent()?.parent().map(|p| p.join("data")))
                .unwrap_or_else(|| PathBuf::from("data"))
        });
        Store::at(root)
    }

    pub fn at(root: PathBuf) -> Store {
        let s = Store { root };
        for d in [s.root.clone(), s.instances_dir(), s.jobs_dir(), s.state_dir(), s.run_dir(), s.locks_dir(), s.sessions_dir()] {
            mkdir_private(&d);
        }
        s
    }

    pub fn instances_dir(&self) -> PathBuf {
        self.root.join("instances")
    }
    pub fn jobs_dir(&self) -> PathBuf {
        self.root.join("jobs")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("run/locks")
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("run/sessions")
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.instances_dir().join(format!("{name}.json"))
    }

    /// All records, oldest first.
    pub fn records(&self) -> Vec<Record> {
        let mut out: Vec<Record> = fs::read_dir(self.instances_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .filter_map(|e| serde_json::from_slice::<Record>(&fs::read(e.path()).ok()?).ok())
            .filter(|r| spec::valid_name(&r.spec.name))
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.spec.name.cmp(&b.spec.name)));
        out
    }

    /// The record of `name` (exact match).
    pub fn record(&self, name: &str) -> Option<Record> {
        if !spec::valid_name(name) {
            return None;
        }
        let r: Record = serde_json::from_slice(&fs::read(self.record_path(name)).ok()?).ok()?;
        (r.spec.name == name).then_some(r)
    }

    /// A record whose name equals `name` ignoring case.
    pub fn record_ignoring_case(&self, name: &str) -> Option<Record> {
        self.records().into_iter().find(|r| r.spec.name.eq_ignore_ascii_case(name))
    }

    pub fn save_record(&self, r: &mut Record) -> std::io::Result<()> {
        let now = now_rfc3339();
        if r.created_at.is_empty() {
            r.created_at = now.clone();
        }
        r.updated_at = now;
        write_atomic(&self.record_path(&r.spec.name), &serde_json::to_vec_pretty(r).unwrap_or_default(), 0o600)
    }

    pub fn delete_record(&self, name: &str) -> std::io::Result<()> {
        match fs::remove_file(self.record_path(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn settings(&self) -> Settings {
        fs::read(self.root.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> std::io::Result<()> {
        write_atomic(&self.root.join("settings.json"), &serde_json::to_vec_pretty(s).unwrap_or_default(), 0o600)
    }

    pub fn updates(&self) -> Updates {
        fs::read(self.state_dir().join("updates.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_updates(&self, u: &Updates) -> std::io::Result<()> {
        write_atomic(&self.state_dir().join("updates.json"), &serde_json::to_vec_pretty(u).unwrap_or_default(), 0o600)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn temp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!(
            "hbmgr-test-{tag}-{}-{}",
            std::process::id(),
            crate::util::hex(&crate::util::random_bytes(4))
        ));
        Store::at(dir)
    }

    #[test]
    fn records_round_trip_and_sort() {
        let s = temp_store("records");
        let mut b = Record { spec: Spec { name: "b".into(), ..Default::default() }, ..Default::default() };
        b.created_at = "2026-11-12T10:00:00Z".into();
        s.save_record(&mut b).unwrap();
        let mut a = Record { spec: Spec { name: "A".into(), ..Default::default() }, ..Default::default() };
        a.created_at = "2026-11-12T11:00:00Z".into();
        s.save_record(&mut a).unwrap();
        let names: Vec<String> = s.records().into_iter().map(|r| r.spec.name).collect();
        assert_eq!(names, vec!["b", "A"]);
        assert!(s.record("a").is_none(), "names are exact");
        assert_eq!(s.record_ignoring_case("a").unwrap().spec.name, "A");
        assert!(s.record("../x").is_none());
        s.delete_record("A").unwrap();
        s.delete_record("A").unwrap();
        assert_eq!(s.records().len(), 1);
        assert_eq!(s.settings().default_image, spec::DEFAULT_IMAGE);
        let _ = fs::remove_dir_all(&s.root);
    }
}

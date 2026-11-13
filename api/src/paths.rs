//! Host folders: what may be mounted into a container, and the folder browser.

use crate::msg::Msg;
use serde_json::{Value, json};
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

pub const SHARE: &str = "/share";
const MAX_ENTRIES: usize = 2000;

/// Folders that must never be mounted: this package's install and data directories (resolved).
#[derive(Clone, Debug, Default)]
pub struct Own {
    pub dirs: Vec<String>,
}

/// An absolute path with `.` and repeated or trailing `/` removed; `..` is refused.
pub fn normalize(p: &str) -> Result<String, Msg> {
    if !p.starts_with('/') {
        return Err(msg!("Use an absolute path that starts with /"));
    }
    if p.len() > 4096 || p.contains(['\0', '\n', '\r', ':', ',']) {
        return Err(msg!("The path contains characters that are not allowed"));
    }
    let mut parts = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => return Err(msg!("The path cannot contain ..")),
            c => parts.push(c),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

/// Components that are never allowed anywhere in a (resolved) path.
fn forbidden_component(parts: &[&str]) -> bool {
    parts.iter().any(|c| *c == ".qpkg" || c.starts_with(".@") || *c == "@Recycle" || *c == "@Recently-Snapshot")
        || parts.windows(2).any(|w| w[0] == "Container" && w[1] == "container-station-data")
}

/// The parts of `p` below `/share`, or `None` when it is not under `/share`.
fn below_share(p: &str) -> Option<Vec<&str>> {
    let rest = p.strip_prefix("/share/")?;
    Some(rest.split('/').filter(|c| !c.is_empty()).collect())
}

/// Whether a resolved path may be browsed: under `/share`, nothing forbidden in it, not this package's folders.
pub fn openable(resolved: &str, own: &Own) -> Result<(), Msg> {
    if resolved == SHARE {
        return Ok(());
    }
    let Some(parts) = below_share(resolved) else { return Err(msg!("Only folders under /share can be used")) };
    if forbidden_component(&parts) {
        return Err(msg!("System folders cannot be used"));
    }
    if own.dirs.iter().any(|d| resolved == d || resolved.starts_with(&format!("{d}/"))) {
        return Err(msg!("The app's own folders cannot be used"));
    }
    Ok(())
}

/// Whether a resolved path may be mounted: openable, and at least `/share/<volume>/<share>` (a shared folder or a
/// folder in one, never a volume root).
pub fn selectable(resolved: &str, own: &Own) -> Result<(), Msg> {
    openable(resolved, own)?;
    match below_share(resolved) {
        Some(parts) if parts.len() >= 2 => Ok(()),
        _ => Err(msg!("Choose a shared folder or a folder inside one")),
    }
}

/// Places a link may never lead to, whoever made it: control sockets and kernel interfaces, whole system trees, and
/// everything `forbidden_component` names. A folder inside a system tree (the QTS certificate folder under
/// `/etc/config`) is not refused here.
fn outside_target_refused(real: &str) -> bool {
    let parts: Vec<&str> = real.split('/').filter(|c| !c.is_empty()).collect();
    if parts.is_empty() || forbidden_component(&parts) {
        return true;
    }
    const TREES: [&str; 8] = ["/proc", "/sys", "/dev", "/run", "/var/run", "/var/lock", "/tmp", "/root"];
    const WHOLE: [&str; 14] = [
        "/etc",
        "/etc/config",
        "/mnt",
        "/mnt/HDA_ROOT",
        "/mnt/HDA_ROOT/.config",
        "/mnt/ext",
        "/var",
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/home",
        "/opt",
        "/share",
    ];
    TREES.iter().any(|t| real == *t || real.starts_with(&format!("{t}/"))) || WHOLE.contains(&real)
}

/// A folder whose real location is outside `/share`, reached through a link in a shared folder (on QTS,
/// `/share/Docker/stunnel -> /etc/config/stunnel` is a common way to hand the NAS certificate to a container). It is
/// accepted when the link that leaves `/share` belongs to root, i.e. an administrator put it there, and the target is
/// not one `outside_target_refused` names.
fn admin_link_outside(norm: &str, real: &str) -> bool {
    if real == SHARE || below_share(real).is_some() || outside_target_refused(real) {
        return false;
    }
    let mut cur = String::new();
    for c in norm.split('/').filter(|c| !c.is_empty()) {
        cur.push('/');
        cur.push_str(c);
        let Ok(r) = fs::canonicalize(&cur) else { return false };
        let r = r.to_string_lossy();
        if r == SHARE || below_share(&r).is_some() {
            continue;
        }
        // the first step that leaves /share
        return fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink() && m.uid() == 0);
    }
    false
}

/// Checks a host folder for a mount and returns its normalised (not resolved) path: that is what is shown and passed
/// to Docker. With `may_be_missing`, a folder that does not exist yet is accepted when its parent passes.
pub fn check_dir(p: &str, may_be_missing: bool, own: &Own) -> Result<String, Msg> {
    let norm = normalize(p.trim())?;
    if below_share(&norm).is_none_or(|v| v.is_empty()) {
        return Err(msg!("Only folders under /share can be used"));
    }
    match fs::canonicalize(&norm) {
        Ok(real) => {
            if !real.is_dir() {
                return Err(msg!("This is not a folder"));
            }
            let real = real.to_string_lossy();
            if let Err(e) = selectable(&real, own)
                && !admin_link_outside(&norm, &real)
            {
                return Err(e);
            }
            Ok(norm)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound && may_be_missing => {
            let (parent, name) = norm.rsplit_once('/').unwrap_or(("", ""));
            let parent_real =
                fs::canonicalize(if parent.is_empty() { "/" } else { parent }).map_err(|_| msg!("The parent folder does not exist"))?;
            if !parent_real.is_dir() {
                return Err(msg!("The parent folder does not exist"));
            }
            let real = format!("{}/{name}", parent_real.to_string_lossy());
            selectable(&real, own)?;
            Ok(norm)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(msg!("The folder does not exist")),
        Err(_) => Err(msg!("The folder cannot be read")),
    }
}

/// Hidden and system entries the browser leaves out. At `/share` the volume roots (`CACHEDEV1_DATA`, `HDA_DATA`)
/// and NFS's directory are left out too: the shared folders are links into them.
fn hidden(name: &str, at_share: bool) -> bool {
    name.starts_with('.')
        || name.starts_with('@')
        || name == "lost+found"
        || (at_share && (name.ends_with("_DATA") || name.starts_with("NFSv=")))
}

/// `GET /fs`: the folders in `path`.
pub fn list(path: &str, own: &Own) -> Result<Value, Msg> {
    let norm = normalize(if path.is_empty() { SHARE } else { path })?;
    if norm != SHARE && below_share(&norm).is_none() {
        return Err(msg!("Only folders under /share can be browsed"));
    }
    let real = fs::canonicalize(&norm).map_err(|_| msg!("The folder does not exist"))?;
    let real_s = real.to_string_lossy().to_string();
    if !real.is_dir() {
        return Err(msg!("This is not a folder"));
    }
    if let Err(e) = openable(&real_s, own)
        && !admin_link_outside(&norm, &real_s)
    {
        return Err(e);
    }
    let at_share = norm == SHARE;
    let mut dirs = Vec::new();
    for entry in fs::read_dir(&real).map_err(|_| msg!("The folder cannot be read"))?.flatten().take(MAX_ENTRIES * 4) {
        let name = entry.file_name().to_string_lossy().to_string();
        if hidden(&name, at_share) || name.contains(['\n', ':', ',']) {
            continue;
        }
        let Ok(child_real) = fs::canonicalize(entry.path()) else { continue };
        if !child_real.is_dir() {
            continue;
        }
        let child_real = child_real.to_string_lossy().to_string();
        let child = if at_share { format!("{SHARE}/{name}") } else { format!("{norm}/{name}") };
        let linked = admin_link_outside(&child, &child_real);
        if openable(&child_real, own).is_err() && !linked {
            continue;
        }
        dirs.push(json!({ "name": name, "path": child, "selectable": linked || selectable(&child_real, own).is_ok() }));
        if dirs.len() >= MAX_ENTRIES {
            break;
        }
    }
    dirs.sort_by_key(|d| d["name"].as_str().unwrap_or("").to_lowercase());
    let parent =
        if at_share { Value::Null } else { json!(norm.rsplit_once('/').map(|(p, _)| p).filter(|p| !p.is_empty()).unwrap_or(SHARE)) };
    Ok(json!({ "path": norm, "parent": parent, "dirs": dirs }))
}

/// `POST /fs/mkdir`: creates one folder whose parent exists and passes the rules.
pub fn mkdir(path: &str, own: &Own) -> Result<String, Msg> {
    let norm = normalize(path.trim())?;
    let name = norm.rsplit_once('/').map(|(_, n)| n).unwrap_or("");
    if name.is_empty() || name.len() > 255 || name.starts_with('.') || name.starts_with('@') {
        return Err(msg!("Folder names cannot start with . or @"));
    }
    if Path::new(&norm).symlink_metadata().is_ok() {
        return Err(msg!("A folder or file with this name already exists"));
    }
    check_dir(&norm, true, own)?;
    fs::DirBuilder::new().mode(0o755).create(&norm).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => msg!("A folder or file with this name already exists"),
        _ => msg!("The folder could not be created: {error}", error = e.to_string()),
    })?;
    Ok(norm)
}

/// Creates a work directory that does not exist yet (its parent must exist). An existing one is left exactly as it is.
pub fn ensure_workdir(path: &str) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o755).create(path)?;
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

/// This package's own folders, resolved.
pub fn own_dirs(data: &Path) -> Own {
    let mut dirs = Vec::new();
    let add = |p: PathBuf, dirs: &mut Vec<String>| {
        if let Ok(r) = p.canonicalize() {
            dirs.push(r.to_string_lossy().to_string());
        }
    };
    if let Some(root) = crate::sys::install_root() {
        add(root, &mut dirs);
    }
    add(data.to_path_buf(), &mut dirs);
    Own { dirs }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> Own {
        Own { dirs: vec!["/share/CACHEDEV1_DATA/.qpkg/Homebridge".into(), "/share/CACHEDEV2_DATA/Develop/hb/data".into()] }
    }

    #[test]
    fn normalises() {
        assert_eq!(normalize("/share//Docker/./hb/"), Ok("/share/Docker/hb".into()));
        assert_eq!(normalize("/"), Ok("/".into()));
        assert!(normalize("share/Docker").is_err());
        assert!(normalize("/share/Docker/../x").is_err());
        assert!(normalize("/share/a:b").is_err());
    }

    #[test]
    fn path_rules() {
        let o = own();
        assert!(selectable("/share/CACHEDEV2_DATA/Docker", &o).is_ok());
        assert!(selectable("/share/CACHEDEV2_DATA/Docker/homebridge-1F", &o).is_ok());
        assert!(selectable("/share/CACHEDEV2_DATA", &o).is_err(), "a volume root");
        assert!(openable("/share/CACHEDEV2_DATA", &o).is_ok(), "but it may be browsed");
        assert!(selectable("/share", &o).is_err());
        assert!(selectable("/etc/config", &o).is_err());
        assert!(selectable("/share/CACHEDEV1_DATA/.qpkg/container-station", &o).is_err());
        assert!(selectable("/share/CACHEDEV1_DATA/.@msdfs_root/NAS", &o).is_err());
        assert!(selectable("/share/CACHEDEV1_DATA/Public/@Recycle", &o).is_err());
        assert!(selectable("/share/CACHEDEV1_DATA/Public/@Recently-Snapshot/x", &o).is_err());
        assert!(selectable("/share/CACHEDEV1_DATA/Container/container-station-data/lib", &o).is_err());
        assert!(selectable("/share/CACHEDEV1_DATA/Container/mine", &o).is_ok());
        assert!(selectable("/share/CACHEDEV2_DATA/Develop/hb/data", &o).is_err(), "the package's data");
        assert!(selectable("/share/CACHEDEV2_DATA/Develop/hb/data/instances", &o).is_err());
        assert!(selectable("/share/CACHEDEV2_DATA/Develop/hb/database", &o).is_ok(), "a prefix that is not a parent");
        assert!(selectable("/share/external/DEV3301_1", &o).is_ok());
    }

    #[test]
    fn link_targets_outside_share() {
        assert!(!outside_target_refused("/mnt/HDA_ROOT/.config/stunnel"), "the QTS certificate folder");
        assert!(!outside_target_refused("/mnt/ext/opt/something"));
        for p in ["/", "/etc", "/etc/config", "/mnt/HDA_ROOT/.config", "/var/run", "/var/run/docker", "/proc/1", "/dev", "/tmp/x", "/root"]
        {
            assert!(outside_target_refused(p), "{p}");
        }
        assert!(outside_target_refused("/mnt/HDA_ROOT/.qpkg/x"), "a package folder, wherever it is");
        assert!(!admin_link_outside("/share/Docker/hb", "/share/CACHEDEV2_DATA/Docker/hb"), "inside /share: the ordinary rules decide");
        assert!(!admin_link_outside("/share/Docker/run", "/var/run"));
        assert!(!admin_link_outside("/share/no-such-share-anywhere/x", "/mnt/HDA_ROOT/.config/stunnel"), "nothing to follow");
    }

    #[test]
    fn hides_system_entries() {
        assert!(hidden(".HideDrawer", true));
        assert!(hidden("@Recycle", false));
        assert!(hidden("CACHEDEV1_DATA", true));
        assert!(hidden("NFSv=4", true));
        assert!(!hidden("Docker", true));
        assert!(!hidden("OLD_DATA", false), "only at /share");
    }

    #[test]
    fn outside_share_is_refused_before_touching_the_disk() {
        let o = own();
        assert!(check_dir("/tmp", false, &o).is_err());
        assert!(check_dir("/share", false, &o).is_err());
        assert!(list("/etc", &o).is_err());
        assert!(mkdir("/tmp/x", &o).is_err());
        assert!(mkdir("/share/Docker/.hidden", &o).is_err());
    }
}

//! Replacing a container with rollback: the one primitive behind create-with-replace, apply, upgrade and rollback.
//!
//! The old container is stopped, set to restart `no` (so it never starts at boot and fights for the address and the
//! MAC) and renamed `<name>.hbmgr-prev`; the new one is created and started under the name and watched for a while.
//! When anything fails, everything is put back exactly: the new container removed, the old one renamed back, its
//! restart policy restored and started again if it was running.

use crate::jobs::JobError;
use crate::msg::Msg;
use crate::spec::backup_name;
use crate::store::Restart;
use serde_json::Value;
use std::time::Duration;

/// A container as the primitive sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ctr {
    pub id: String,
    pub running: bool,
    pub restart: Restart,
    pub image: String,
    pub image_id: String,
    pub labels: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunState {
    pub running: bool,
    pub restarting: bool,
    pub restart_count: i64,
    pub exit_code: i64,
    pub started_at: String,
}

/// What the primitive needs from Docker. Errors are Docker's own technical text (English), passed on as an argument.
pub trait Engine {
    fn find(&mut self, name: &str) -> Result<Option<Ctr>, String>;
    fn stop(&mut self, id: &str) -> Result<(), String>;
    fn start(&mut self, id: &str) -> Result<(), String>;
    fn set_restart(&mut self, id: &str, r: &Restart) -> Result<(), String>;
    fn rename(&mut self, id: &str, to: &str) -> Result<(), String>;
    fn remove(&mut self, id: &str) -> Result<(), String>;
    fn create(&mut self, name: &str, body: &Value) -> Result<String, String>;
    fn state(&mut self, id: &str) -> Result<RunState, String>;
    fn last_logs(&mut self, id: &str, lines: usize) -> Vec<String>;
    fn wait(&mut self, d: Duration);
}

/// Where the primitive reports its steps and log lines.
pub trait Report {
    fn step(&mut self, key: &str, status: &str);
    fn log(&mut self, msg: Msg);
    /// Text that is not translated: a container's own output.
    fn raw(&mut self, text: &str);
}

pub fn no_restart() -> Restart {
    Restart { name: "no".into(), max_retry: 0 }
}

/// How the replace ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The new container runs. `previous` is the old container, now the stopped backup.
    Replaced { new_id: String, previous: Option<Ctr> },
    /// It failed and the old container is back as it was.
    RolledBack(JobError),
    /// It failed and there was nothing to put back, or putting back failed too.
    Failed(JobError),
}

fn docker_err(msg: Msg) -> JobError {
    JobError::new("docker_error", msg)
}

fn state_of(e: &mut dyn Engine, id: &str) -> Result<RunState, Msg> {
    e.state(id).map_err(|m| msg!("Could not read the state of the container: {error}", error = m))
}

/// The job error when the change failed and putting the previous container back failed as well.
fn not_restored(code: &str) -> JobError {
    JobError::new(code, msg!("The change failed and the previous container could not be fully restored. Check the log."))
}

/// Watches a started container for `secs` seconds: it must keep running without restarting.
pub fn verify(e: &mut dyn Engine, id: &str, secs: u32) -> Result<(), Msg> {
    let first = state_of(e, id)?;
    if !first.running || first.restarting {
        return Err(msg!("The container stopped right after it started (exit code {code})", code = first.exit_code));
    }
    for _ in 0..secs {
        e.wait(Duration::from_secs(1));
        let s = state_of(e, id)?;
        if s.restarting || s.restart_count > first.restart_count || s.started_at != first.started_at {
            return Err(msg!("The container keeps restarting"));
        }
        if !s.running {
            return Err(msg!("The container stopped right after it started (exit code {code})", code = s.exit_code));
        }
    }
    Ok(())
}

/// The old container's state, to put it back.
struct Saved {
    ctr: Ctr,
    renamed: bool,
    restart_changed: bool,
}

/// Puts the previous container back as `name`: name, restart policy, running state. Returns whether all of it worked
/// (what did not is logged).
fn restore(e: &mut dyn Engine, r: &mut dyn Report, name: &str, saved: &Saved) -> bool {
    let mut ok = true;
    let id = &saved.ctr.id;
    if saved.renamed
        && let Err(m) = e.rename(id, name)
    {
        r.log(msg!("Could not rename the previous container back to {name}: {error}", name = name, error = m));
        ok = false;
    }
    if saved.restart_changed
        && let Err(m) = e.set_restart(id, &saved.ctr.restart)
    {
        r.log(msg!("Could not restore the restart policy of {name}: {error}", name = name, error = m));
        ok = false;
    }
    if saved.ctr.running
        && let Err(m) = e.start(id)
    {
        r.log(msg!("Could not start {name} again: {error}", name = name, error = m));
        ok = false;
    }
    if ok {
        r.log(msg!("{name} is back as it was", name = name));
    }
    ok
}

/// Ends a failed replace: removes the new container if there is one, puts the old one back.
fn fail(
    e: &mut dyn Engine,
    r: &mut dyn Report,
    name: &str,
    step: &str,
    err: JobError,
    new_id: Option<&str>,
    saved: Option<&Saved>,
) -> Outcome {
    r.step(step, "failed");
    r.log(err.msg.clone());
    if let Some(id) = new_id {
        let output = e.last_logs(id, 20);
        if !output.is_empty() {
            r.log(msg!("Last output of the new container:"));
            for line in output {
                r.raw(&line);
            }
        }
        if let Err(m) = e.remove(id) {
            r.log(msg!("Could not remove the new container: {error}", error = m));
        }
    }
    let Some(saved) = saved else { return Outcome::Failed(err) };
    r.log(msg!("Restoring the previous container…"));
    if restore(e, r, name, saved) { Outcome::RolledBack(err) } else { Outcome::Failed(not_restored(&err.code)) }
}

/// Replaces the container `name` (if any) with a new one created from `body`.
pub fn replace(e: &mut dyn Engine, r: &mut dyn Report, name: &str, body: &Value, verify_secs: u32) -> Outcome {
    let backup = backup_name(name);
    let old = match e.find(name) {
        Ok(o) => o,
        Err(m) => {
            r.step("stop", "failed");
            return Outcome::Failed(docker_err(msg!("Docker reported an error: {error}", error = m)));
        }
    };
    let mut saved = None;
    if let Some(old) = old {
        r.step("stop", "running");
        let mut s = Saved { ctr: old, renamed: false, restart_changed: false };
        if s.ctr.running {
            r.log(msg!("Stopping the old container…"));
            if let Err(m) = e.stop(&s.ctr.id) {
                // It may have stopped partway: start it again if it was running.
                let err = docker_err(msg!("Could not stop the old container: {error}", error = m));
                return fail(e, r, name, "stop", err, None, Some(&s));
            }
        }
        if let Err(m) = e.set_restart(&s.ctr.id, &no_restart()) {
            return fail(
                e,
                r,
                name,
                "stop",
                docker_err(msg!("Could not change the restart policy of the old container: {error}", error = m)),
                None,
                Some(&s),
            );
        }
        s.restart_changed = true;
        match e.find(&backup) {
            Ok(Some(b)) => {
                r.log(msg!("Removing the older backup container…"));
                if let Err(m) = e.remove(&b.id) {
                    let err = docker_err(msg!("Could not remove the older backup container: {error}", error = m));
                    return fail(e, r, name, "stop", err, None, Some(&s));
                }
            }
            Ok(None) => {}
            Err(m) => return fail(e, r, name, "stop", docker_err(msg!("Docker reported an error: {error}", error = m)), None, Some(&s)),
        }
        if let Err(m) = e.rename(&s.ctr.id, &backup) {
            return fail(
                e,
                r,
                name,
                "stop",
                docker_err(msg!("Could not rename the old container to {name}: {error}", name = &backup, error = m)),
                None,
                Some(&s),
            );
        }
        s.renamed = true;
        r.log(msg!("The old container is now named {name}", name = &backup));
        r.step("stop", "done");
        saved = Some(s);
    } else {
        r.step("stop", "skipped");
    }

    r.step("create", "running");
    let new_id = match e.create(name, body) {
        Ok(id) => id,
        Err(m) => {
            return fail(
                e,
                r,
                name,
                "create",
                docker_err(msg!("Could not create the new container: {error}", error = m)),
                None,
                saved.as_ref(),
            );
        }
    };
    r.log(msg!("Created the new container {id}", id = &new_id[..new_id.len().min(12)]));
    r.step("create", "done");

    r.step("start", "running");
    if let Err(m) = e.start(&new_id) {
        return fail(
            e,
            r,
            name,
            "start",
            docker_err(msg!("Could not start the new container: {error}", error = m)),
            Some(&new_id),
            saved.as_ref(),
        );
    }
    r.step("start", "done");

    r.step("verify", "running");
    r.log(msg!("Watching the container for {seconds} seconds to make sure it keeps running…", seconds = verify_secs));
    if let Err(m) = verify(e, &new_id, verify_secs) {
        return fail(e, r, name, "verify", JobError::new("verify_failed", m), Some(&new_id), saved.as_ref());
    }
    r.step("verify", "done");
    Outcome::Replaced { new_id, previous: saved.map(|s| s.ctr) }
}

/// Swaps the backup container back in: the current container becomes the backup. `backup_restart` is the restart
/// policy the backup had before it was replaced.
pub fn swap_back(e: &mut dyn Engine, r: &mut dyn Report, name: &str, backup_restart: &Restart, verify_secs: u32) -> Outcome {
    let backup = backup_name(name);
    let swap = format!("{name}.hbmgr-swap");
    let (cur, bak) = match (e.find(name), e.find(&backup)) {
        (Ok(c), Ok(Some(b))) => (c, b),
        (Ok(_), Ok(None)) => {
            r.step("stop", "failed");
            return Outcome::Failed(JobError::new("conflict", msg!("There is no backup container to roll back to")));
        }
        (Err(m), _) | (_, Err(m)) => {
            r.step("stop", "failed");
            return Outcome::Failed(docker_err(msg!("Docker reported an error: {error}", error = m)));
        }
    };

    // The current container moves aside under a temporary name.
    let mut saved = None;
    if let Some(c) = cur {
        r.step("stop", "running");
        let mut s = Saved { ctr: c, renamed: false, restart_changed: false };
        if s.ctr.running {
            r.log(msg!("Stopping the current container…"));
            if let Err(m) = e.stop(&s.ctr.id) {
                let err = docker_err(msg!("Could not stop the current container: {error}", error = m));
                return fail(e, r, name, "stop", err, None, Some(&s));
            }
        }
        if let Err(m) = e.set_restart(&s.ctr.id, &no_restart()) {
            return fail(
                e,
                r,
                name,
                "stop",
                docker_err(msg!("Could not change the restart policy of the current container: {error}", error = m)),
                None,
                Some(&s),
            );
        }
        s.restart_changed = true;
        if let Ok(Some(stale)) = e.find(&swap) {
            let _ = e.remove(&stale.id);
        }
        if let Err(m) = e.rename(&s.ctr.id, &swap) {
            return fail(
                e,
                r,
                name,
                "stop",
                docker_err(msg!("Could not rename the current container to {name}: {error}", name = &swap, error = m)),
                None,
                Some(&s),
            );
        }
        s.renamed = true;
        r.step("stop", "done");
        saved = Some(s);
    } else {
        r.step("stop", "skipped");
    }

    // Undo for the backup: back to its own name, restart `no`, stopped.
    let put_backup_back = |e: &mut dyn Engine, r: &mut dyn Report, renamed: bool| {
        let _ = e.stop(&bak.id);
        let _ = e.set_restart(&bak.id, &no_restart());
        if renamed && let Err(m) = e.rename(&bak.id, &backup) {
            r.log(msg!("Could not rename the backup container to {name}: {error}", name = &backup, error = m));
        }
    };
    // The current container's undo renames it from the temporary name.
    let restore_current = |e: &mut dyn Engine, r: &mut dyn Report, step: &str, err: JobError, saved: Option<&Saved>| {
        r.step(step, "failed");
        r.log(err.msg.clone());
        let Some(s) = saved else { return Outcome::Failed(err) };
        r.log(msg!("Restoring the previous container…"));
        if restore(e, r, name, s) { Outcome::RolledBack(err) } else { Outcome::Failed(not_restored(&err.code)) }
    };

    r.step("restore", "running");
    if let Err(m) = e.rename(&bak.id, name) {
        return restore_current(
            e,
            r,
            "restore",
            docker_err(msg!("Could not rename the backup container to {name}: {error}", name = name, error = m)),
            saved.as_ref(),
        );
    }
    if let Err(m) = e.set_restart(&bak.id, backup_restart) {
        put_backup_back(e, r, true);
        return restore_current(
            e,
            r,
            "restore",
            docker_err(msg!("Could not restore the restart policy of the backup container: {error}", error = m)),
            saved.as_ref(),
        );
    }
    r.step("restore", "done");

    r.step("start", "running");
    if let Err(m) = e.start(&bak.id) {
        put_backup_back(e, r, true);
        return restore_current(
            e,
            r,
            "start",
            docker_err(msg!("Could not start the backup container: {error}", error = m)),
            saved.as_ref(),
        );
    }
    r.step("start", "done");

    r.step("verify", "running");
    r.log(msg!("Watching the container for {seconds} seconds to make sure it keeps running…", seconds = verify_secs));
    if let Err(m) = verify(e, &bak.id, verify_secs) {
        let output = e.last_logs(&bak.id, 20);
        if !output.is_empty() {
            r.log(msg!("Last output of the backup container:"));
            for line in output {
                r.raw(&line);
            }
        }
        put_backup_back(e, r, true);
        return restore_current(e, r, "verify", JobError::new("verify_failed", m), saved.as_ref());
    }
    r.step("verify", "done");

    // The replaced container becomes the backup (it already has restart `no`).
    if let Some(s) = &saved
        && let Err(m) = e.rename(&s.ctr.id, &backup)
    {
        r.log(msg!(
            "Could not rename the replaced container to {name}: {error}. It is now named {current}.",
            name = &backup,
            error = m,
            current = &swap
        ));
    }
    Outcome::Replaced { new_id: bak.id.clone(), previous: saved.map(|s| s.ctr) }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Debug, Clone)]
    pub struct FakeCtr {
        pub name: String,
        pub running: bool,
        pub restart: Restart,
        pub dies: bool,
        pub restart_count: i64,
    }

    /// An in-memory Docker that records every call and fails where told to.
    #[derive(Default)]
    pub struct Fake {
        pub ctrs: BTreeMap<String, FakeCtr>,
        pub calls: Vec<String>,
        /// `op:target` that fails (e.g. `create:hb`, `start:new1`, `rename:old->hb`).
        pub fail: Vec<String>,
        /// Containers created from this body exit at once.
        pub dies: bool,
        next: u32,
    }

    impl Fake {
        pub fn with(name: &str, id: &str, running: bool, restart: &str) -> Fake {
            let mut f = Fake::default();
            f.ctrs.insert(
                id.into(),
                FakeCtr {
                    name: name.into(),
                    running,
                    restart: Restart { name: restart.into(), max_retry: 0 },
                    dies: false,
                    restart_count: 0,
                },
            );
            f
        }

        fn check(&mut self, op: &str, target: &str) -> Result<(), String> {
            self.calls.push(format!("{op}:{target}"));
            if self.fail.iter().any(|f| f == &format!("{op}:{target}")) { Err(format!("{op} failed")) } else { Ok(()) }
        }

        pub fn by_name(&self, name: &str) -> Option<(&String, &FakeCtr)> {
            self.ctrs.iter().find(|(_, c)| c.name == name)
        }
    }

    impl Engine for Fake {
        fn find(&mut self, name: &str) -> Result<Option<Ctr>, String> {
            Ok(self.by_name(name).map(|(id, c)| Ctr {
                id: id.clone(),
                running: c.running,
                restart: c.restart.clone(),
                image: String::new(),
                image_id: String::new(),
                labels: Default::default(),
            }))
        }
        fn stop(&mut self, id: &str) -> Result<(), String> {
            self.check("stop", id)?;
            self.ctrs.get_mut(id).ok_or("no such container")?.running = false;
            Ok(())
        }
        fn start(&mut self, id: &str) -> Result<(), String> {
            self.check("start", id)?;
            let c = self.ctrs.get_mut(id).ok_or("no such container")?;
            c.running = !c.dies;
            Ok(())
        }
        fn set_restart(&mut self, id: &str, r: &Restart) -> Result<(), String> {
            self.check("restart", &format!("{id}={}", r.name))?;
            self.ctrs.get_mut(id).ok_or("no such container")?.restart = r.clone();
            Ok(())
        }
        fn rename(&mut self, id: &str, to: &str) -> Result<(), String> {
            self.check("rename", &format!("{id}->{to}"))?;
            if self.by_name(to).is_some() {
                return Err("name in use".into());
            }
            self.ctrs.get_mut(id).ok_or("no such container")?.name = to.into();
            Ok(())
        }
        fn remove(&mut self, id: &str) -> Result<(), String> {
            self.check("remove", id)?;
            self.ctrs.remove(id).map(drop).ok_or_else(|| "no such container".into())
        }
        fn create(&mut self, name: &str, _body: &Value) -> Result<String, String> {
            self.check("create", name)?;
            if self.by_name(name).is_some() {
                return Err("name in use".into());
            }
            self.next += 1;
            let id = format!("new{}", self.next);
            self.ctrs.insert(
                id.clone(),
                FakeCtr { name: name.into(), running: false, restart: Restart::default(), dies: self.dies, restart_count: 0 },
            );
            Ok(id)
        }
        fn state(&mut self, id: &str) -> Result<RunState, String> {
            let c = self.ctrs.get(id).ok_or("no such container")?;
            Ok(RunState { running: c.running, restarting: false, restart_count: c.restart_count, exit_code: 1, started_at: "t".into() })
        }
        fn last_logs(&mut self, _id: &str, _lines: usize) -> Vec<String> {
            vec!["Error: boom".into()]
        }
        fn wait(&mut self, _d: Duration) {}
    }

    #[derive(Default)]
    pub struct Rec {
        pub steps: Vec<(String, String)>,
        pub log: Vec<String>,
    }

    impl Report for Rec {
        fn step(&mut self, key: &str, status: &str) {
            self.steps.push((key.into(), status.into()));
        }
        fn log(&mut self, msg: Msg) {
            self.log.push(msg.english());
        }
        fn raw(&mut self, text: &str) {
            self.log.push(format!("|{text}"));
        }
    }

    fn body() -> Value {
        serde_json::json!({ "Image": "x" })
    }

    #[test]
    fn replaces_a_running_container_and_keeps_it_as_the_backup() {
        let mut f = Fake::with("hb", "old", true, "always");
        let mut r = Rec::default();
        let out = replace(&mut f, &mut r, "hb", &body(), 10);
        assert!(matches!(out, Outcome::Replaced { ref new_id, previous: Some(ref p) } if new_id == "new1" && p.id == "old" && p.running));
        // Stopped, restart set to `no` before the rename, then renamed.
        assert_eq!(f.calls, vec!["stop:old", "restart:old=no", "rename:old->hb.hbmgr-prev", "create:hb", "start:new1"]);
        let old = &f.ctrs["old"];
        assert_eq!((old.name.as_str(), old.running, old.restart.name.as_str()), ("hb.hbmgr-prev", false, "no"));
        assert!(f.ctrs["new1"].running);
        assert_eq!(r.steps.last().unwrap(), &("verify".to_string(), "done".to_string()));
    }

    #[test]
    fn an_older_backup_is_removed_first() {
        let mut f = Fake::with("hb", "old", true, "always");
        f.ctrs.insert(
            "older".into(),
            FakeCtr { name: "hb.hbmgr-prev".into(), running: false, restart: no_restart(), dies: false, restart_count: 0 },
        );
        let out = replace(&mut f, &mut Rec::default(), "hb", &body(), 10);
        assert!(matches!(out, Outcome::Replaced { .. }));
        assert!(!f.ctrs.contains_key("older"));
        assert_eq!(f.calls[..4], ["stop:old", "restart:old=no", "remove:older", "rename:old->hb.hbmgr-prev"]);
    }

    #[test]
    fn creates_when_there_is_nothing_to_replace() {
        let mut f = Fake::default();
        let mut r = Rec::default();
        let out = replace(&mut f, &mut r, "hb", &body(), 10);
        assert!(matches!(out, Outcome::Replaced { previous: None, .. }));
        assert_eq!(r.steps[0], ("stop".to_string(), "skipped".to_string()));
        assert_eq!(f.calls, vec!["create:hb", "start:new1"]);
    }

    #[test]
    fn a_new_container_that_dies_is_rolled_back_exactly() {
        let mut f = Fake::with("hb", "old", true, "unless-stopped");
        f.dies = true;
        let mut r = Rec::default();
        let out = replace(&mut f, &mut r, "hb", &body(), 10);
        let Outcome::RolledBack(err) = out else { panic!("{out:?}") };
        assert_eq!(err.code, "verify_failed");
        assert!(!f.ctrs.contains_key("new1"), "the new container is removed");
        let old = &f.ctrs["old"];
        assert_eq!((old.name.as_str(), old.running, old.restart.name.as_str()), ("hb", true, "unless-stopped"));
        assert_eq!(f.calls[f.calls.len() - 4..], ["remove:new1", "rename:old->hb", "restart:old=unless-stopped", "start:old"]);
        assert!(r.log.iter().any(|l| l == "|Error: boom"), "the new container's last output is logged untranslated");
        assert!(r.steps.contains(&("verify".to_string(), "failed".to_string())));
    }

    #[test]
    fn a_stopped_container_stays_stopped_after_a_rollback() {
        let mut f = Fake::with("hb", "old", false, "always");
        f.fail.push("create:hb".into());
        let out = replace(&mut f, &mut Rec::default(), "hb", &body(), 10);
        assert!(matches!(out, Outcome::RolledBack(_)));
        assert_eq!(f.calls, vec!["restart:old=no", "rename:old->hb.hbmgr-prev", "create:hb", "rename:old->hb", "restart:old=always"]);
        assert!(!f.ctrs["old"].running);
    }

    #[test]
    fn a_failed_start_removes_the_new_container() {
        let mut f = Fake::with("hb", "old", true, "always");
        f.fail.push("start:new1".into());
        let out = replace(&mut f, &mut Rec::default(), "hb", &body(), 10);
        assert!(matches!(out, Outcome::RolledBack(ref e) if e.message.starts_with("Could not start the new container")));
        assert!(!f.ctrs.contains_key("new1"));
        assert_eq!(f.ctrs["old"].name, "hb");
        assert!(f.ctrs["old"].running);
    }

    #[test]
    fn a_failed_rename_puts_the_old_container_back() {
        let mut f = Fake::with("hb", "old", true, "always");
        f.fail.push("rename:old->hb.hbmgr-prev".into());
        let out = replace(&mut f, &mut Rec::default(), "hb", &body(), 10);
        assert!(matches!(out, Outcome::RolledBack(_)));
        assert_eq!(f.calls, vec!["stop:old", "restart:old=no", "rename:old->hb.hbmgr-prev", "restart:old=always", "start:old"]);
    }

    #[test]
    fn without_an_old_container_a_failure_is_just_a_failure() {
        let mut f = Fake { dies: true, ..Default::default() };
        let out = replace(&mut f, &mut Rec::default(), "hb", &body(), 10);
        assert!(matches!(out, Outcome::Failed(ref e) if e.code == "verify_failed"));
        assert!(f.ctrs.is_empty());
    }

    #[test]
    fn a_failed_restore_is_reported() {
        let mut f = Fake::with("hb", "old", true, "always");
        f.fail.push("create:hb".into());
        f.fail.push("start:old".into());
        let out = replace(&mut f, &mut Rec::default(), "hb", &body(), 10);
        let Outcome::Failed(e) = out else { panic!() };
        assert_eq!(e.code, "docker_error");
        assert!(e.message.contains("could not be fully restored"), "{}", e.message);
    }

    #[test]
    fn swaps_the_backup_back_in() {
        let mut f = Fake::with("hb", "cur", true, "always");
        f.ctrs.insert(
            "bak".into(),
            FakeCtr { name: "hb.hbmgr-prev".into(), running: false, restart: no_restart(), dies: false, restart_count: 0 },
        );
        let mut r = Rec::default();
        let original = Restart { name: "unless-stopped".into(), max_retry: 0 };
        let out = swap_back(&mut f, &mut r, "hb", &original, 10);
        assert!(matches!(out, Outcome::Replaced { ref new_id, previous: Some(ref p) } if new_id == "bak" && p.id == "cur"));
        let (bak, cur) = (&f.ctrs["bak"], &f.ctrs["cur"]);
        assert_eq!((bak.name.as_str(), bak.running, bak.restart.name.as_str()), ("hb", true, "unless-stopped"));
        assert_eq!((cur.name.as_str(), cur.running, cur.restart.name.as_str()), ("hb.hbmgr-prev", false, "no"));
    }

    #[test]
    fn a_backup_that_dies_is_swapped_out_again() {
        let mut f = Fake::with("hb", "cur", true, "always");
        f.ctrs.insert(
            "bak".into(),
            FakeCtr { name: "hb.hbmgr-prev".into(), running: false, restart: no_restart(), dies: true, restart_count: 0 },
        );
        let out = swap_back(&mut f, &mut Rec::default(), "hb", &Restart::default(), 10);
        assert!(matches!(out, Outcome::RolledBack(_)));
        let (bak, cur) = (&f.ctrs["bak"], &f.ctrs["cur"]);
        assert_eq!((bak.name.as_str(), bak.running, bak.restart.name.as_str()), ("hb.hbmgr-prev", false, "no"));
        assert_eq!((cur.name.as_str(), cur.running, cur.restart.name.as_str()), ("hb", true, "always"));
    }

    #[test]
    fn no_backup_no_swap() {
        let mut f = Fake::with("hb", "cur", true, "always");
        let out = swap_back(&mut f, &mut Rec::default(), "hb", &Restart::default(), 10);
        assert!(matches!(out, Outcome::Failed(ref e) if e.code == "conflict"));
        assert!(f.calls.is_empty());
    }

    #[test]
    fn verify_notices_restarts() {
        let mut f = Fake::with("hb", "c", true, "always");
        assert!(verify(&mut f, "c", 3).is_ok());
        struct Restarting(Fake, u32);
        impl Engine for Restarting {
            fn find(&mut self, n: &str) -> Result<Option<Ctr>, String> {
                self.0.find(n)
            }
            fn stop(&mut self, id: &str) -> Result<(), String> {
                self.0.stop(id)
            }
            fn start(&mut self, id: &str) -> Result<(), String> {
                self.0.start(id)
            }
            fn set_restart(&mut self, id: &str, r: &Restart) -> Result<(), String> {
                self.0.set_restart(id, r)
            }
            fn rename(&mut self, id: &str, to: &str) -> Result<(), String> {
                self.0.rename(id, to)
            }
            fn remove(&mut self, id: &str) -> Result<(), String> {
                self.0.remove(id)
            }
            fn create(&mut self, n: &str, b: &Value) -> Result<String, String> {
                self.0.create(n, b)
            }
            fn state(&mut self, id: &str) -> Result<RunState, String> {
                self.1 += 1;
                let mut s = self.0.state(id)?;
                s.restart_count = i64::from(self.1 > 2);
                Ok(s)
            }
            fn last_logs(&mut self, id: &str, n: usize) -> Vec<String> {
                self.0.last_logs(id, n)
            }
            fn wait(&mut self, _d: Duration) {}
        }
        let mut r = Restarting(Fake::with("hb", "c", true, "always"), 0);
        assert_eq!(verify(&mut r, "c", 5).unwrap_err(), msg!("The container keeps restarting"));
    }
}

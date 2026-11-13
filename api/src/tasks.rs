//! What the background worker does for each kind of job.

use crate::docker::Docker;
use crate::jobs::{self, JobError};
use crate::msg::Msg;
use crate::ops::{self, Ctr, Engine, Outcome, Report, RunState};
use crate::pull::PullProgress;
use crate::spec::{self, LABEL_CONFIG, LABEL_INSTANCE, Spec};
use crate::store::{BackupRecord, Restart, Store};
use crate::{cleanup, discover, instance, net, paths, sys, updates, util};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Duration;

/// How long a new container must keep running to count as working.
const VERIFY_SECS: u32 = 10;

/// Docker as the replace primitive sees it.
pub struct DockerEngine<'a> {
    pub d: &'a Docker,
}

fn restart_of(inspect: &Value) -> Restart {
    let name = inspect["HostConfig"]["RestartPolicy"]["Name"].as_str().unwrap_or("");
    Restart {
        name: if name.is_empty() { "no".into() } else { name.into() },
        max_retry: inspect["HostConfig"]["RestartPolicy"]["MaximumRetryCount"].as_i64().unwrap_or(0),
    }
}

impl Engine for DockerEngine<'_> {
    fn find(&mut self, name: &str) -> Result<Option<Ctr>, String> {
        let Some(l) = self.d.find(name).map_err(|e| e.detail())? else { return Ok(None) };
        let Some(i) = self.d.inspect(&l.id).map_err(|e| e.detail())? else { return Ok(None) };
        Ok(Some(Ctr {
            id: l.id,
            running: i["State"]["Running"].as_bool().unwrap_or(false),
            restart: restart_of(&i),
            image: i["Config"]["Image"].as_str().unwrap_or("").to_string(),
            image_id: i["Image"].as_str().unwrap_or("").to_string(),
            labels: i["Config"]["Labels"].as_object().cloned().unwrap_or_default(),
        }))
    }
    fn stop(&mut self, id: &str) -> Result<(), String> {
        self.d.stop(id).map_err(|e| e.detail())
    }
    fn start(&mut self, id: &str) -> Result<(), String> {
        self.d.start(id).map_err(|e| e.detail())
    }
    fn set_restart(&mut self, id: &str, r: &Restart) -> Result<(), String> {
        self.d.set_restart(id, &r.name, r.max_retry).map_err(|e| e.detail())
    }
    fn rename(&mut self, id: &str, to: &str) -> Result<(), String> {
        self.d.rename(id, to).map_err(|e| e.detail())
    }
    fn remove(&mut self, id: &str) -> Result<(), String> {
        self.d.remove(id).map_err(|e| e.detail())
    }
    fn create(&mut self, name: &str, body: &Value) -> Result<String, String> {
        self.d.create(name, body).map_err(|e| e.detail())
    }
    fn state(&mut self, id: &str) -> Result<RunState, String> {
        let i = self.d.inspect(id).map_err(|e| e.detail())?.ok_or("the container is gone")?;
        let st = &i["State"];
        Ok(RunState {
            running: st["Running"].as_bool().unwrap_or(false),
            restarting: st["Restarting"].as_bool().unwrap_or(false),
            restart_count: i["RestartCount"].as_i64().unwrap_or(0),
            exit_code: st["ExitCode"].as_i64().unwrap_or(0),
            started_at: st["StartedAt"].as_str().unwrap_or("").to_string(),
        })
    }
    fn last_logs(&mut self, id: &str, lines: usize) -> Vec<String> {
        let tty = self.d.inspect(id).ok().flatten().is_some_and(|i| i["Config"]["Tty"].as_bool() == Some(true));
        self.d.logs(id, lines, tty).map(|b| crate::docker::log_lines(&b)).unwrap_or_default()
    }
    fn wait(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

impl Report for jobs::Ctx<'_> {
    fn step(&mut self, key: &str, status: &str) {
        jobs::Ctx::step(self, key, status);
    }
    fn log(&mut self, msg: Msg) {
        jobs::Ctx::log(self, msg);
    }
    fn raw(&mut self, text: &str) {
        jobs::Ctx::raw(self, text);
    }
}

fn no_instance() -> Msg {
    msg!("The task does not name an instance")
}

/// How a job ended.
struct Fin {
    status: &'static str,
    result: Value,
    error: Option<JobError>,
}

fn failed(code: &str, msg: Msg) -> Fin {
    Fin { status: "failed", result: Value::Null, error: Some(JobError::new(code, msg)) }
}

/// Runs the job in `ctx` to its end.
pub fn run(ctx: &mut jobs::Ctx) {
    let docker = Docker::from_env();
    let kind = ctx.file.job.kind.clone();
    ctx.log(msg!("Task started"));
    let fin = match kind.as_str() {
        "create" | "apply" | "upgrade" => recreate(ctx, &docker, &kind),
        "rollback" => rollback(ctx, &docker),
        "remove" => remove(ctx, &docker),
        "check_updates" => check_updates(ctx, &docker),
        _ => failed("internal", msg!("This kind of task is not known")),
    };
    if let Some(e) = &fin.error
        && !ctx.has_logged(&e.msg)
    {
        ctx.log(e.msg.clone());
    }
    ctx.log(match fin.status {
        "succeeded" => msg!("Done"),
        "rolled_back" => msg!("The change was undone and the previous container is back"),
        _ => msg!("The task failed"),
    });
    ctx.finish(fin.status, fin.result, fin.error);
}

/// The spec's `restart` for a container's own restart policy (`on-failure` has no spec value).
pub fn spec_restart(r: &Restart) -> String {
    match r.name.as_str() {
        "always" => "always",
        "unless-stopped" | "on-failure" => "unless-stopped",
        _ => "no",
    }
    .into()
}

/// The spec a container was created from: its own label when this package created it for `name`, else what can be
/// read from it. `restart` is the container's policy before it became a backup (a backup's own policy is `no`).
fn spec_of_container(docker: &Docker, id: &str, name: &str, base: &Spec, restart: &Restart) -> Option<Spec> {
    let i = docker.inspect(id).ok().flatten()?;
    let labels = &i["Config"]["Labels"];
    let mut s =
        match labels[LABEL_CONFIG].as_str().filter(|_| labels[LABEL_INSTANCE].as_str() == Some(name)).and_then(|c| base.with_config(c)) {
            Some(s) => s,
            None => {
                let image_env = image_env(docker, &i);
                let nets = docker.all_networks().unwrap_or_default();
                let ifaces = net::interfaces().unwrap_or_default();
                let (mut s, _) = discover::to_spec(&i, &image_env, &nets, &ifaces, &sys::timezone());
                s.name = name.to_string();
                s.display_name = base.display_name.clone();
                s.emoji = base.emoji.clone();
                s.color = base.color.clone();
                s.domain = base.domain.clone();
                s
            }
        };
    s.restart = spec_restart(restart);
    Some(s)
}

/// The environment the image of a container sets itself.
pub fn image_env(docker: &Docker, inspect: &Value) -> Vec<String> {
    let image = inspect["Image"].as_str().or_else(|| inspect["Config"]["Image"].as_str()).unwrap_or("");
    docker
        .inspect_image_anywhere(image)
        .ok()
        .flatten()
        .and_then(|i| i["Config"]["Env"].as_array().cloned())
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn backup_record(docker: &Docker, prev: &Ctr, name: &str, base: &Spec) -> BackupRecord {
    BackupRecord {
        container_id: prev.id.clone(),
        image: prev.image.clone(),
        image_id: prev.image_id.clone(),
        created_at: util::now_rfc3339(),
        restart: prev.restart.clone(),
        was_running: prev.running,
        spec: spec_of_container(docker, &prev.id, name, base, &prev.restart),
    }
}

fn pull(ctx: &mut jobs::Ctx, docker: &Docker, image: &str) -> Result<(), JobError> {
    ctx.log(msg!("Downloading {image}…", image = image));
    let mut p = PullProgress::default();
    let r = docker.pull(image, |msg| {
        p.feed(msg);
        ctx.progress(p.percent(), Some(p.detail()));
    });
    match r {
        Ok(()) => {
            match &p.status {
                Some(s) if s.contains("up to date") => ctx.log(msg!("{image} is up to date", image = image)),
                _ => ctx.log(msg!("Downloaded {image}", image = image)),
            }
            Ok(())
        }
        Err(e) => Err(JobError::new("docker_error", msg!("Could not download the image: {error}", error = e.detail()))),
    }
}

/// create, apply and upgrade: (pull,) replace the container from the spec, keep the old one as the backup.
fn recreate(ctx: &mut jobs::Ctx, docker: &Docker, kind: &str) -> Fin {
    let store: &Store = ctx.store;
    let Some(name) = ctx.file.job.instance.clone() else { return failed("internal", no_instance()) };
    let Some(rec) = store.record(&name) else { return failed("not_found", msg!("This instance could not be found")) };
    let params = ctx.file.params.clone();
    let new_record = params["new_record"].as_bool().unwrap_or(false);
    let give_up = |ctx: &mut jobs::Ctx, fin: Fin| {
        if new_record {
            let _ = ctx.store.delete_record(&name);
            ctx.log(msg!("The new instance was not added"));
        }
        fin
    };
    let mut spec = rec.spec.clone();
    if kind == "upgrade"
        && let Some(img) = params["image"].as_str().filter(|i| !i.is_empty())
    {
        if img != spec.image {
            ctx.log(msg!("Switching the image to {image}", image = img));
        }
        spec.image = img.to_string();
    }

    // 1. The image.
    ctx.step("pull", "running");
    let had_image = match docker.inspect_image(&spec.image) {
        Ok(i) => i.is_some(),
        Err(e) => return give_up(ctx, failed("docker_unavailable", e.msg())),
    };
    if kind == "upgrade" || !had_image {
        if let Err(e) = pull(ctx, docker, &spec.image) {
            ctx.step("pull", "failed");
            return give_up(ctx, Fin { status: "failed", result: Value::Null, error: Some(e) });
        }
        updates::note_pulled(store, docker, &spec.image);
        ctx.step("pull", "done");
    } else {
        ctx.log(msg!("Using the image {image} that is already on the NAS", image = &spec.image));
        ctx.step("pull", "skipped");
    }
    let image_id = docker.inspect_image(&spec.image).ok().flatten().and_then(|i| i["Id"].as_str().map(str::to_string)).unwrap_or_default();

    // An upgrade that brought nothing new leaves the container alone (unless it still has to move to system-docker).
    if kind == "upgrade"
        && let Ok(Some(cur)) = docker.find(&name)
        && instance::is_managed(&cur, &name)
        && !cur.in_cs
        && cur.image_id == image_id
        && cur.label(spec::LABEL_SPEC) == Some(spec.hash().as_str())
    {
        ctx.log(msg!("There is no newer image, so the container does not need to be recreated"));
        for step in ["stop", "create", "start", "verify"] {
            ctx.step(step, "skipped");
        }
        cleanup_images(ctx, docker, &[]);
        return Fin { status: "succeeded", result: json!({ "changed": false }), error: None };
    }
    // The image of the backup that this replace removes (an upgrade cleans it up when nothing uses it any more).
    let replaced_backup_image: Vec<String> = if kind == "upgrade" {
        docker.find(&spec::backup_name(&name)).ok().flatten().filter(|b| !b.in_cs).map(|b| b.image_id).into_iter().collect()
    } else {
        Vec::new()
    };

    // 2. What the container needs: the work directory, the network, a free address.
    match paths::ensure_workdir(&spec.workdir) {
        Ok(true) => ctx.log(msg!("Created the work folder {path}", path = &spec.workdir)),
        Ok(false) => {}
        Err(e) => {
            ctx.step("create", "failed");
            return give_up(ctx, failed("internal", msg!("Could not create the work folder: {error}", error = e.to_string())));
        }
    }
    let network = match ensure_network(ctx, docker, &spec) {
        Ok(n) => n,
        Err(e) => {
            ctx.step("create", "failed");
            return give_up(ctx, Fin { status: "failed", result: Value::Null, error: Some(e) });
        }
    };
    if spec.network.mode == "static"
        && let Some(user) = address_taken(docker, &name, &spec.network.ip)
    {
        ctx.step("create", "failed");
        let taken = msg!("The address {ip} is already used by container {name}", ip = &spec.network.ip, name = &user);
        return give_up(ctx, failed("conflict", taken));
    }
    if spec.network.mac.is_empty() {
        spec.network.mac = spec::random_mac(&|_| false);
    }

    // 3. Replace.
    let body = spec.create_body(&network);
    let mut engine = DockerEngine { d: docker };
    match ops::replace(&mut engine, ctx, &name, &body, VERIFY_SECS) {
        Outcome::Replaced { new_id, previous } => {
            let mut rec = store.record(&name).unwrap_or(rec);
            let old_spec = rec.spec.clone();
            rec.spec = spec;
            if let Some(prev) = &previous {
                rec.backup = Some(backup_record(docker, prev, &name, &old_spec));
            }
            if let Err(e) = store.save_record(&mut rec) {
                ctx.log(msg!("Could not save the instance settings: {error}", error = e.to_string()));
            }
            if kind == "upgrade" {
                cleanup_images(ctx, docker, &replaced_backup_image);
            }
            let short: String = new_id.chars().take(12).collect();
            Fin { status: "succeeded", result: json!({ "changed": true, "container_id": short }), error: None }
        }
        Outcome::RolledBack(e) => give_up(ctx, Fin { status: "rolled_back", result: Value::Null, error: Some(e) }),
        Outcome::Failed(e) => give_up(ctx, Fin { status: "failed", result: Value::Null, error: Some(e) }),
    }
}

/// The last step of an upgrade: removes the Homebridge images on system-docker that nothing uses any more (the rules
/// are in `cleanup.rs`). `also` are image ids that may go even when they are not Homebridge's (the replaced backup's).
/// What fails is logged; it never changes how the task ends.
fn cleanup_images(ctx: &mut jobs::Ctx, docker: &Docker, also: &[String]) {
    ctx.step("cleanup", "running");
    let (images, list) = match (docker.images(), docker.list_containers()) {
        (Ok(i), Ok(l)) => (i, l),
        (Err(e), _) | (_, Err(e)) => {
            ctx.log(msg!("Could not list the images: {error}", error = e.detail()));
            ctx.step("cleanup", "done");
            return;
        }
    };
    let used: HashSet<String> = list.into_iter().filter(|c| !c.in_cs).map(|c| c.image_id).collect();
    let gone = cleanup::leftovers(&images, &used, also);
    if gone.is_empty() {
        ctx.log(msg!("There are no unused images to remove"));
    }
    for image in gone {
        match docker.remove_image(&image.id) {
            Ok(()) => ctx.log(msg!("Removed the unused image {image}", image = &image.label)),
            Err(e) if e.is_not_found() => {}
            Err(e) => ctx.log(msg!("Could not remove the image {image}: {error}", image = &image.label, error = e.detail())),
        }
    }
    ctx.step("cleanup", "done");
}

/// The name of a container (other than this instance's own and its backup) configured with or holding `ip`.
fn address_taken(docker: &Docker, name: &str, ip: &str) -> Option<String> {
    let backup = spec::backup_name(name);
    let list = docker.list_containers().ok()?;
    list.iter()
        .filter(|c| c.name != name && c.name != backup && !c.name.starts_with(&format!("{name}.hbmgr-")))
        .find(|c| c.endpoints().iter().any(|(_, conf, cur, _)| conf == ip || cur == ip))
        .map(|c| c.name.clone())
}

/// The qnet network for the spec: an existing one that fits, or a new one.
fn ensure_network(ctx: &mut jobs::Ctx, docker: &Docker, spec: &Spec) -> Result<String, JobError> {
    let ifaces = net::interfaces().map_err(|e| JobError::new(e.code, e.msg))?;
    let iface = ifaces.iter().find(|i| i.iface == spec.network.iface).ok_or_else(|| {
        JobError::new("validation", msg!("The network interface {iface} was not found. Choose one again.", iface = &spec.network.iface))
    })?;
    let nets = docker.networks().map_err(|e| JobError::new("docker_error", e.msg()))?;
    if let Some(n) = net::pick_network(&nets, &spec.network.mode, &iface.iface, iface.subnet.as_deref()) {
        ctx.log(msg!("Using the network {name}", name = &n));
        return Ok(n);
    }
    let (name, body) = net::network_create(&spec.network.mode, iface);
    ctx.log(msg!("Creating the network {name}", name = &name));
    docker
        .create_network(&body)
        .map_err(|e| JobError::new("docker_error", msg!("Could not create the network: {error}", error = e.detail())))?;
    Ok(name)
}

/// rollback: the backup container takes the name back; the current one becomes the backup.
fn rollback(ctx: &mut jobs::Ctx, docker: &Docker) -> Fin {
    let store: &Store = ctx.store;
    let Some(name) = ctx.file.job.instance.clone() else { return failed("internal", no_instance()) };
    let Some(rec) = store.record(&name) else { return failed("not_found", msg!("This instance could not be found")) };
    let backup_name = spec::backup_name(&name);
    let bak = match docker.find(&backup_name) {
        Ok(Some(b)) => b,
        Ok(None) => return failed("conflict", msg!("There is no backup container to roll back to")),
        Err(e) => return failed("docker_unavailable", e.msg()),
    };
    let recorded = rec.backup.clone().filter(|b| b.container_id == bak.id);
    let restart = match &recorded {
        Some(r) => r.restart.clone(),
        None => {
            // Not recorded here: the backup's own spec label says how it was created.
            let label_restart = bak
                .label(LABEL_CONFIG)
                .and_then(|c| serde_json::from_str::<Value>(c).ok())
                .and_then(|v| v["restart"].as_str().map(str::to_string));
            Restart { name: label_restart.unwrap_or_else(|| "always".into()), max_retry: 0 }
        }
    };
    let back_spec =
        recorded.as_ref().and_then(|r| r.spec.clone()).or_else(|| spec_of_container(docker, &bak.id, &name, &rec.spec, &restart)).map(
            |mut s| {
                // What is not part of the container stays as the user has it now.
                s.name = name.clone();
                s.display_name = rec.spec.display_name.clone();
                s.emoji = rec.spec.emoji.clone();
                s.color = rec.spec.color.clone();
                s.domain = rec.spec.domain.clone();
                s
            },
        );
    let mut engine = DockerEngine { d: docker };
    match ops::swap_back(&mut engine, ctx, &name, &restart, VERIFY_SECS) {
        Outcome::Replaced { previous, .. } => {
            let mut rec = store.record(&name).unwrap_or(rec);
            let current_spec = rec.spec.clone();
            if let Some(s) = back_spec {
                rec.spec = s;
            }
            rec.backup = previous.as_ref().map(|p| backup_record(docker, p, &name, &current_spec));
            if let Err(e) = store.save_record(&mut rec) {
                ctx.log(msg!("Could not save the instance settings: {error}", error = e.to_string()));
            }
            Fin { status: "succeeded", result: json!({ "changed": true }), error: None }
        }
        Outcome::RolledBack(e) => Fin { status: "rolled_back", result: Value::Null, error: Some(e) },
        Outcome::Failed(e) => Fin { status: "failed", result: Value::Null, error: Some(e) },
    }
}

/// remove: the container (when this package created it) and the backup go; the record goes; folders stay.
fn remove(ctx: &mut jobs::Ctx, docker: &Docker) -> Fin {
    let store: &Store = ctx.store;
    let Some(name) = ctx.file.job.instance.clone() else { return failed("internal", no_instance()) };
    ctx.step("stop", "running");
    let cur = match docker.find(&name) {
        Ok(c) => c,
        Err(e) => {
            ctx.step("stop", "failed");
            return failed("docker_unavailable", e.msg());
        }
    };
    match cur {
        Some(c) if instance::is_managed(&c, &name) => {
            if (c.state == "running" || c.state == "restarting")
                && let Err(e) = docker.stop(&c.id)
            {
                ctx.log(msg!("Could not stop the container: {error}", error = e.detail()));
            }
            ctx.step("stop", "done");
            ctx.step("remove", "running");
            if let Err(e) = docker.remove(&c.id) {
                ctx.step("remove", "failed");
                return failed("docker_error", msg!("Could not remove the container: {error}", error = e.detail()));
            }
            ctx.log(msg!("Removed the container {name}", name = &name));
            ctx.step("remove", "done");
        }
        Some(_) => {
            ctx.log(msg!("The container {name} was not created by this app, so it is left as it is", name = &name));
            ctx.step("stop", "skipped");
            ctx.step("remove", "skipped");
        }
        None => {
            ctx.step("stop", "skipped");
            ctx.step("remove", "skipped");
        }
    }
    ctx.step("backup", "running");
    match docker.find(&spec::backup_name(&name)) {
        Ok(Some(b)) => {
            if let Err(e) = docker.remove(&b.id) {
                ctx.step("backup", "failed");
                return failed("docker_error", msg!("Could not remove the backup container: {error}", error = e.detail()));
            }
            ctx.log(msg!("Removed the backup container"));
            ctx.step("backup", "done");
        }
        _ => ctx.step("backup", "skipped"),
    }
    if let Err(e) = store.delete_record(&name) {
        return failed("internal", msg!("Could not delete the instance settings: {error}", error = e.to_string()));
    }
    ctx.log(msg!("The work folder and the other folders are left as they are"));
    Fin { status: "succeeded", result: json!({ "removed": true }), error: None }
}

fn check_updates(ctx: &mut jobs::Ctx, docker: &Docker) -> Fin {
    ctx.step("check", "running");
    if let Err(e) = docker.version() {
        ctx.step("check", "failed");
        return failed("docker_unavailable", e.msg());
    }
    let store: &Store = ctx.store;
    let u = updates::check_all(store, docker, |image, i, n| {
        ctx.log(msg!("Checking {image}", image = image));
        let detail = msg!("Checking {image} ({done} of {count})", image = image, done = i + 1, count = n);
        ctx.progress(Some((i * 100 / n.max(1)) as u8), Some(detail));
    });
    for i in &u.images {
        match (&i.error_msg, i.available) {
            (Some(error), _) => ctx.log(error.clone()),
            (None, true) => ctx.log(msg!("A newer version of {image} is available", image = &i.image)),
            (None, false) => ctx.log(msg!("{image} is up to date", image = &i.image)),
        }
    }
    ctx.step("check", "done");
    let available = u.images.iter().filter(|i| i.available).count();
    Fin { status: "succeeded", result: json!({ "checked": u.images.len(), "available": available }), error: None }
}

/// `hbmgr job-run <id>`: the detached worker.
pub fn worker(id: &str) -> i32 {
    let store = Store::open();
    let Some(jf) = jobs::load(&store, id) else {
        eprintln!("hbmgr: no job {id}");
        return 1;
    };
    if jf.job.status != "running" {
        eprintln!("hbmgr: job {id} is not running");
        return 1;
    }
    let Some(_lock) = jobs::worker_lock(&store, &jf.lock) else {
        eprintln!("hbmgr: job {id}: lock {} is not ours", jf.lock);
        let mut ctx = jobs::Ctx::new(&store, jf);
        ctx.finish(
            "failed",
            Value::Null,
            Some(JobError::new("busy", msg!("Another task is running for this instance. Try again when it has finished."))),
        );
        return 1;
    };
    let mut ctx = jobs::Ctx::new(&store, jf);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut ctx)));
    if r.is_err() {
        let error = msg!("The background task ran into an unexpected error");
        ctx.log(error.clone());
        ctx.finish("failed", Value::Null, Some(JobError::new("internal", error)));
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_specs_keep_the_original_restart_policy() {
        let r = |n: &str| Restart { name: n.into(), max_retry: 0 };
        assert_eq!(spec_restart(&r("always")), "always");
        assert_eq!(spec_restart(&r("unless-stopped")), "unless-stopped");
        assert_eq!(spec_restart(&r("on-failure")), "unless-stopped");
        assert_eq!(spec_restart(&r("no")), "no");
        assert_eq!(spec_restart(&r("")), "no");
    }
}

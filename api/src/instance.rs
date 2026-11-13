//! The Instance object: a record plus what Docker says about its container, its backup, its image and its work
//! directory.

use crate::docker::{Docker, Listed};
use crate::error::ApiError;
use crate::spec::{self, LABEL_INSTANCE, LABEL_SPEC};
use crate::store::{Record, Store, Updates};
use crate::util::{docker_time, rfc3339};
use crate::{hbinfo, jobs, updates};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Image inspects, fetched once per request.
#[derive(Default)]
pub struct Images {
    seen: HashMap<(String, bool), Option<Value>>,
}

impl Images {
    /// An image on system-docker (where containers are created).
    pub fn get(&mut self, docker: &Docker, image: &str) -> Option<Value> {
        self.fetch(docker, image, false)
    }

    /// An image on either daemon (the image a container still on Container Station's own daemon runs).
    pub fn get_anywhere(&mut self, docker: &Docker, image: &str) -> Option<Value> {
        self.fetch(docker, image, true)
    }

    fn fetch(&mut self, docker: &Docker, image: &str, anywhere: bool) -> Option<Value> {
        if image.is_empty() {
            return None;
        }
        self.seen
            .entry((image.to_string(), anywhere))
            .or_insert_with(|| if anywhere { docker.inspect_image_anywhere(image) } else { docker.inspect_image(image) }.ok().flatten())
            .clone()
    }
}

/// `State.Status` as the API names it.
pub fn status_name(s: &str) -> &'static str {
    match s {
        "running" => "running",
        "restarting" => "restarting",
        "paused" => "paused",
        "dead" => "dead",
        _ => "stopped",
    }
}

/// The container's endpoint on its network: (network, current IPv4, MAC).
pub fn endpoint(inspect: &Value) -> Option<(String, String, String)> {
    let nets = inspect["NetworkSettings"]["Networks"].as_object()?;
    let mode = inspect["HostConfig"]["NetworkMode"].as_str().unwrap_or("");
    let (name, ep) = nets.get_key_value(mode).or_else(|| nets.iter().next())?;
    Some((name.clone(), ep["IPAddress"].as_str().unwrap_or("").to_string(), ep["MacAddress"].as_str().unwrap_or("").to_string()))
}

pub fn is_managed(c: &Listed, name: &str) -> bool {
    c.label(LABEL_INSTANCE) == Some(name)
}

/// Builds the Instance object.
pub fn view(store: &Store, docker: &Docker, rec: &Record, list: &[Listed], upd: &Updates, images: &mut Images) -> Value {
    let s = &rec.spec;
    let ctr = list.iter().find(|c| c.name == s.name);
    let managed = ctr.filter(|c| is_managed(c, &s.name));
    let inspect = managed.and_then(|c| docker.inspect(&c.id).ok().flatten());

    let mut ip_now: Option<String> = None;
    let state = match &inspect {
        Some(i) => {
            let st = &i["State"];
            let running = st["Running"].as_bool().unwrap_or(false);
            let (net, ip, mac) = endpoint(i).unwrap_or_default();
            if running && !ip.is_empty() {
                ip_now = Some(ip.clone());
            }
            let image_id = i["Image"].as_str().unwrap_or("").to_string();
            let img = images.get_anywhere(docker, &image_id);
            let created = img.as_ref().and_then(|m| m["Created"].as_str()).and_then(docker_time);
            let version = img
                .as_ref()
                .and_then(|m| m["Config"]["Labels"]["org.opencontainers.image.version"].as_str())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .or_else(|| created.as_ref().map(|c| c[..10].to_string()));
            json!({
                "status": status_name(st["Status"].as_str().unwrap_or("")),
                "foreign": false,
                "container_id": i["Id"].as_str().map(|id| id.chars().take(12).collect::<String>()),
                "started_at": st["StartedAt"].as_str().and_then(docker_time),
                "finished_at": if running { None } else { st["FinishedAt"].as_str().and_then(docker_time) },
                "exit_code": if running { None } else { st["ExitCode"].as_i64() },
                "restart_count": i["RestartCount"].as_i64().unwrap_or(0),
                "ip": ip_now,
                "mac": if running && !mac.is_empty() { Some(mac) } else { None },
                "network_name": if net.is_empty() { None } else { Some(net) },
                "image_id": image_id,
                "image_created": created,
                "image_version": version,
                "in_container_station": managed.is_some_and(|c| c.in_cs),
            })
        }
        None => json!({
            "status": "missing",
            "foreign": ctr.is_some(),
            "container_id": null,
            "started_at": null,
            "finished_at": null,
            "exit_code": null,
            "restart_count": null,
            "ip": null,
            "mac": null,
            "network_name": null,
            "image_id": null,
            "image_created": null,
            "image_version": null,
            "in_container_station": false,
        }),
    };

    // A container still on Container Station's own daemon is recreated on system-docker by an apply.
    let pending = managed.is_none_or(|c| c.in_cs || c.label(LABEL_SPEC) != Some(s.hash().as_str()));
    let running_image = inspect.as_ref().and_then(|i| i["Image"].as_str()).map(str::to_string);
    let local_image = images.get(docker, &s.image).and_then(|i| i["Id"].as_str().map(str::to_string));
    let update = updates::instance_update(upd, &s.image, local_image.as_deref(), running_image.as_deref());

    let backup_name = spec::backup_name(&s.name);
    let backup = list.iter().find(|c| c.name == backup_name).map(|b| {
        let recorded = rec.backup.as_ref().filter(|r| r.container_id == b.id);
        json!({
            "container_name": backup_name,
            "image": recorded.map(|r| r.image.clone()).filter(|i| !i.is_empty()).unwrap_or_else(|| b.image.clone()),
            "image_id": b.image_id,
            "created_at": recorded.map(|r| r.created_at.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| rfc3339(b.created)),
        })
    });

    json!({
        "spec": s,
        "created_at": rec.created_at,
        "updated_at": rec.updated_at,
        "state": state,
        "pending_changes": pending,
        "update": update,
        "backup": backup,
        "homebridge": hbinfo::info(&s.workdir, &s.domain, ip_now.as_deref()),
        "job": jobs::holder(store, &jobs::instance_key(&s.name)),
    })
}

/// One instance, freshly read.
pub fn view_one(store: &Store, docker: &Docker, name: &str) -> Result<Value, ApiError> {
    let rec = store.record(name).ok_or_else(|| ApiError::not_found(msg!("This instance could not be found")))?;
    let list = docker.list_containers()?;
    Ok(view(store, docker, &rec, &list, &store.updates(), &mut Images::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_endpoint_of_the_network_mode() {
        let i: Value = serde_json::from_str(include_str!("../testdata/inspect-homebridge.json")).unwrap();
        assert_eq!(endpoint(&i), Some(("qnet-static-bond0-3f9a2c".into(), "192.168.10.7".into(), "02:42:5e:7a:21:0c".into())));
        assert_eq!(status_name("exited"), "stopped");
        assert_eq!(status_name("created"), "stopped");
        assert_eq!(status_name("running"), "running");
    }
}

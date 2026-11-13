//! Image updates: the registry's digest (asked through Docker, `GET /distribution/<image>/json`) against the digest
//! of the local image the tag points at. Checks run only when asked (the UI asks when its page opens); the result is
//! cached in `state/updates.json`.

use crate::docker::Docker;
use crate::spec;
use crate::store::{ImageUpdate, Store, Updates};
use serde_json::{Value, json};

/// The local digest of `image` for its own repository, from an image inspect's `RepoDigests`.
pub fn local_digest(image_inspect: &Value, image: &str) -> Option<String> {
    let want = spec::canonical_repo(image);
    image_inspect["RepoDigests"].as_array()?.iter().filter_map(Value::as_str).find_map(|rd| {
        let (repo, digest) = rd.split_once('@')?;
        (spec::canonical_repo(&format!("{repo}:x")) == want).then(|| digest.to_string())
    })
}

/// Compares one image with the registry. The local image is system-docker's, else (for a container that still runs on
/// Container Station's own daemon) that daemon's.
pub fn check_image(docker: &Docker, image: &str) -> ImageUpdate {
    let local = match docker.inspect_image_anywhere(image) {
        Ok(Some(i)) => local_digest(&i, image),
        _ => None,
    };
    let (remote, error_msg) = match docker.remote_digest(image) {
        Ok(d) => (Some(d), None),
        Err(e) => (None, Some(msg!("Could not ask the image registry: {error}", error = e.detail()))),
    };
    let available = matches!((&local, &remote), (Some(l), Some(r)) if l != r);
    let error = error_msg.as_ref().map(|m| m.english());
    ImageUpdate { image: image.to_string(), local_digest: local, remote_digest: remote, available, error, error_msg }
}

/// Checks every image the instances use and saves the result.
pub fn check_all(store: &Store, docker: &Docker, mut on_image: impl FnMut(&str, usize, usize)) -> Updates {
    let mut images: Vec<String> = store.records().into_iter().map(|r| r.spec.image).collect();
    images.sort();
    images.dedup();
    let n = images.len();
    let mut out = Updates { checked_at: Some(crate::util::now_rfc3339()), images: Vec::new() };
    for (i, image) in images.iter().enumerate() {
        on_image(image, i, n);
        out.images.push(check_image(docker, image));
    }
    let _ = store.save_updates(&out);
    out
}

/// After a pull: the cached entry of `image` gets the new local digest.
pub fn note_pulled(store: &Store, docker: &Docker, image: &str) {
    let mut u = store.updates();
    let Some(entry) = u.images.iter_mut().find(|i| i.image == image) else { return };
    if let Ok(Some(i)) = docker.inspect_image(image) {
        entry.local_digest = local_digest(&i, image);
        entry.available = matches!((&entry.local_digest, &entry.remote_digest), (Some(l), Some(r)) if l != r);
        let _ = store.save_updates(&u);
    }
}

/// The `update` object of an instance. `local_image_id` is the image the tag points at now, `running_image_id` the one
/// the container was created from.
pub fn instance_update(u: &Updates, image: &str, local_image_id: Option<&str>, running_image_id: Option<&str>) -> Value {
    let entry = u.entry(image);
    let newer_remote = entry.is_some_and(|e| e.available);
    let newer_local = matches!((local_image_id, running_image_id), (Some(l), Some(r)) if !l.is_empty() && !r.is_empty() && l != r);
    let reason = if newer_remote {
        Some("newer_remote")
    } else if newer_local {
        Some("newer_local")
    } else {
        None
    };
    let legacy = spec::legacy_image(image);
    json!({
        "available": reason.is_some(),
        "reason": reason,
        "checked_at": u.checked_at,
        "error": entry.and_then(|e| e.error.clone()),
        "error_msg": entry.and_then(|e| e.error_msg.clone()),
        "legacy_image": legacy.is_some(),
        "suggested_image": legacy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_local_digest_of_the_repository() {
        let i = json!({ "RepoDigests": ["other/repo@sha256:1", "oznu/homebridge@sha256:defc"] });
        assert_eq!(local_digest(&i, "oznu/homebridge:latest").as_deref(), Some("sha256:defc"));
        let lib = json!({ "RepoDigests": ["alpine@sha256:abc"] });
        assert_eq!(local_digest(&lib, "docker.io/library/alpine:3.24").as_deref(), Some("sha256:abc"));
        assert_eq!(local_digest(&json!({}), "alpine:3.24"), None);
        let reg = json!({ "RepoDigests": ["registry.local:5000/hb@sha256:9"] });
        assert_eq!(local_digest(&reg, "registry.local:5000/hb:latest").as_deref(), Some("sha256:9"));
    }

    #[test]
    fn instance_update_reasons() {
        let u = Updates {
            checked_at: Some("2026-11-12T06:00:00Z".into()),
            images: vec![
                ImageUpdate {
                    image: "oznu/homebridge:latest".into(),
                    local_digest: Some("sha256:a".into()),
                    remote_digest: Some("sha256:b".into()),
                    available: true,
                    error: None,
                    error_msg: None,
                },
                ImageUpdate {
                    image: "homebridge/homebridge:latest".into(),
                    local_digest: Some("sha256:c".into()),
                    remote_digest: None,
                    available: false,
                    error: Some("x".into()),
                    error_msg: None,
                },
            ],
        };
        let a = instance_update(&u, "oznu/homebridge:latest", Some("sha256:1"), Some("sha256:1"));
        assert_eq!(a["reason"], "newer_remote");
        assert_eq!(a["legacy_image"], true);
        assert_eq!(a["suggested_image"], spec::DEFAULT_IMAGE);
        let b = instance_update(&u, "homebridge/homebridge:latest", Some("sha256:2"), Some("sha256:1"));
        assert_eq!(b["reason"], "newer_local");
        assert_eq!(b["available"], true);
        assert_eq!(b["error"], "x");
        assert_eq!(b["suggested_image"], Value::Null);
        let c = instance_update(&u, "homebridge/homebridge:latest", Some("sha256:1"), Some("sha256:1"));
        assert_eq!((c["available"].clone(), c["reason"].clone()), (json!(false), Value::Null));
        let never = instance_update(&Updates::default(), "alpine:3.24", None, None);
        assert_eq!(never["checked_at"], Value::Null);
    }
}

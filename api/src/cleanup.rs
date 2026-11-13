//! The images upgrades leave behind on system-docker, where no UI can clean them up.
//!
//! An upgrade keeps the previous image through the backup container; when that backup is replaced by the next one,
//! its image is used by nothing any more. Such an image goes only when it is untagged, no container of system-docker
//! uses it (in any state), and it is Homebridge's: every repository in its `RepoDigests` looks like Homebridge (the rule
//! of `discover::homebridge_repository`), or it is the image the replaced backup of this instance used. Tagged images
//! are never touched (an image that another task has just downloaded is still tagged), nothing is forced and nothing
//! is pruned: other packages keep their own images on system-docker.

use crate::discover;
use serde_json::Value;
use std::collections::HashSet;

/// An image that may go: its id and a short name for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Leftover {
    pub id: String,
    pub label: String,
}

fn untagged(image: &Value) -> bool {
    image["RepoTags"].as_array().is_none_or(|tags| tags.iter().all(|t| t.as_str().is_none_or(|t| t == "<none>:<none>")))
}

/// The repositories in an image's `RepoDigests`.
fn digest_repos(image: &Value) -> Vec<String> {
    image["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|rd| rd.split_once('@').map(|(repo, _)| repo.to_string()))
        .filter(|repo| !repo.is_empty() && repo != "<none>")
        .collect()
}

fn label(id: &str, repo: Option<&String>) -> String {
    let short: String = id.trim_start_matches("sha256:").chars().take(12).collect();
    match repo {
        Some(r) => format!("{r} ({short})"),
        None => short,
    }
}

/// The images of `images` (`GET /images/json`) that may go. `used` holds the image ids of system-docker's containers,
/// `also` image ids that may go even though their repository does not look like Homebridge's (a custom image).
pub fn leftovers(images: &[Value], used: &HashSet<String>, also: &[String]) -> Vec<Leftover> {
    images
        .iter()
        .filter_map(|image| {
            let id = image["Id"].as_str().filter(|id| !id.is_empty())?;
            if used.contains(id) || !untagged(image) {
                return None;
            }
            let repos = digest_repos(image);
            let homebridge = !repos.is_empty() && repos.iter().all(|r| discover::homebridge_repository(r));
            (homebridge || also.iter().any(|a| a == id)).then(|| Leftover { id: id.to_string(), label: label(id, repos.first()) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What system-docker can hold besides Homebridge's images: another package's (tagged, built locally, their base
    /// images).
    fn images() -> Vec<Value> {
        vec![
            json!({ "Id": "sha256:hbnew", "RepoTags": ["homebridge/homebridge:latest"], "RepoDigests": ["homebridge/homebridge@sha256:n"] }),
            json!({ "Id": "sha256:hbprev", "RepoTags": [], "RepoDigests": ["homebridge/homebridge@sha256:p"] }),
            json!({ "Id": "sha256:hbold", "RepoTags": [], "RepoDigests": ["homebridge/homebridge@sha256:o"] }),
            json!({ "Id": "sha256:oznu", "RepoTags": ["<none>:<none>"], "RepoDigests": ["oznu/homebridge@sha256:z"] }),
            json!({ "Id": "sha256:ghcr", "RepoTags": null, "RepoDigests": ["ghcr.io/homebridge/homebridge@sha256:g"] }),
            json!({ "Id": "sha256:hbtag", "RepoTags": ["homebridge/homebridge:2025-01-01"], "RepoDigests": ["homebridge/homebridge@sha256:t"] }),
            json!({ "Id": "sha256:otherprev", "RepoTags": ["otherpkg/php:8.5-previous"], "RepoDigests": [] }),
            json!({ "Id": "sha256:otherbuild", "RepoTags": [], "RepoDigests": [] }),
            json!({ "Id": "sha256:otherbase", "RepoTags": [], "RepoDigests": ["php@sha256:b"] }),
            json!({ "Id": "sha256:mixed", "RepoTags": [], "RepoDigests": ["homebridge/homebridge@sha256:m", "php@sha256:m"] }),
            json!({ "Id": "sha256:custom", "RepoTags": [], "RepoDigests": ["registry.local:5000/hb@sha256:c"] }),
            json!({ "Id": "sha256:customtag", "RepoTags": ["registry.local:5000/hb:2"], "RepoDigests": [] }),
        ]
    }

    fn ids(v: &[Leftover]) -> Vec<&str> {
        v.iter().map(|l| l.id.as_str()).collect()
    }

    #[test]
    fn only_untagged_unused_homebridge_images_go() {
        // The running container uses the new image, the backup the previous one.
        let used: HashSet<String> = ["sha256:hbnew".to_string(), "sha256:hbprev".to_string()].into();
        let gone = leftovers(&images(), &used, &[]);
        assert_eq!(ids(&gone), ["sha256:hbold", "sha256:oznu", "sha256:ghcr"]);
        assert_eq!(gone[0].label, "homebridge/homebridge (hbold)");
    }

    #[test]
    fn other_packages_images_stay() {
        let gone = leftovers(&images(), &HashSet::new(), &[]);
        for keep in ["sha256:otherprev", "sha256:otherbuild", "sha256:otherbase", "sha256:mixed", "sha256:hbtag", "sha256:hbnew"] {
            assert!(!ids(&gone).contains(&keep), "{keep} must stay");
        }
    }

    #[test]
    fn a_replaced_backups_custom_image_goes_when_untagged_and_unused() {
        let also = ["sha256:custom".to_string(), "sha256:customtag".to_string(), "sha256:otherbase".to_string()];
        let used: HashSet<String> = ["sha256:otherbase".to_string()].into();
        let gone = leftovers(&images(), &used, &also);
        assert!(ids(&gone).contains(&"sha256:custom"));
        assert!(!ids(&gone).contains(&"sha256:customtag"), "a tagged image stays");
        assert!(!ids(&gone).contains(&"sha256:otherbase"), "an image in use stays");
        let only_id =
            leftovers(&[json!({ "Id": "sha256:abcdef0123456789", "RepoTags": [] })], &HashSet::new(), &["sha256:abcdef0123456789".into()]);
        assert_eq!(only_id[0].label, "abcdef012345");
    }
}

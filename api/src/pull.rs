//! Turns the JSON lines of an image pull into one percentage and a line of text for the progress dialog.

use crate::msg::Msg;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Waiting,
    Downloading,
    Downloaded,
    Extracting,
    Done,
}

#[derive(Debug, Clone)]
struct Layer {
    id: String,
    phase: Phase,
    dl: (u64, u64),
    ex: (u64, u64),
}

#[derive(Debug, Default, Clone)]
pub struct PullProgress {
    layers: Vec<Layer>,
    /// The `Status: …` line at the end (`Downloaded newer image …`, `Image is up to date …`).
    pub status: Option<String>,
}

impl PullProgress {
    /// Takes one message of the pull stream.
    pub fn feed(&mut self, msg: &Value) {
        let status = msg["status"].as_str().unwrap_or("");
        let Some(id) = msg["id"].as_str().filter(|id| !id.is_empty()) else {
            if let Some(s) = status.strip_prefix("Status: ") {
                self.status = Some(s.to_string());
            }
            return;
        };
        if status.starts_with("Pulling from") || status.starts_with("Pulling repository") {
            return;
        }
        let detail = &msg["progressDetail"];
        let cur = detail["current"].as_u64();
        let total = detail["total"].as_u64();
        let idx = match self.layers.iter().position(|l| l.id == id) {
            Some(i) => i,
            None => {
                self.layers.push(Layer { id: id.to_string(), phase: Phase::Waiting, dl: (0, 0), ex: (0, 0) });
                self.layers.len() - 1
            }
        };
        let l = &mut self.layers[idx];
        match status {
            "Pulling fs layer" | "Waiting" => {}
            "Downloading" => {
                l.phase = l.phase.max(Phase::Downloading);
                if let Some(t) = total.filter(|t| *t > 0) {
                    l.dl.1 = t;
                }
                if let Some(c) = cur {
                    l.dl.0 = c.min(l.dl.1.max(c));
                }
            }
            "Verifying Checksum" | "Download complete" => {
                l.phase = l.phase.max(Phase::Downloaded);
                l.dl.0 = l.dl.1;
            }
            "Extracting" => {
                l.phase = l.phase.max(Phase::Extracting);
                l.dl.0 = l.dl.1;
                if let Some(t) = total.filter(|t| *t > 0) {
                    l.ex.1 = t;
                }
                if let Some(c) = cur {
                    l.ex.0 = c;
                }
            }
            "Pull complete" | "Already exists" => {
                l.phase = Phase::Done;
                l.dl.0 = l.dl.1;
                l.ex.0 = l.ex.1;
            }
            _ => {}
        }
    }

    /// 0–100 over all layers (downloading and extracting count half each), `None` before anything is known.
    pub fn percent(&self) -> Option<u8> {
        if self.layers.is_empty() {
            return None;
        }
        let known: Vec<u64> = self.layers.iter().map(|l| l.dl.1).filter(|t| *t > 0).collect();
        let fallback = if known.is_empty() { 1.0 } else { known.iter().sum::<u64>() as f64 / known.len() as f64 };
        let (mut done, mut all) = (0.0, 0.0);
        for l in &self.layers {
            let weight = if l.dl.1 > 0 { l.dl.1 as f64 } else { fallback };
            let frac = |(c, t): (u64, u64)| if t > 0 { (c as f64 / t as f64).min(1.0) } else { 0.0 };
            let f = match l.phase {
                Phase::Done => 1.0,
                Phase::Waiting => 0.0,
                Phase::Downloading => 0.5 * frac(l.dl),
                Phase::Downloaded => 0.5,
                Phase::Extracting => 0.5 + 0.5 * frac(l.ex),
            };
            done += weight * f;
            all += weight;
        }
        Some(((done / all) * 100.0).floor().clamp(0.0, 100.0) as u8)
    }

    /// What the pull is doing: connecting, downloading (layers and megabytes), extracting, done.
    pub fn detail(&self) -> Msg {
        let n = self.layers.len();
        if n == 0 {
            return msg!("Connecting to the image registry…");
        }
        let downloaded = self.layers.iter().filter(|l| l.phase >= Phase::Downloaded).count();
        if downloaded < n {
            let cur: u64 = self.layers.iter().map(|l| l.dl.0).sum();
            let total: u64 = self.layers.iter().map(|l| l.dl.1).sum();
            if total == 0 {
                return msg!("Downloaded {done} of {count} layers", done = downloaded, count = n);
            }
            return msg!(
                "Downloaded {done} of {count} layers ({received} MB of {total} MB)",
                done = downloaded,
                count = n,
                received = mb(cur),
                total = mb(total)
            );
        }
        let done = self.layers.iter().filter(|l| l.phase == Phase::Done).count();
        if done < n {
            return msg!("Extracted {done} of {count} layers", done = done, count = n);
        }
        msg!("Download complete")
    }
}

/// Bytes in (decimal) megabytes, rounded, as Docker shows them.
fn mb(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed(p: &mut PullProgress, status: &str, id: &str, cur: Option<u64>, total: Option<u64>) {
        let mut v = json!({ "status": status, "id": id, "progressDetail": {} });
        if let (Some(c), Some(t)) = (cur, total) {
            v["progressDetail"] = json!({ "current": c, "total": t });
        }
        p.feed(&v);
    }

    #[test]
    fn aggregates_layers() {
        let mut p = PullProgress::default();
        assert_eq!(p.percent(), None);
        assert_eq!(p.detail().english(), "Connecting to the image registry…");
        p.feed(&json!({ "status": "Pulling from homebridge/homebridge", "id": "latest" }));
        assert_eq!(p.percent(), None, "the tag line is not a layer");
        feed(&mut p, "Already exists", "aaa", None, None);
        feed(&mut p, "Pulling fs layer", "bbb", None, None);
        feed(&mut p, "Pulling fs layer", "ccc", None, None);
        feed(&mut p, "Downloading", "bbb", Some(100_000_000), Some(400_000_000));
        feed(&mut p, "Downloading", "ccc", Some(50_000_000), Some(100_000_000));
        assert_eq!(p.detail().english(), "Downloaded 1 of 3 layers (150 MB of 500 MB)");
        assert_eq!(p.detail().args["received"], 150);
        let pct = p.percent().unwrap();
        assert!(pct > 30 && pct < 50, "{pct}");
        feed(&mut p, "Download complete", "ccc", None, None);
        feed(&mut p, "Download complete", "bbb", None, None);
        assert_eq!(p.detail().english(), "Extracted 1 of 3 layers");
        assert_eq!(p.percent(), Some(66));
        feed(&mut p, "Extracting", "bbb", Some(200_000_000), Some(400_000_000));
        let mid = p.percent().unwrap();
        feed(&mut p, "Pull complete", "bbb", None, None);
        feed(&mut p, "Pull complete", "ccc", None, None);
        assert!(mid > 66 && mid < 100);
        assert_eq!(p.percent(), Some(100));
        assert_eq!(p.detail().english(), "Download complete");
        p.feed(&json!({ "status": "Digest: sha256:49e1" }));
        p.feed(&json!({ "status": "Status: Downloaded newer image for homebridge/homebridge:latest" }));
        assert_eq!(p.status.as_deref(), Some("Downloaded newer image for homebridge/homebridge:latest"));
    }

    #[test]
    fn up_to_date_images_are_complete_at_once() {
        let mut p = PullProgress::default();
        feed(&mut p, "Already exists", "aaa", None, None);
        assert_eq!(p.percent(), Some(100));
    }
}

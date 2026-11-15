//! Performance numbers for the Stats screen: how fast each model writes,
//! and how much memory this computer, its GPU and llama-server use.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

/// Writing speed of one model since Relay started.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ModelSpeed {
    /// The model's file name.
    pub model: String,
    pub replies: u64,
    /// Tokens written in all.
    pub tokens: u64,
    /// Average writing speed (all tokens over all writing time).
    pub avg_tokens_per_sec: f64,
    pub last_tokens_per_sec: f64,
    /// How fast the last long-enough prompt was read.
    pub last_prompt_per_sec: Option<f64>,
    pub last_at: String,
    #[serde(skip)]
    secs: f64,
}

/// Speeds per model, kept in memory.
#[derive(Default)]
pub struct Speeds(Mutex<BTreeMap<String, ModelSpeed>>);

impl Speeds {
    pub fn record(&self, model: &str, tokens: u64, tokens_per_sec: f64, prompt_per_sec: Option<f64>) {
        if model.is_empty() || tokens == 0 || !(tokens_per_sec.is_finite() && tokens_per_sec > 0.0) {
            return;
        }
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let s = map.entry(model.to_string()).or_insert_with(|| ModelSpeed {
            model: model.to_string(),
            ..Default::default()
        });
        s.replies += 1;
        s.tokens += tokens;
        s.secs += tokens as f64 / tokens_per_sec;
        s.avg_tokens_per_sec = s.tokens as f64 / s.secs;
        s.last_tokens_per_sec = tokens_per_sec;
        if prompt_per_sec.is_some() {
            s.last_prompt_per_sec = prompt_per_sec;
        }
        s.last_at = crate::db::now();
    }

    /// Newest first.
    pub fn list(&self) -> Vec<ModelSpeed> {
        let mut v: Vec<ModelSpeed> = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| b.last_at.cmp(&a.last_at));
        v
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Memory {
    pub total_mb: u64,
    pub used_mb: u64,
    /// Memory used by llama-server, when it runs here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GpuMemory {
    pub name: String,
    pub total_mb: u64,
    pub used_mb: u64,
}

const MB: u64 = 1024 * 1024;

/// This computer's memory, and llama-server's share of it if `pid` is given.
/// None where the system doesn't say.
pub fn memory(pid: Option<u32>) -> Option<Memory> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new().ok()?;
    sys.refresh_memory();
    let model_mb = pid.and_then(|pid| {
        let pid = Pid::from_u32(pid);
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        sys.process(pid).map(|p| p.memory() / MB)
    });
    let total_mb = sys.total_memory() / MB;
    (total_mb > 0).then(|| Memory {
        total_mb,
        used_mb: sys.used_memory() / MB,
        model_mb,
    })
}

/// NVIDIA GPU memory in use (empty without nvidia-smi).
pub async fn gpu_memory() -> Vec<GpuMemory> {
    let out = crate::setup::run_capture(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .await;
    out.map(|s| parse_gpu_memory(&s)).unwrap_or_default()
}

fn parse_gpu_memory(s: &str) -> Vec<GpuMemory> {
    s.lines()
        .filter_map(|l| {
            let mut parts = l.rsplitn(3, ',').map(str::trim);
            let total = parts.next()?.parse().ok()?;
            let used = parts.next()?.parse().ok()?;
            let name = parts.next()?.to_string();
            Some(GpuMemory {
                name,
                total_mb: total,
                used_mb: used,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_by_time() {
        let s = Speeds::default();
        // 100 tokens at 50/s (2 s) and 100 at 25/s (4 s): 200 in 6 s.
        s.record("a.gguf", 100, 50.0, Some(800.0));
        s.record("a.gguf", 100, 25.0, None);
        s.record("b.gguf", 0, 10.0, None);
        let l = s.list();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].replies, 2);
        assert!((l[0].avg_tokens_per_sec - 200.0 / 6.0).abs() < 1e-9);
        assert_eq!(l[0].last_tokens_per_sec, 25.0);
        assert_eq!(l[0].last_prompt_per_sec, Some(800.0));
    }

    #[test]
    fn parses_nvidia_smi() {
        let g = parse_gpu_memory("NVIDIA GeForce RTX 4090, 20480, 24564\nbad line\n");
        assert_eq!(
            g,
            vec![GpuMemory {
                name: "NVIDIA GeForce RTX 4090".into(),
                used_mb: 20480,
                total_mb: 24564
            }]
        );
    }

    #[test]
    fn reads_memory() {
        let m = memory(Some(std::process::id())).unwrap();
        assert!(m.total_mb > 0 && m.used_mb <= m.total_mb);
        assert!(m.model_mb.is_some());
    }
}

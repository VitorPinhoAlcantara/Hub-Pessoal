use sysinfo::System;

use crate::platform::gpu::GpuReader;
pub use crate::platform::gpu::GpuSnapshot;

pub const HISTORY_LEN: usize = 60;
pub const WARNING_USAGE: f32 = 80.0;
pub const DANGER_USAGE: f32 = 95.0;
const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warning,
    Danger,
}

pub fn usage_level(percent: f32) -> Level {
    if percent > DANGER_USAGE {
        Level::Danger
    } else if percent > WARNING_USAGE {
        Level::Warning
    } else {
        Level::Normal
    }
}

pub fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / BYTES_PER_GB)
}

pub fn percent_of(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0) as f32
    }
}

pub fn push_history(history: &mut Vec<f32>, value: f32) {
    history.push(value);
    if history.len() > HISTORY_LEN {
        history.remove(0);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub cpu: f32,
    pub cores: usize,
    pub ram_used: u64,
    pub ram_total: u64,
    pub gpu: Option<GpuSnapshot>,
}

pub struct Sampler {
    system: Option<System>,
    gpu: Option<GpuReader>,
}

impl Sampler {
    pub fn new() -> Self {
        let mut system = System::new()
            .map_err(|error| tracing::warn!(%error, "sysinfo indisponível"))
            .ok();
        if let Some(system) = &mut system {
            system.refresh_cpu_usage();
            system.refresh_memory();
        }
        Self {
            system,
            gpu: GpuReader::new(),
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let gpu = self.gpu.as_mut().and_then(GpuReader::read);
        let Some(system) = &mut self.system else {
            return Snapshot {
                cpu: 0.0,
                cores: 0,
                ram_used: 0,
                ram_total: 0,
                gpu,
            };
        };
        system.refresh_cpu_usage();
        system.refresh_memory();
        Snapshot {
            cpu: system.global_cpu_usage(),
            cores: system.cpus().len(),
            ram_used: system.used_memory(),
            ram_total: system.total_memory(),
            gpu,
        }
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_as_gb_with_one_decimal() {
        assert_eq!(format_gb(0), "0.0 GB");
        assert_eq!(format_gb(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(
            format_gb(8 * 1024 * 1024 * 1024 + 512 * 1024 * 1024),
            "8.5 GB"
        );
    }

    #[test]
    fn usage_levels_follow_thresholds() {
        assert_eq!(usage_level(10.0), Level::Normal);
        assert_eq!(usage_level(80.0), Level::Normal);
        assert_eq!(usage_level(80.5), Level::Warning);
        assert_eq!(usage_level(95.0), Level::Warning);
        assert_eq!(usage_level(99.0), Level::Danger);
    }

    #[test]
    fn percent_handles_zero_total() {
        assert_eq!(percent_of(5, 0), 0.0);
        assert_eq!(percent_of(1, 4), 25.0);
    }

    #[test]
    fn history_is_capped_and_drops_oldest() {
        let mut history = Vec::new();
        for value in 0..(HISTORY_LEN + 5) {
            push_history(&mut history, value as f32);
        }
        assert_eq!(history.len(), HISTORY_LEN);
        assert_eq!(history[0], 5.0);
        assert_eq!(*history.last().unwrap(), (HISTORY_LEN + 4) as f32);
    }
}

//! How the machine is holding up, for the meter in the title bar: CPU,
//! memory, and the disk the open track's folder lives on. Agents are
//! heavy (each session is a node or native process), so this is what the
//! human glances at before opening another lane.

use std::path::Path;

use parking_lot::Mutex;
use serde::Serialize;
use sysinfo::{Disks, System};

/// One reading. Sizes in bytes; `cpu` in percent over the interval since
/// the previous reading (the first one reads 0).
#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    /// Mount point of the disk measured, e.g. `C:\`.
    pub disk_mount: String,
    /// Agent sessions open right now (conductors and lanes).
    pub sessions: usize,
    /// Of those, how many have a turn in flight.
    pub working: usize,
}

/// Keeps the `System` between readings: CPU usage is a difference of two.
#[derive(Default)]
pub struct Meter {
    sys: Mutex<Option<System>>,
}

impl Meter {
    /// CPU and memory now, and the disk holding `path` (or the largest
    /// disk when no path is given or none matches).
    pub fn read(&self, path: Option<&Path>) -> Metrics {
        let (cpu, mem_used, mem_total) = {
            let mut guard = self.sys.lock();
            let sys = guard.get_or_insert_with(System::new);
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            (sys.global_cpu_usage(), sys.used_memory(), sys.total_memory())
        };
        let disks = Disks::new_with_refreshed_list();
        let chosen = path
            .and_then(|p| {
                let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
                // The deepest mount point that contains the path.
                disks
                    .list()
                    .iter()
                    .filter(|d| p.starts_with(d.mount_point()) || strip_verbatim(&p).starts_with(d.mount_point()))
                    .max_by_key(|d| d.mount_point().as_os_str().len())
            })
            .or_else(|| disks.list().iter().max_by_key(|d| d.total_space()));
        let (disk_used, disk_total, disk_mount) = chosen
            .map(|d| {
                let total = d.total_space();
                (total.saturating_sub(d.available_space()), total, d.mount_point().display().to_string())
            })
            .unwrap_or_default();
        Metrics {
            cpu,
            mem_used,
            mem_total,
            disk_used,
            disk_total,
            disk_mount,
            sessions: 0,
            working: 0,
        }
    }
}

/// `\?\C:\x` → `C:\x`: canonicalize on Windows adds the verbatim prefix,
/// mount points do not have it.
fn strip_verbatim(p: &Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    std::path::PathBuf::from(s.strip_prefix(r"\?\").unwrap_or(&s).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_something_plausible() {
        let meter = Meter::default();
        let first = meter.read(Some(&std::env::temp_dir()));
        assert!(first.mem_total > 0 && first.mem_used <= first.mem_total);
        assert!(first.disk_total > 0 && first.disk_used <= first.disk_total, "{first:?}");
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let second = meter.read(None);
        assert!((0.0..=100.0).contains(&second.cpu));
    }
}

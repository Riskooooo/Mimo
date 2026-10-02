//! Collects a [`SystemSnapshot`] for the PC check-up: CPU, memory, disks
//! and processes via sysinfo; temperature (ACPI thermal zone — the one
//! sensor Windows exposes without admin rights) and battery via one
//! PowerShell query; GPU via `nvidia-smi` when an NVIDIA card is present.
//! Takes about a second, so callers run it off the UI thread.

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::process::Command;

use mimo_core::system::{BatteryInfo, DiskInfo, GpuInfo, ProcessInfo, SystemSnapshot};
use serde::Deserialize;
use sysinfo::{Disks, ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TOP_PROCESSES: usize = 5;

pub fn collect() -> SystemSnapshot {
    let mut sys = System::new();
    // CPU usage (overall and per process) is a difference between two
    // samples taken a moment apart.
    sys.refresh_cpu_usage();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL.max(std::time::Duration::from_millis(300)));
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let cpu_cores = sys.cpus().len().max(1);
    let (temperature_c, battery) = thermal_and_battery();

    SystemSnapshot {
        cpu_name: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cpu_cores,
        cpu_usage: sys.global_cpu_usage(),
        memory_used: sys.used_memory(),
        memory_total: sys.total_memory(),
        disks: disks(),
        uptime_secs: System::uptime(),
        temperature_c,
        gpu: gpu(),
        battery,
        top_processes: top_processes(&sys, cpu_cores),
        process_count: sys.processes().len(),
    }
}

fn disks() -> Vec<DiskInfo> {
    let mut disks: Vec<DiskInfo> = Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| !d.is_removable() && d.total_space() > 0)
        .map(|d| DiskInfo {
            mount: d.mount_point().to_string_lossy().to_string(),
            used: d.total_space().saturating_sub(d.available_space()),
            total: d.total_space(),
        })
        .collect();
    disks.sort_by(|a, b| a.mount.cmp(&b.mount));
    disks.dedup_by(|a, b| a.mount == b.mount);
    disks
}

/// Heaviest apps by memory, with their processes added up ("chrome.exe"
/// runs as dozens of processes; what matters is the total).
fn top_processes(sys: &System, cpu_cores: usize) -> Vec<ProcessInfo> {
    let mut by_name: HashMap<String, ProcessInfo> = HashMap::new();
    for process in sys.processes().values() {
        let name = process.name().to_string_lossy().to_string();
        let entry = by_name.entry(name.clone()).or_insert_with(|| ProcessInfo { name, ..Default::default() });
        entry.memory += process.memory();
        // sysinfo reports per-process CPU relative to one core.
        entry.cpu += process.cpu_usage() / cpu_cores as f32;
    }
    let mut top: Vec<ProcessInfo> = by_name
        .into_values()
        .filter(|p| !matches!(p.name.as_str(), "System" | "Idle" | "System Idle Process" | "Memory Compression" | "Registry"))
        .collect();
    top.sort_by_key(|p| std::cmp::Reverse(p.memory));
    top.truncate(TOP_PROCESSES);
    top
}

#[derive(Deserialize)]
struct PowerShellReadings {
    temp: Option<f64>,
    battery: Option<u8>,
    /// Win32_Battery.BatteryStatus: 1 = on battery, 2 = on AC, 6-9 = charging.
    status: Option<u16>,
}

fn thermal_and_battery() -> (Option<f32>, Option<BatteryInfo>) {
    const SCRIPT: &str = "\
        $t = Get-CimInstance Win32_PerfFormattedData_Counters_ThermalZoneInformation -ErrorAction SilentlyContinue | \
             Measure-Object -Property HighPrecisionTemperature -Maximum; \
        $b = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Select-Object -First 1; \
        @{ temp = $(if ($t.Maximum) { [math]::Round($t.Maximum / 10 - 273.15, 1) } else { $null }); \
           battery = $b.EstimatedChargeRemaining; status = $b.BatteryStatus } | ConvertTo-Json -Compress";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return (None, None);
    };
    let Ok(readings) = serde_json::from_slice::<PowerShellReadings>(&output.stdout) else {
        return (None, None);
    };
    // A thermal zone reporting below ~10 °C or above 120 °C is a bogus sensor.
    let temperature = readings.temp.filter(|t| (10.0..120.0).contains(t)).map(|t| t as f32);
    let battery = readings.battery.map(|percent| BatteryInfo {
        percent,
        charging: matches!(readings.status, Some(2 | 6..=9)),
    });
    (temperature, battery)
}

fn gpu() -> Option<GpuInfo> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=name,temperature.gpu,utilization.gpu", "--format=csv,noheader,nounits"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let mut fields = line.lines().next()?.split(',').map(str::trim);
    Some(GpuInfo {
        name: fields.next()?.to_string(),
        temperature_c: fields.next().and_then(|t| t.parse().ok()),
        usage: fields.next().and_then(|u| u.parse().ok()),
    })
}

#[cfg(test)]
mod probe {
    #[test]
    #[ignore = "manual probe of the real machine"]
    fn print_snapshot() {
        let s = super::collect();
        println!("{s:#?}");
        for a in mimo_core::system::advise(&s, mimo_core::Lang::Fr) {
            println!("{:?} | {} | {}", a.level, a.title, a.detail);
        }
    }
}

//! PC check-up: a snapshot of the machine's state (collected by the shell)
//! turned into plain-language advice — what's fine, what to watch, what to
//! do. Thresholds live here so they're tested, not scattered in UI code.

use serde::Serialize;

use crate::info::Lang;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SystemSnapshot {
    pub cpu_name: String,
    pub cpu_cores: usize,
    /// Overall CPU load, 0–100.
    pub cpu_usage: f32,
    /// Bytes.
    pub memory_used: u64,
    pub memory_total: u64,
    pub disks: Vec<DiskInfo>,
    pub uptime_secs: u64,
    /// Motherboard/CPU area (ACPI thermal zone), °C.
    pub temperature_c: Option<f32>,
    pub gpu: Option<GpuInfo>,
    pub battery: Option<BatteryInfo>,
    /// Heaviest processes by memory.
    pub top_processes: Vec<ProcessInfo>,
    pub process_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DiskInfo {
    /// "C:\"
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub temperature_c: Option<f32>,
    pub usage: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BatteryInfo {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ProcessInfo {
    pub name: String,
    pub memory: u64,
    pub cpu: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Good,
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Advice {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

fn percent(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        used as f64 * 100.0 / total as f64
    }
}

/// Worst issues first; ends with what's fine.
pub fn advise(s: &SystemSnapshot, lang: Lang) -> Vec<Advice> {
    let fr = lang == Lang::Fr;
    let t = |fr_text: String, en_text: String| if fr { fr_text } else { en_text };
    let mut out = Vec::new();
    let mut push = |level, title: String, detail: String| out.push(Advice { level, title, detail });

    // Memory
    let mem = percent(s.memory_used, s.memory_total);
    let heaviest = s.top_processes.first();
    let hog = heaviest
        .map(|p| t(format!(" {} en utilise le plus ({:.1} Go).", p.name, p.memory as f64 / GB), format!(" {} uses the most ({:.1} GB).", p.name, p.memory as f64 / GB)))
        .unwrap_or_default();
    if mem >= 90.0 {
        push(Level::Critical, t("Mémoire presque pleine".into(), "Memory almost full".into()),
            t(format!("{mem:.0} % utilisés : ferme des onglets ou des applications.{hog}"), format!("{mem:.0}% used: close some tabs or apps.{hog}")));
    } else if mem >= 75.0 {
        push(Level::Warning, t("Mémoire chargée".into(), "Memory is busy".into()),
            t(format!("{mem:.0} % utilisés.{hog}"), format!("{mem:.0}% used.{hog}")));
    } else {
        push(Level::Good, t("Mémoire OK".into(), "Memory OK".into()),
            t(format!("{mem:.0} % utilisés, il reste de la marge."), format!("{mem:.0}% used, plenty of headroom.")));
    }

    // CPU
    if s.cpu_usage >= 85.0 {
        let busiest = s.top_processes.iter().max_by(|a, b| a.cpu.total_cmp(&b.cpu));
        let who = busiest
            .filter(|p| p.cpu > 10.0)
            .map(|p| t(format!(" Le plus gourmand : {}.", p.name), format!(" Busiest: {}.", p.name)))
            .unwrap_or_default();
        push(Level::Warning, t("Processeur très sollicité".into(), "CPU under heavy load".into()),
            t(format!("{:.0} % d'utilisation.{who}", s.cpu_usage), format!("{:.0}% load.{who}", s.cpu_usage)));
    } else {
        push(Level::Good, t("Processeur OK".into(), "CPU OK".into()),
            t(format!("{:.0} % d'utilisation.", s.cpu_usage), format!("{:.0}% load.", s.cpu_usage)));
    }

    // Disks
    for disk in &s.disks {
        let free_pct = 100.0 - percent(disk.used, disk.total);
        let free_gb = disk.total.saturating_sub(disk.used) as f64 / GB;
        if free_pct < 10.0 {
            push(Level::Critical, t(format!("Disque {} presque plein", disk.mount), format!("Disk {} almost full", disk.mount)),
                t(format!("Plus que {free_gb:.0} Go libres : vide la corbeille, les téléchargements, ou lance le Nettoyage de disque."),
                  format!("Only {free_gb:.0} GB free: empty the recycle bin and downloads, or run Disk Cleanup.")));
        } else if free_pct < 20.0 {
            push(Level::Warning, t(format!("Disque {} bien rempli", disk.mount), format!("Disk {} filling up", disk.mount)),
                t(format!("{free_gb:.0} Go libres ({free_pct:.0} %)."), format!("{free_gb:.0} GB free ({free_pct:.0}%).")));
        }
    }

    if let Some(fullest) = s.disks.iter().max_by(|a, b| percent(a.used, a.total).total_cmp(&percent(b.used, b.total))) {
        let free_pct = 100.0 - percent(fullest.used, fullest.total);
        if free_pct >= 20.0 {
            let free_gb = fullest.total.saturating_sub(fullest.used) as f64 / GB;
            push(Level::Good, t("Stockage OK".into(), "Storage OK".into()),
                t(format!("{free_gb:.0} Go libres sur {}.", fullest.mount), format!("{free_gb:.0} GB free on {}.", fullest.mount)));
        }
    }

    // Temperatures
    match s.temperature_c {
        Some(c) if c >= 90.0 => push(Level::Critical, t("PC brûlant".into(), "PC running very hot".into()),
            t(format!("{c:.0} °C : vérifie que les aérations ne sont pas bouchées et pose-le sur une surface dure."),
              format!("{c:.0} °C: make sure the vents aren't blocked and keep it on a hard surface."))),
        Some(c) if c >= 80.0 => push(Level::Warning, t("PC chaud".into(), "PC running hot".into()),
            t(format!("{c:.0} °C : un nettoyage des ventilateurs peut aider."), format!("{c:.0} °C: cleaning the fans may help."))),
        Some(c) => push(Level::Good, t("Température OK".into(), "Temperature OK".into()),
            t(format!("{c:.0} °C."), format!("{c:.0} °C."))),
        None => {}
    }
    if let Some(gpu) = &s.gpu {
        if let Some(c) = gpu.temperature_c.filter(|c| *c >= 85.0) {
            push(Level::Warning, t("Carte graphique chaude".into(), "Graphics card running hot".into()),
                t(format!("{} à {c:.0} °C.", gpu.name), format!("{} at {c:.0} °C.", gpu.name)));
        }
    }

    // Battery
    if let Some(battery) = &s.battery {
        if battery.percent < 20 && !battery.charging {
            push(Level::Warning, t("Batterie faible".into(), "Battery low".into()),
                t(format!("{} % : branche le chargeur.", battery.percent), format!("{}%: plug in the charger.", battery.percent)));
        }
    }

    // Uptime
    let days = s.uptime_secs / 86_400;
    if days >= 7 {
        push(Level::Info, t("Pense à redémarrer".into(), "Consider restarting".into()),
            t(format!("Allumé depuis {days} jours : un redémarrage libère la mémoire et applique les mises à jour."),
              format!("Up for {days} days: a restart frees memory and applies updates.")));
    }

    out.sort_by_key(|a| std::cmp::Reverse(a.level));
    out
}

/// One line for the pill.
pub fn summary(advice: &[Advice], lang: Lang) -> String {
    let issues = advice.iter().filter(|a| a.level >= Level::Warning).count();
    match (lang, issues) {
        (Lang::Fr, 0) => "Ton PC est en pleine forme.".to_string(),
        (Lang::Fr, 1) => "1 point à surveiller — voici le diagnostic.".to_string(),
        (Lang::Fr, n) => format!("{n} points à surveiller — voici le diagnostic."),
        (Lang::En, 0) => "Your PC is in great shape.".to_string(),
        (Lang::En, 1) => "1 thing to look at — here's the check-up.".to_string(),
        (Lang::En, n) => format!("{n} things to look at — here's the check-up."),
    }
}

const DIAGNOSE_WORDS: &[&str] = &["diagnostic", "diagnostics", "diagnostique", "checkup", "check-up", "bilan"];
const COMPUTER_WORDS: &[&str] = &["pc", "ordi", "ordinateur", "computer", "machine", "systeme", "system"];

/// "fais un diagnostic", "comment va mon pc", "check my computer".
pub fn is_diagnostic_request(tokens: &[String]) -> bool {
    let has = |w: &str| tokens.iter().any(|t| t == w);
    let about_computer = COMPUTER_WORDS.iter().any(|w| has(w));
    DIAGNOSE_WORDS.iter().any(|w| has(w))
        || (has("check") && (has("up") || about_computer))
        || (about_computer && ((has("comment") && has("va")) || has("etat") || (has("how") && (has("is") || has("s")))))
}

pub fn voice_phrases(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "run a diagnostic", "diagnostic", "check my pc", "check my computer", "how is my computer",
            "how's my pc", "system check", "check up",
        ],
        _ => &[
            "fais un diagnostic", "lance un diagnostic", "diagnostic", "fais un diagnostic du pc",
            "diagnostique mon pc", "comment va mon pc", "comment va mon ordinateur", "comment va mon ordi",
            "état du pc", "fais un bilan du pc",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::tokenize;

    fn healthy() -> SystemSnapshot {
        SystemSnapshot {
            cpu_usage: 12.0,
            memory_used: 6 * 1024 * 1024 * 1024,
            memory_total: 16 * 1024 * 1024 * 1024,
            disks: vec![DiskInfo { mount: "C:\\".into(), used: 200, total: 1000 }],
            uptime_secs: 3600,
            temperature_c: Some(45.0),
            ..Default::default()
        }
    }

    #[test]
    fn healthy_pc_gets_only_good_news() {
        let advice = advise(&healthy(), Lang::Fr);
        assert!(advice.iter().all(|a| a.level == Level::Good));
        assert_eq!(summary(&advice, Lang::Fr), "Ton PC est en pleine forme.");
    }

    #[test]
    fn problems_come_first_and_are_counted() {
        let snapshot = SystemSnapshot {
            memory_used: 15 * 1024 * 1024 * 1024,
            disks: vec![DiskInfo { mount: "C:\\".into(), used: 950, total: 1000 }],
            uptime_secs: 10 * 86_400,
            top_processes: vec![ProcessInfo { name: "chrome.exe".into(), memory: 3 * 1024 * 1024 * 1024, cpu: 5.0 }],
            ..healthy()
        };
        let advice = advise(&snapshot, Lang::En);
        assert_eq!(advice[0].level, Level::Critical);
        assert!(advice.iter().any(|a| a.detail.contains("chrome.exe")));
        assert!(advice.iter().any(|a| a.title == "Consider restarting"));
        assert_eq!(summary(&advice, Lang::En), "2 things to look at — here's the check-up.");
    }

    #[test]
    fn recognizes_diagnostic_requests() {
        for q in ["fais un diagnostic", "comment va mon pc ?", "état du PC", "run a diagnostic", "check my computer", "how's my pc"] {
            assert!(is_diagnostic_request(&tokenize(q)), "{q}");
        }
        for q in ["ouvre youtube", "comment ça va", "check the weather"] {
            assert!(!is_diagnostic_request(&tokenize(q)), "{q}");
        }
    }
}

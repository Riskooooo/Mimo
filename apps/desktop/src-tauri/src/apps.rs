//! Discovers the apps installed on this PC — every Start menu entry, classic
//! programs and Store apps alike — and keeps the engine's catalog of them
//! current, so "lance spotify" can open the real app.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use mimo_core::{AppCatalog, Engine, InstalledApp};
use serde::Deserialize;
use tauri::{AppHandle, Manager};

use crate::voice::VoiceWake;

/// Apps get installed and removed while Mimo runs; re-scan this often.
const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);

/// Keeps PowerShell from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StartApp {
    name: String,
    #[serde(rename = "AppID")]
    app_id: String,
}

/// Scans now and then every [`REFRESH_EVERY`], on a background thread (a
/// scan takes about a second).
pub fn keep_catalog_current(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("mimo-apps".into())
        .spawn(move || loop {
            match discover() {
                Ok(catalog) => {
                    let count = catalog.apps().len();
                    let changed = app
                        .state::<Mutex<Engine>>()
                        .lock()
                        .expect("engine mutex poisoned")
                        .set_installed_apps(catalog);
                    if changed {
                        eprintln!("[apps] {count} installed apps");
                        // Voice recognition's grammar includes app names.
                        app.state::<VoiceWake>().refresh(&app);
                    }
                }
                Err(err) => eprintln!("[apps] scan failed: {err}"),
            }
            std::thread::sleep(REFRESH_EVERY);
        });
}

/// Lists Start menu apps through PowerShell's `Get-StartApps`, which covers
/// both desktop shortcuts and packaged (Store) apps with their launch ids.
fn discover() -> Result<AppCatalog, String> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
             @(Get-StartApps | Select-Object Name, AppID) | ConvertTo-Json -Compress",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("couldn't run PowerShell: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let json = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<StartApp> = serde_json::from_str(json.trim_start_matches('\u{feff}').trim())
        .map_err(|err| format!("unexpected Get-StartApps output: {err}"))?;
    Ok(AppCatalog::new(
        entries
            .iter()
            .filter_map(|entry| InstalledApp::new(&entry.name, &entry.app_id))
            .collect(),
    ))
}

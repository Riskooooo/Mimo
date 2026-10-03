//! The "panel" window: a real, larger window (iOS-style cards) for content
//! that doesn't fit the pill — the PC check-up, notifications, reminders.
//! The shell keeps the current content so the window can fetch it on load
//! (an event alone could arrive before its listener is ready).

use std::sync::Mutex;

use mimo_core::activity::{AppUsage, Period};
use mimo_core::notifications::NotificationItem;
use mimo_core::reminders::Reminder;
use mimo_core::system::{Advice, SystemSnapshot};
use mimo_core::tasks::Task;
use mimo_core::Lang;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PanelContent {
    Diagnostic {
        lang: &'static str,
        snapshot: SystemSnapshot,
        advice: Vec<Advice>,
        summary: String,
    },
    Notifications {
        lang: &'static str,
        items: Vec<NotificationItem>,
        error: Option<String>,
    },
    Reminders {
        lang: &'static str,
        items: Vec<Reminder>,
    },
    Tasks {
        lang: &'static str,
        items: Vec<Task>,
        summary: String,
    },
    Activity {
        lang: &'static str,
        period: Period,
        total_secs: i64,
        /// Most used first.
        apps: Vec<AppUsage>,
        /// Active minutes per hour of today, or per day of the last 7
        /// days, starting at `chart_start` (unix, local midnight).
        chart: Vec<u32>,
        chart_start: i64,
        summary: String,
    },
}

pub fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "fr",
        Lang::En => "en",
    }
}

#[derive(Default)]
pub struct Panel {
    current: Mutex<Option<PanelContent>>,
}

impl Panel {
    pub fn current(&self) -> Option<PanelContent> {
        self.current.lock().expect("panel mutex poisoned").clone()
    }

    pub fn current_mut(&self) -> std::sync::MutexGuard<'_, Option<PanelContent>> {
        self.current.lock().expect("panel mutex poisoned")
    }
}

/// Updates the panel in place if it's currently showing the same kind of
/// content (e.g. the task list changed by voice) — without bringing it up.
pub fn refresh_if_showing(app: &AppHandle, content: PanelContent) {
    let panel = app.state::<Panel>();
    let mut current = panel.current.lock().expect("panel mutex poisoned");
    let same_kind = current
        .as_ref()
        .is_some_and(|c| std::mem::discriminant(c) == std::mem::discriminant(&content));
    let visible = app
        .get_webview_window("panel")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if same_kind && visible {
        *current = Some(content.clone());
        if let Some(window) = app.get_webview_window("panel") {
            let _ = window.emit("mimo://panel", content);
        }
    }
}

/// Shows the panel window centered on screen with `content`.
pub fn show(app: &AppHandle, content: PanelContent) {
    *app.state::<Panel>().current.lock().expect("panel mutex poisoned") = Some(content.clone());
    if let Some(window) = app.get_webview_window("panel") {
        if !window.is_visible().unwrap_or(false) {
            let _ = window.center();
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = window.emit("mimo://panel", content);
    }
}

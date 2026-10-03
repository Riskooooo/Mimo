//! Screenshots (PNG, Pictures\Mimo Capture) and screen recordings (MP4,
//! Videos\Mimo Records) of the primary monitor, through Windows Graphics
//! Capture (`windows-capture`). The pill and its tray menu are excluded from
//! captures (`SetWindowDisplayAffinity`) so they never show up in them.
//! Recordings are video only and last until "arrête l'enregistrement" (or
//! Mimo quits, which finishes the file).

use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use mimo_core::capture::{RECORDING_FOLDER, SCREENSHOT_FOLDER};
use tauri::{AppHandle, Manager};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, ImageFormat, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Recordings are capped at 30 fps: plenty for a screen, half the work.
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// Takes a screenshot of the primary monitor; returns where it was saved.
pub fn screenshot(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().picture_dir().map_err(|e| e.to_string())?.join(SCREENSHOT_FOLDER);
    let path = dir.join(file_name("Capture", "png"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    hide_pill_from_captures(app, true);
    let (sender, receiver) = mpsc::channel();
    let result = Snapshot::start_free_threaded(settings(primary()?, (path.clone(), sender)))
        .map_err(|e| e.to_string())
        .and_then(|control| {
            let saved = receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap_or_else(|_| Err("no frame from the screen".to_string()));
            let _ = control.stop();
            saved
        });
    if !app.state::<Recorder>().is_recording() {
        hide_pill_from_captures(app, false);
    }
    result.map(|()| path)
}

/// Saves the first frame, then stops.
struct Snapshot {
    path: PathBuf,
    done: Option<mpsc::Sender<Result<(), String>>>,
}

impl GraphicsCaptureApiHandler for Snapshot {
    type Flags = (PathBuf, mpsc::Sender<Result<(), String>>);
    type Error = Error;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (path, done) = ctx.flags;
        Ok(Self { path, done: Some(done) })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, control: InternalCaptureControl) -> Result<(), Self::Error> {
        if let Some(done) = self.done.take() {
            let saved = frame.save_as_image(&self.path, ImageFormat::Png).map_err(|e| e.to_string());
            let _ = done.send(saved);
            control.stop();
        }
        Ok(())
    }
}

/// The recording in progress, if any.
#[derive(Default)]
pub struct Recorder {
    active: Mutex<Option<Active>>,
}

struct Active {
    control: CaptureControl<Recording, Error>,
    started: Instant,
    path: PathBuf,
}

/// Feeds every frame to the MP4 encoder.
struct Recording {
    encoder: Option<VideoEncoder>,
}

impl GraphicsCaptureApiHandler for Recording {
    type Flags = (u32, u32, PathBuf);
    type Error = Error;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (width, height, path) = ctx.flags;
        // ~4 bits per pixel per second: about 8 Mbps at 1080p.
        let bitrate = (width * height * 4).clamp(4_000_000, 25_000_000);
        let encoder = VideoEncoder::new(
            // H.264 plays everywhere; HEVC needs a paid codec on many PCs.
            VideoSettingsBuilder::new(width, height)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(30)
                .bitrate(bitrate),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &path,
        )?;
        Ok(Self { encoder: Some(encoder) })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame(frame)?;
        }
        Ok(())
    }
}

impl Recorder {
    pub fn is_recording(&self) -> bool {
        self.lock().is_some()
    }

    /// Starts recording the primary monitor. `Ok(false)` if already recording.
    pub fn start(&self, app: &AppHandle) -> Result<bool, String> {
        let mut active = self.lock();
        if active.is_some() {
            return Ok(false);
        }
        let dir = app.path().video_dir().map_err(|e| e.to_string())?.join(RECORDING_FOLDER);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(file_name("Record", "mp4"));

        let monitor = primary()?;
        let width = monitor.width().map_err(|e| e.to_string())?;
        let height = monitor.height().map_err(|e| e.to_string())?;
        hide_pill_from_captures(app, true);
        let control = Recording::start_free_threaded(settings(monitor, (width, height, path.clone())))
            .map_err(|e| {
                hide_pill_from_captures(app, false);
                e.to_string()
            })?;
        eprintln!("[capture] recording {width}x{height} to {}", path.display());
        *active = Some(Active { control, started: Instant::now(), path });
        Ok(true)
    }

    /// Stops and finishes the file; returns it and how long it lasted.
    /// `Ok(None)` if nothing was being recorded.
    pub fn stop(&self, app: &AppHandle) -> Result<Option<(PathBuf, Duration)>, String> {
        let Some(active) = self.lock().take() else { return Ok(None) };
        hide_pill_from_captures(app, false);
        let length = active.started.elapsed();
        let recording = active.control.callback();
        let stopped = active.control.stop().map_err(|e| e.to_string());
        // The encoder is finished after the capture thread is gone, so no
        // frame can arrive in between.
        let encoder = recording.lock().encoder.take();
        let finished = match encoder {
            Some(encoder) => encoder.finish().map_err(|e| e.to_string()),
            None => Err("the recording had already failed".to_string()),
        };
        eprintln!("[capture] recording stopped after {:.0?}: {stopped:?} {finished:?}", length);
        stopped.and(finished).map(|()| Some((active.path, length)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Active>> {
        self.active.lock().expect("recorder mutex poisoned")
    }
}

fn primary() -> Result<Monitor, String> {
    Monitor::primary().map_err(|e| e.to_string())
}

fn settings<F>(monitor: Monitor, flags: F) -> Settings<F, Monitor> {
    Settings::new(
        monitor,
        CursorCaptureSettings::WithCursor,
        // No yellow "being captured" frame around the screen (Windows 11).
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Custom(FRAME_INTERVAL),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    )
}

/// "Capture 2026-10-03 16.05.12.png" (colons aren't allowed in file names).
fn file_name(prefix: &str, extension: &str) -> String {
    format!("{prefix} {}.{extension}", Local::now().format("%Y-%m-%d %H.%M.%S"))
}

/// Keeps the pill and its tray menu out of screenshots and recordings
/// (they stay visible on screen).
fn hide_pill_from_captures(app: &AppHandle, hidden: bool) {
    const WDA_NONE: u32 = 0;
    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;
    for label in ["main", "tray-menu"] {
        if let Some(hwnd) = app.get_webview_window(label).and_then(|w| w.hwnd().ok()) {
            unsafe { SetWindowDisplayAffinity(hwnd.0, if hidden { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE }) };
        }
    }
}

#[link(name = "user32")]
extern "system" {
    fn SetWindowDisplayAffinity(hwnd: *mut std::ffi::c_void, affinity: u32) -> i32;
}

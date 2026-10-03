//! What's in front right now: the foreground window's app, title, whether
//! it covers its whole monitor, and how long since the last keyboard/mouse
//! input. Plain Win32 calls (microseconds each), declared here rather than
//! pulling in a bindings crate.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;

use mimo_core::activity::{clean_title, identify, Observation};

type Handle = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Default)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[repr(C)]
struct LastInputInfo {
    size: u32,
    time: u32,
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const MONITOR_DEFAULTTONEAREST: u32 = 2;

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> Handle;
    fn GetWindowThreadProcessId(window: Handle, process_id: *mut u32) -> u32;
    fn GetWindowTextW(window: Handle, text: *mut u16, max: i32) -> i32;
    fn GetWindowRect(window: Handle, rect: *mut Rect) -> i32;
    fn MonitorFromWindow(window: Handle, flags: u32) -> Handle;
    fn GetMonitorInfoW(monitor: Handle, info: *mut MonitorInfo) -> i32;
    fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount() -> u32;
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(path: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(path: *const u16, handle: u32, len: u32, data: *mut c_void) -> i32;
    fn VerQueryValueW(block: *const c_void, sub_block: *const u16, buffer: *mut *mut c_void, len: *mut u32) -> i32;
}

/// Samples the foreground window. Keeps executables' descriptions (read
/// from their files) so each is only read once.
#[derive(Default)]
pub struct Sampler {
    descriptions: HashMap<String, Option<String>>,
}

impl Sampler {
    /// `None` when nothing the user works in is in front (the desktop, the
    /// lock screen, Mimo itself).
    pub fn sample(&mut self) -> Option<Observation> {
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return None;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id == 0 || process_id == std::process::id() {
            return None;
        }
        let path = process_path(process_id)?;
        let exe = Path::new(&path).file_stem()?.to_string_lossy().to_lowercase();
        let title = window_title(window);
        let description = self.descriptions.entry(path.clone()).or_insert_with(|| file_description(&path));
        let (app, label) = identify(&exe, description.as_deref(), &title)?;
        Some(Observation {
            title: clean_title(&title, &label),
            app,
            label,
            idle_secs: idle_secs(),
            fullscreen: is_fullscreen(window),
        })
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn window_title(window: Handle) -> String {
    let mut buffer = [0u16; 512];
    let len = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn process_path(process_id: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len) };
    unsafe { CloseHandle(process) };
    (ok != 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}

/// The executable's "File description" ("Google Chrome"), in the first
/// language its version info lists.
fn file_description(path: &str) -> Option<String> {
    let path = wide(path);
    let mut ignored = 0;
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }

    let query = |sub_block: &str| -> Option<(*mut c_void, u32)> {
        let sub_block = wide(sub_block);
        let mut value: *mut c_void = std::ptr::null_mut();
        let mut len = 0;
        let ok = unsafe { VerQueryValueW(data.as_ptr().cast(), sub_block.as_ptr(), &mut value, &mut len) };
        (ok != 0 && !value.is_null() && len > 0).then_some((value, len))
    };

    // Pairs of (language, code page).
    let mut code = "040904b0".to_string();
    if let Some((value, len)) = query("\\VarFileInfo\\Translation") {
        if len >= 4 {
            let pair = unsafe { std::slice::from_raw_parts(value as *const u16, 2) };
            code = format!("{:04x}{:04x}", pair[0], pair[1]);
        }
    }
    let (value, len) = query(&format!("\\StringFileInfo\\{code}\\FileDescription"))?;
    // `len` counts UTF-16 units, the terminating zero included.
    let text = unsafe { std::slice::from_raw_parts(value as *const u16, len as usize) };
    let text = String::from_utf16_lossy(text);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn idle_secs() -> u64 {
    let mut info = LastInputInfo { size: std::mem::size_of::<LastInputInfo>() as u32, time: 0 };
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return 0;
    }
    // Both are milliseconds since boot, wrapping every ~49 days.
    (unsafe { GetTickCount() }.wrapping_sub(info.time) / 1000) as u64
}

/// The window covers its whole monitor (a video, a game).
fn is_fullscreen(window: Handle) -> bool {
    let mut rect = Rect::default();
    if unsafe { GetWindowRect(window, &mut rect) } == 0 {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let mut info = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, ..Default::default() };
    if monitor.is_null() || unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return false;
    }
    let screen = info.monitor;
    rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom
}

#[cfg(test)]
mod probe {
    #[test]
    #[ignore = "manual probe of the real machine"]
    fn print_foreground() {
        let mut sampler = super::Sampler::default();
        for _ in 0..5 {
            println!("{:?}", sampler.sample());
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
}

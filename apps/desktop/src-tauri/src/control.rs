//! Carries out [`ControlCommand`]s: the master volume through Core Audio,
//! media keys as simulated key presses (whatever app plays media picks them
//! up, like a keyboard's own keys), the built-in screen's brightness
//! through WMI (laptops; external monitors don't expose it there), and
//! locking / sleeping through user32 / powrprof.

use std::os::windows::process::CommandExt;
use std::time::Duration;

use mimo_core::control::{
    format_brightness, format_done, format_failed, format_muted, format_unmuted, format_volume, ControlCommand, Level,
};
use mimo_core::Lang;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Lock/sleep wait this long so the pill's reply is seen first.
const BEFORE_LEAVING: Duration = Duration::from_millis(1200);

/// Does it; returns the reply and whether it worked. Blocking (WMI takes
/// about a second): call it off the UI thread.
pub fn run(command: ControlCommand, lang: Lang) -> (String, bool) {
    let outcome = match command {
        ControlCommand::Volume(level) => set_volume(level).map(|v| format_volume(v, lang)),
        ControlCommand::Mute => set_mute(true).map(|_| format_muted(lang)),
        ControlCommand::Unmute => set_mute(false).map(|v| format_unmuted(v, lang)),
        ControlCommand::Brightness(level) => set_brightness(level).map(|v| format_brightness(v, lang)),
        ControlCommand::PlayPause => press(VK_MEDIA_PLAY_PAUSE).map(|_| format_done(command, lang)),
        ControlCommand::NextTrack => press(VK_MEDIA_NEXT_TRACK).map(|_| format_done(command, lang)),
        ControlCommand::PreviousTrack => press(VK_MEDIA_PREV_TRACK).map(|_| format_done(command, lang)),
        ControlCommand::Lock | ControlCommand::Sleep => {
            std::thread::spawn(move || {
                std::thread::sleep(BEFORE_LEAVING);
                let result = if command == ControlCommand::Lock { lock() } else { sleep() };
                if let Err(err) = result {
                    eprintln!("[control] {err}");
                }
            });
            Ok(format_done(command, lang))
        }
    };
    match outcome {
        Ok(reply) => (reply, true),
        Err(err) => {
            eprintln!("[control] {command:?}: {err}");
            (format_failed(command, lang), false)
        }
    }
}

/// The default output device's volume control. COM is initialized on the
/// calling thread first (harmless if it already is).
fn endpoint_volume() -> Result<IAudioEndpointVolume, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("device enumerator: {e}"))?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia).map_err(|e| format!("no output device: {e}"))?;
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).map_err(|e| format!("volume control: {e}"))
    }
}

fn percent(scalar: f32) -> u8 {
    (scalar * 100.0).round().clamp(0.0, 100.0) as u8
}

/// Sets the master volume (and unmutes: asking for sound means wanting it).
fn set_volume(level: Level) -> Result<u8, String> {
    let volume = endpoint_volume()?;
    unsafe {
        let current = percent(volume.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())?);
        let target = level.apply(current);
        volume.SetMasterVolumeLevelScalar(f32::from(target) / 100.0, std::ptr::null()).map_err(|e| e.to_string())?;
        volume.SetMute(false, std::ptr::null()).map_err(|e| e.to_string())?;
        Ok(target)
    }
}

/// Mutes or unmutes; returns the volume level.
fn set_mute(mute: bool) -> Result<u8, String> {
    let volume = endpoint_volume()?;
    unsafe {
        volume.SetMute(mute, std::ptr::null()).map_err(|e| e.to_string())?;
        Ok(percent(volume.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())?))
    }
}

/// A media key press, as a keyboard's own media keys send it.
fn press(key: VIRTUAL_KEY) -> Result<(), String> {
    let input = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: key, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [input(KEYEVENTF_EXTENDEDKEY), input(KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    (sent as usize == inputs.len()).then_some(()).ok_or_else(|| "SendInput was blocked".to_string())
}

/// Reads the built-in screen's brightness and sets the new level in one
/// PowerShell call. The script only ever gets numbers computed here.
fn set_brightness(level: Level) -> Result<u8, String> {
    let (delta, target) = match level {
        Level::Up => (i32::from(mimo_core::control::STEP), -1),
        Level::Down => (-i32::from(mimo_core::control::STEP), -1),
        Level::Set(value) => (0, i32::from(value.min(100))),
    };
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         $now = (Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Select-Object -First 1).CurrentBrightness; \
         $to = if ({target} -ge 0) {{ {target} }} else {{ [math]::Max(0, [math]::Min(100, $now + ({delta}))) }}; \
         $m = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Select-Object -First 1; \
         Invoke-CimMethod -InputObject $m -MethodName WmiSetBrightness -Arguments @{{ Timeout = 1; Brightness = [byte]$to }} | Out-Null; \
         $to"
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("can't run PowerShell: {err}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    match text.trim().parse::<u8>() {
        Ok(value) if output.status.success() => Ok(value),
        _ => Err(format!("brightness not adjustable: {}", String::from_utf8_lossy(&output.stderr).trim())),
    }
}

fn lock() -> Result<(), String> {
    unsafe { LockWorkStation() }.map_err(|err| format!("can't lock: {err}"))
}

/// Sleep (not hibernate), wake events allowed.
fn sleep() -> Result<(), String> {
    let done = unsafe { SetSuspendState(false, false, false) };
    done.then_some(()).ok_or_else(|| format!("can't sleep: {}", std::io::Error::last_os_error()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real PC (puts things back): `cargo test -p desktop
    /// volume_and_brightness_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn volume_and_brightness_round_trip() {
        let volume = endpoint_volume().expect("volume control");
        let before = percent(unsafe { volume.GetMasterVolumeLevelScalar() }.expect("volume"));
        let muted = unsafe { volume.GetMute() }.expect("mute").as_bool();
        println!("volume {before} % (muted: {muted})");
        assert_eq!(set_volume(Level::Set(before)), Ok(before));
        unsafe { volume.SetMute(muted, std::ptr::null()) }.expect("restore mute");

        match set_brightness(Level::Down) {
            Ok(lower) => {
                println!("brightness lowered to {lower} %");
                println!("brightness back to {:?}", set_brightness(Level::Up));
            }
            Err(err) => println!("brightness: {err}"),
        }
    }
}

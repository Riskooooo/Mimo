//! Text-to-speech with the voices Windows already has (System.Speech, via
//! a hidden PowerShell). The text goes through stdin, never the command
//! line, so nothing in it can be interpreted as script.

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use mimo_core::Lang;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The utterance in progress, so a new one (or `stop`) cuts it off.
#[derive(Default)]
pub struct Speech {
    current: Mutex<Option<Child>>,
}

impl Speech {
    pub fn say(&self, text: &str, lang: Lang) {
        self.stop();
        let culture = match lang {
            Lang::Fr => "fr-FR",
            Lang::En => "en-US",
        };
        let script = format!(
            "[Console]::InputEncoding = [Text.Encoding]::UTF8; \
             Add-Type -AssemblyName System.Speech; \
             $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             try {{ $s.SelectVoiceByHints([System.Speech.Synthesis.VoiceGender]::NotSet, \
                    [System.Speech.Synthesis.VoiceAge]::NotSet, 0, [Globalization.CultureInfo]'{culture}') }} catch {{}}; \
             $s.Speak([Console]::In.ReadToEnd())"
        );
        let child = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        match child {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                    // Dropping stdin closes it, which ends ReadToEnd().
                }
                *self.current.lock().expect("speech mutex poisoned") = Some(child);
            }
            Err(err) => eprintln!("[speech] {err}"),
        }
    }

    pub fn stop(&self) {
        if let Some(mut child) = self.current.lock().expect("speech mutex poisoned").take() {
            let _ = child.kill();
        }
    }
}

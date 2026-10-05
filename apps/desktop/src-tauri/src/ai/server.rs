//! Runs llama.cpp's `llama-server` as a hidden child process listening on
//! localhost only, and talks to it over its OpenAI-style HTTP API. A Job
//! Object ties it to Mimo: if Mimo exits or crashes, Windows ends it too
//! (it holds gigabytes of memory). It runs below normal priority so it
//! never slows down what the user is doing.

use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mimo_core::ai::Message;
use serde_json::{json, Value};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use super::download::{model_path, ENGINE_DIR, SERVER_EXE};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
/// Loading 2.5 GB from a slow disk onto the CPU can take a while.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
const CONTEXT: &str = "4096";

pub struct Server {
    child: Child,
    job: Option<HANDLE>,
    port: u16,
}

// The job handle is only closed in `drop`.
unsafe impl Send for Server {}

impl Server {
    /// Starts the server and waits until the model is loaded.
    pub fn start(dir: &Path) -> Result<Self, String> {
        let port = free_port()?;
        let log = File::create(dir.join("server.log")).map_err(|err| format!("can't create the log: {err}"))?;
        let child = Command::new(dir.join(ENGINE_DIR).join(SERVER_EXE))
            .arg("-m")
            .arg(model_path(dir))
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-c", CONTEXT, "-np", "1"])
            .current_dir(dir.join(ENGINE_DIR))
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|err| err.to_string())?)
            .stderr(log)
            .creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
            .spawn()
            .map_err(|err| format!("can't start the AI engine: {err}"))?;
        let job = tie_to_mimo(&child);
        let mut server = Server { child, job, port };

        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = server.child.try_wait() {
                return Err(format!("the AI engine stopped ({status}), see server.log"));
            }
            let health = ureq::get(&server.url("/health"))
                .config()
                .timeout_global(Some(Duration::from_secs(2)))
                .http_status_as_error(false)
                .build()
                .call();
            if health.is_ok_and(|r| r.status() == 200) {
                eprintln!("[ai] engine ready on port {port} in {:.1} s", started.elapsed().as_secs_f32());
                return Ok(server);
            }
            if started.elapsed() > READY_TIMEOUT {
                return Err("the AI engine took too long to load".to_string());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// One chat completion. `schema`: the JSON the answer must follow.
    pub fn chat(&self, messages: &[Message], schema: Option<Value>, max_tokens: u32, timeout: Duration) -> Result<String, String> {
        let mut body = json!({
            "messages": messages,
            "temperature": 0.3,
            "max_tokens": max_tokens,
        });
        if let Some(schema) = schema {
            body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "reply", "schema": schema } });
        }
        let started = Instant::now();
        let text = ureq::post(&self.url("/v1/chat/completions"))
            .config()
            .timeout_global(Some(timeout))
            .build()
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|err| format!("AI request failed: {err}"))?
            .body_mut()
            .read_to_string()
            .map_err(|err| err.to_string())?;
        let reply: Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
        let content = reply["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| format!("unexpected AI response: {text}"))?;
        eprintln!("[ai] answered in {:.1} s", started.elapsed().as_secs_f32());
        Ok(content.to_string())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(job) = self.job.take() {
            unsafe {
                let _ = CloseHandle(job);
            }
        }
    }
}

/// A port nobody listens on right now.
fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|err| format!("no free port: {err}"))?;
    listener.local_addr().map(|a| a.port()).map_err(|err| err.to_string())
}

/// Puts the child in a job that kills it when Mimo's handle closes (Mimo
/// exiting or crashing). Best effort: without it, `Drop` still stops it.
fn tie_to_mimo(child: &Child) -> Option<HANDLE> {
    unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set.is_err() || AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).is_err() {
            let _ = CloseHandle(job);
            return None;
        }
        Some(job)
    }
}

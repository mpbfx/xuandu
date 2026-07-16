use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::settings::AppSettings;

const MAX_RECENT_EVENTS: usize = 200;
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticEvent {
    timestamp_ms: u128,
    sequence: u64,
    category: String,
    code: Option<String>,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticExport {
    generated_at_ms: u128,
    app_version: &'static str,
    operating_system: &'static str,
    architecture: &'static str,
    settings: DiagnosticSettings,
    events: Vec<DiagnosticEvent>,
    privacy_note: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticSettings {
    speaker_id: String,
    uses_custom_speaker: bool,
    speech_rate: i32,
    loudness_rate: i32,
    pitch: i32,
    embed_voice_instruction: bool,
    shortcut: String,
    launch_at_login: bool,
}

pub struct DiagnosticStore {
    directory: PathBuf,
    log_path: PathBuf,
    sequence: AtomicU64,
    recent: Mutex<VecDeque<DiagnosticEvent>>,
}

impl DiagnosticStore {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let directory = app
            .path()
            .app_log_dir()
            .map_err(|error| format!("无法定位诊断目录：{error}"))?;
        fs::create_dir_all(&directory).map_err(|error| format!("无法创建诊断目录：{error}"))?;
        Ok(Self {
            log_path: directory.join("xuandu.log.jsonl"),
            directory,
            sequence: AtomicU64::new(0),
            recent: Mutex::new(VecDeque::new()),
        })
    }

    pub fn record(&self, category: &str, code: Option<&str>, message: &str) {
        let event = DiagnosticEvent {
            timestamp_ms: now_ms(),
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            category: sanitize(category),
            code: code.map(sanitize),
            message: sanitize(message),
        };
        let mut recent = self.recent.lock();
        if recent.len() == MAX_RECENT_EVENTS {
            recent.pop_front();
        }
        recent.push_back(event.clone());
        drop(recent);

        if fs::metadata(&self.log_path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
            let _ = fs::rename(&self.log_path, self.log_path.with_extension("jsonl.old"));
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    pub fn export(&self, settings: &AppSettings) -> Result<String, String> {
        let export = DiagnosticExport {
            generated_at_ms: now_ms(),
            app_version: env!("CARGO_PKG_VERSION"),
            operating_system: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            settings: DiagnosticSettings {
                speaker_id: settings.speaker_id.clone(),
                uses_custom_speaker: settings.custom_speaker_id.is_some(),
                speech_rate: settings.speech_rate,
                loudness_rate: settings.loudness_rate,
                pitch: settings.pitch,
                embed_voice_instruction: settings.embed_voice_instruction,
                shortcut: settings.shortcut.clone(),
                launch_at_login: settings.launch_at_login,
            },
            events: self.recent.lock().iter().cloned().collect(),
            privacy_note: "API Key and selected text are never included in this export.",
        };
        let path = self.directory.join(format!(
            "xuandu-diagnostics-{}.json",
            export.generated_at_ms
        ));
        let contents = serde_json::to_vec_pretty(&export)
            .map_err(|error| format!("无法生成诊断文件：{error}"))?;
        fs::write(&path, contents).map_err(|error| format!("无法写入诊断文件：{error}"))?;
        Ok(path.to_string_lossy().into_owned())
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn sanitize(value: &str) -> String {
    value.chars().take(240).collect()
}

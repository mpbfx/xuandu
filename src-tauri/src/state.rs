use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as AutoStartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::{
    audio::PlaybackController,
    diagnostics::DiagnosticStore,
    selection::{self, prepare_selection, selection_hash, PreparedSelection},
    settings::{AppSettings, SecretStore, SettingsPatch, SettingsStore},
    tts::{TtsClient, TtsOptions},
    voice_catalog::{find as find_voice, resource_id_for},
};

const CAPTURE_DELAY: Duration = Duration::from_millis(150);
const DUPLICATE_WINDOW: Duration = Duration::from_millis(1200);
const TEST_TEXT: &str = "欢迎使用选读。现在请听一段更完整的试听语音，用来感受当前音色、语速和表达方式是否符合你的预期。";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReadingMode {
    Off,
    Armed,
    Playing,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppNotice {
    pub kind: NoticeKind,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub mode: ReadingMode,
    pub has_api_key: bool,
    pub accessibility_trusted: bool,
    pub notice: Option<AppNotice>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutValidation {
    pub shortcut: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutError {
    pub code: &'static str,
    pub message: String,
}

impl ShortcutError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone)]
struct ShortcutRecordingSession {
    previous: String,
}

type StatusHook = Arc<dyn Fn(AppStatus) + Send + Sync>;

#[derive(Clone, Copy)]
struct LastSelection {
    fingerprint: u64,
    at: Instant,
}

pub struct AppState {
    app: AppHandle,
    store: SettingsStore,
    secrets: SecretStore,
    settings: Mutex<AppSettings>,
    mode: Mutex<ReadingMode>,
    notice: Mutex<Option<AppNotice>>,
    status_hook: Mutex<Option<StatusHook>>,
    audio: PlaybackController,
    tts: TtsClient,
    capture_epoch: AtomicU64,
    speech_epoch: AtomicU64,
    listener_started: AtomicBool,
    last_selection: Mutex<Option<LastSelection>>,
    shortcut_recording: Mutex<Option<ShortcutRecordingSession>>,
    diagnostics: DiagnosticStore,
}

impl AppState {
    pub fn new(app: AppHandle) -> Result<Self, String> {
        let store = SettingsStore::new(&app)?;
        let settings = store.load();
        let diagnostics = DiagnosticStore::new(&app)?;
        let secrets = SecretStore;
        let initial_mode = if secrets.get()?.is_some() {
            ReadingMode::Armed
        } else {
            ReadingMode::Off
        };
        diagnostics.record("lifecycle", Some("startup"), "application started");
        Ok(Self {
            app,
            store,
            secrets,
            settings: Mutex::new(settings),
            mode: Mutex::new(initial_mode),
            notice: Mutex::new(None),
            status_hook: Mutex::new(None),
            audio: PlaybackController::default(),
            tts: TtsClient::default(),
            capture_epoch: AtomicU64::new(0),
            speech_epoch: AtomicU64::new(0),
            listener_started: AtomicBool::new(false),
            last_selection: Mutex::new(None),
            shortcut_recording: Mutex::new(None),
            diagnostics,
        })
    }

    pub fn status(&self) -> AppStatus {
        let secret = self.secrets.get();
        AppStatus {
            mode: *self.mode.lock(),
            has_api_key: secret
                .as_ref()
                .ok()
                .and_then(|value| value.as_ref())
                .is_some(),
            accessibility_trusted: selection::is_accessibility_trusted(),
            notice: self.notice.lock().clone().or_else(|| {
                secret.err().map(|_| AppNotice {
                    kind: NoticeKind::Error,
                    code: Some("keychain".to_owned()),
                    message: "无法访问系统钥匙串，请重新保存 API Key。".to_owned(),
                })
            }),
        }
    }

    pub fn settings(&self) -> AppSettings {
        self.settings.lock().clone()
    }

    pub fn needs_onboarding(&self) -> bool {
        let status = self.status();
        !status.has_api_key || !status.accessibility_trusted
    }

    pub fn set_status_hook(&self, hook: StatusHook) {
        *self.status_hook.lock() = Some(hook);
        self.emit_status();
    }

    pub fn install_global_shortcut(&self) -> Result<(), String> {
        let shortcut = parse_shortcut(&self.settings.lock().shortcut)?;
        self.app
            .global_shortcut()
            .register(shortcut)
            .map_err(|error| format!("无法注册快捷键：{error}"))
    }

    pub fn validate_shortcut(&self, shortcut: &str) -> Result<ShortcutValidation, ShortcutError> {
        validate_shortcut_value(shortcut)
    }

    pub fn begin_shortcut_recording(&self) -> Result<(), ShortcutError> {
        let mut session = self.shortcut_recording.lock();
        if session.is_some() {
            return Ok(());
        }

        let previous = self.settings().shortcut;
        let parsed = parse_shortcut(&previous)
            .map_err(|message| ShortcutError::new("invalid_combination", message))?;
        self.app
            .global_shortcut()
            .unregister(parsed)
            .map_err(|error| {
                ShortcutError::new(
                    "registration_failed",
                    format!("无法开始录制快捷键：{error}"),
                )
            })?;
        *session = Some(ShortcutRecordingSession { previous });
        self.diagnostics.record(
            "shortcut",
            Some("recording_started"),
            "shortcut recording started",
        );
        Ok(())
    }

    pub fn cancel_shortcut_recording(&self) -> Result<(), ShortcutError> {
        let Some(session) = self.shortcut_recording.lock().take() else {
            return Ok(());
        };
        self.diagnostics.record(
            "shortcut",
            Some("recording_cancelled"),
            "shortcut recording cancelled",
        );
        self.restore_shortcut(&session.previous)
    }

    pub fn commit_shortcut_recording(&self, shortcut: &str) -> Result<AppSettings, ShortcutError> {
        let session = self.shortcut_recording.lock().take().ok_or_else(|| {
            ShortcutError::new(
                "registration_failed",
                "快捷键录制会话已结束，请重新点击录制。",
            )
        })?;
        let validation = match self.validate_shortcut(shortcut) {
            Ok(validation) => validation,
            Err(error) => {
                let _ = self.restore_shortcut(&session.previous);
                return Err(error);
            }
        };
        let previous = self.settings();

        if validation.shortcut == session.previous {
            self.restore_shortcut(&session.previous)?;
            return Ok(previous);
        }

        let next_shortcut = parse_shortcut(&validation.shortcut)
            .map_err(|message| ShortcutError::new("invalid_combination", message))?;
        if let Err(error) = self.app.global_shortcut().register(next_shortcut) {
            let _ = self.restore_shortcut(&session.previous);
            return Err(ShortcutError::new(
                "already_registered",
                format!("这个快捷键可能已被系统或其他应用占用：{error}"),
            ));
        }

        let next = match previous.apply_patch(SettingsPatch {
            shortcut: Some(validation.shortcut),
            ..Default::default()
        }) {
            Ok(next) => next,
            Err(message) => {
                let _ = self.app.global_shortcut().unregister(next_shortcut);
                let _ = self.restore_shortcut(&session.previous);
                return Err(ShortcutError::new("invalid_combination", message));
            }
        };
        if let Err(error) = self.store.save(&next) {
            let _ = self.app.global_shortcut().unregister(next_shortcut);
            let _ = self.restore_shortcut(&session.previous);
            return Err(ShortcutError::new("persistence_failed", error));
        }

        *self.settings.lock() = next.clone();
        self.diagnostics
            .record("shortcut", Some("updated"), "shortcut updated successfully");
        self.set_notice(NoticeKind::Success, None, "快捷键已更新。".to_owned());
        Ok(next)
    }

    pub fn update_settings(&self, patch: SettingsPatch) -> Result<AppSettings, String> {
        let previous = self.settings();
        let next = previous.apply_patch(patch)?;

        if previous.shortcut != next.shortcut {
            self.replace_global_shortcut(&previous.shortcut, &next.shortcut)?;
        }
        if previous.launch_at_login != next.launch_at_login {
            if let Err(error) = self.set_autostart(next.launch_at_login) {
                if previous.shortcut != next.shortcut {
                    let _ = self.replace_global_shortcut(&next.shortcut, &previous.shortcut);
                }
                return Err(error);
            }
        }
        if let Err(error) = self.store.save(&next) {
            if previous.launch_at_login != next.launch_at_login {
                let _ = self.set_autostart(previous.launch_at_login);
            }
            if previous.shortcut != next.shortcut {
                let _ = self.replace_global_shortcut(&next.shortcut, &previous.shortcut);
            }
            return Err(error);
        }

        *self.settings.lock() = next.clone();
        self.set_notice(NoticeKind::Success, None, "已自动保存。".to_owned());
        Ok(next)
    }

    pub fn save_api_key(&self, api_key: &str) -> Result<(), String> {
        self.secrets.set(api_key)?;
        *self.mode.lock() = ReadingMode::Armed;
        self.diagnostics.record(
            "credentials",
            Some("saved"),
            "API credential saved in system keychain",
        );
        self.set_notice(
            NoticeKind::Success,
            None,
            "API Key 已保存到系统钥匙串。".to_owned(),
        );
        Ok(())
    }

    pub fn clear_api_key(&self) -> Result<(), String> {
        self.audio.stop();
        self.speech_epoch.fetch_add(1, Ordering::SeqCst);
        *self.mode.lock() = ReadingMode::Off;
        self.secrets.clear()?;
        self.diagnostics
            .record("credentials", Some("cleared"), "API credential cleared");
        self.set_notice(
            NoticeKind::Info,
            None,
            "API Key 已从系统钥匙串清除。".to_owned(),
        );
        Ok(())
    }

    pub fn request_accessibility(&self) -> Result<(), String> {
        selection::request_accessibility()
    }

    pub fn toggle_reading(self: &Arc<Self>) -> Result<AppStatus, String> {
        let current = *self.mode.lock();
        if matches!(current, ReadingMode::Armed | ReadingMode::Playing) {
            self.stop_reading();
            return Ok(self.status());
        }

        if self.secrets.get()?.is_none() {
            *self.mode.lock() = ReadingMode::Error;
            self.set_notice(
                NoticeKind::Error,
                Some("api-key".to_owned()),
                "请先在高级设置中添加火山引擎 API Key。".to_owned(),
            );
            return Ok(self.status());
        }
        if !selection::is_accessibility_trusted() {
            *self.mode.lock() = ReadingMode::Error;
            self.set_notice(
                NoticeKind::Error,
                Some("accessibility".to_owned()),
                "请先授予辅助功能权限。".to_owned(),
            );
            return Ok(self.status());
        }
        self.ensure_selection_listener()?;

        *self.mode.lock() = ReadingMode::Armed;
        self.set_notice(
            NoticeKind::Info,
            None,
            "朗读模式已开启：用鼠标选中文字即可朗读。".to_owned(),
        );
        Ok(self.emit_status())
    }

    pub fn stop_playback(&self) -> AppStatus {
        self.speech_epoch.fetch_add(1, Ordering::SeqCst);
        self.audio.stop();
        self.diagnostics
            .record("playback", Some("stopped"), "playback stopped");
        if *self.mode.lock() == ReadingMode::Playing {
            *self.mode.lock() = ReadingMode::Armed;
        }
        self.set_notice(NoticeKind::Info, None, "已停止当前朗读。".to_owned());
        self.status()
    }

    pub fn start_selection_listener(self: &Arc<Self>) {
        let _ = self.ensure_selection_listener();
    }

    pub async fn preview_voice(
        self: &Arc<Self>,
        speaker_id: String,
        apply_instruction: bool,
    ) -> Result<(), String> {
        let voice =
            find_voice(&speaker_id).ok_or_else(|| "该音色不在内置中文 2.0 目录中。".to_owned())?;
        let instruction_enabled = apply_instruction
            && self
                .settings()
                .voice_instruction
                .as_ref()
                .is_some_and(|instruction| !instruction.trim().is_empty());
        self.diagnostics.record(
            "preview",
            Some(if instruction_enabled {
                "instruction"
            } else {
                "original"
            }),
            if instruction_enabled {
                "instruction preview started"
            } else {
                "original preview started"
            },
        );
        self.preview_speech(
            voice.speaker_id.clone(),
            voice.resource_id.clone(),
            TEST_TEXT,
            apply_instruction,
            format!(
                "已完成 {} {}。",
                voice.name,
                if instruction_enabled {
                    "语音指令试听"
                } else {
                    "原声试听"
                }
            ),
        )
        .await
    }

    pub fn show_settings(&self) {
        if let Some(window) = self.app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }

    pub fn export_diagnostics(&self) -> Result<String, String> {
        self.diagnostics.record(
            "diagnostics",
            Some("export"),
            "diagnostics exported by user",
        );
        self.diagnostics.export(&self.settings())
    }

    fn replace_global_shortcut(&self, previous: &str, next: &str) -> Result<(), String> {
        let manager = self.app.global_shortcut();
        let previous_shortcut = parse_shortcut(previous)?;
        let next_shortcut = parse_shortcut(next)?;
        manager
            .unregister(previous_shortcut)
            .map_err(|error| format!("无法更新快捷键：{error}"))?;
        if let Err(error) = manager.register(next_shortcut) {
            let _ = manager.register(parse_shortcut(previous)?);
            return Err(format!("快捷键不可用，可能已被其他应用占用：{error}"));
        }
        Ok(())
    }

    fn restore_shortcut(&self, shortcut: &str) -> Result<(), ShortcutError> {
        let parsed = parse_shortcut(shortcut)
            .map_err(|message| ShortcutError::new("invalid_combination", message))?;
        self.app
            .global_shortcut()
            .register(parsed)
            .map_err(|error| {
                ShortcutError::new("registration_failed", format!("无法恢复原快捷键：{error}"))
            })
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), String> {
        let manager = self.app.autolaunch();
        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        result.map_err(|error| format!("无法更新登录启动：{error}"))
    }

    async fn preview_speech(
        self: &Arc<Self>,
        speaker: String,
        resource_id: String,
        text: &str,
        apply_instruction: bool,
        success_message: impl Into<String>,
    ) -> Result<(), String> {
        if self.secrets.get()?.is_none() {
            return Err("请先添加火山引擎 API Key。".to_owned());
        }
        let resume_mode = match *self.mode.lock() {
            ReadingMode::Armed | ReadingMode::Playing => ReadingMode::Armed,
            ReadingMode::Off | ReadingMode::Error => ReadingMode::Off,
        };
        let job_id = self.speech_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.audio.stop();
        let result = self
            .run_speech_with_voice(job_id, text, speaker, resource_id, apply_instruction)
            .await;
        if self.speech_epoch.load(Ordering::SeqCst) != job_id {
            return Ok(());
        }

        match &result {
            Ok(()) => {
                *self.mode.lock() = resume_mode;
                self.set_notice(NoticeKind::Success, None, success_message.into());
            }
            Err(message) => {
                *self.mode.lock() = if resume_mode == ReadingMode::Armed {
                    ReadingMode::Armed
                } else {
                    ReadingMode::Error
                };
                self.set_notice(NoticeKind::Error, Some("tts".to_owned()), message.clone());
            }
        }
        result
    }

    fn schedule_capture(self: &Arc<Self>) {
        let capture_id = self.capture_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let state = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(CAPTURE_DELAY).await;
            if state.capture_epoch.load(Ordering::SeqCst) != capture_id || !state.is_listening() {
                return;
            }
            state.capture_current_selection();
        });
    }

    fn ensure_selection_listener(self: &Arc<Self>) -> Result<(), String> {
        if !selection::is_accessibility_trusted() || self.listener_started.load(Ordering::SeqCst) {
            return Ok(());
        }
        if self
            .listener_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }

        let state = Arc::downgrade(self);
        if let Err(error) = selection::start_mouse_release_listener(move || {
            if let Some(state) = state.upgrade() {
                state.schedule_capture();
            }
        }) {
            self.listener_started.store(false, Ordering::SeqCst);
            return Err(error);
        }
        Ok(())
    }

    fn capture_current_selection(self: &Arc<Self>) {
        let Ok(Some(raw)) = selection::selected_text() else {
            return;
        };
        match prepare_selection(&raw) {
            PreparedSelection::Ignore => {}
            PreparedSelection::TooLong => {
                self.set_notice(
                    NoticeKind::Warning,
                    Some("selection-length".to_owned()),
                    "选中文字超过 5000 字，请缩短后再朗读。".to_owned(),
                );
            }
            PreparedSelection::Speak(text) if self.is_duplicate(&text) => {}
            PreparedSelection::Speak(text) => self.start_speech(&text, true),
        }
    }

    fn start_speech(self: &Arc<Self>, text: &str, resume_armed: bool) {
        let job_id = self.speech_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.audio.stop();
        let state = Arc::clone(self);
        let text = text.to_owned();
        tauri::async_runtime::spawn(async move {
            let settings = state.settings();
            let result = state
                .run_speech_with_voice(
                    job_id,
                    &text,
                    settings.active_speaker().to_owned(),
                    resource_id_for(settings.active_speaker()).to_owned(),
                    true,
                )
                .await;
            if state.speech_epoch.load(Ordering::SeqCst) != job_id {
                return;
            }

            match result {
                Ok(()) => {
                    *state.mode.lock() = if resume_armed {
                        ReadingMode::Armed
                    } else {
                        ReadingMode::Off
                    };
                    state.clear_notice();
                }
                Err(message) => {
                    *state.mode.lock() = if resume_armed {
                        ReadingMode::Armed
                    } else {
                        ReadingMode::Error
                    };
                    state.set_notice(NoticeKind::Error, Some("tts".to_owned()), message);
                }
            }
        });
    }

    async fn run_speech_with_voice(
        &self,
        job_id: u64,
        text: &str,
        speaker: String,
        resource_id: String,
        apply_instruction: bool,
    ) -> Result<(), String> {
        let api_key = self
            .secrets
            .get()?
            .ok_or_else(|| "请先添加火山引擎 API Key。".to_owned())?;
        let playback = self.audio.start()?;
        *self.mode.lock() = ReadingMode::Playing;
        self.clear_notice();

        let settings = self.settings();
        let voice_instruction = supported_voice_instruction(
            &speaker,
            settings.voice_instruction.clone(),
            apply_instruction,
        );
        if let Some(instruction) = voice_instruction.as_ref() {
            self.diagnostics.record(
                "tts",
                Some("voice_instruction_attached"),
                &format!(
                    "voice instruction attached ({} chars)",
                    instruction.chars().count()
                ),
            );
        }
        let options = TtsOptions {
            api_key,
            resource_id,
            speaker,
            speech_rate: settings.speech_rate,
            loudness_rate: settings.loudness_rate,
            pitch: settings.pitch,
            voice_instruction,
            sample_rate: playback.sample_rate(),
        };
        let result = self.tts.synthesize_into(text, &options, &playback).await;
        if result.is_err() && self.speech_epoch.load(Ordering::SeqCst) == job_id {
            self.audio.stop();
        }
        result
    }

    fn stop_reading(&self) {
        self.speech_epoch.fetch_add(1, Ordering::SeqCst);
        self.audio.stop();
        *self.mode.lock() = ReadingMode::Off;
        self.set_notice(NoticeKind::Info, None, "朗读模式已关闭。".to_owned());
    }

    fn is_listening(&self) -> bool {
        matches!(*self.mode.lock(), ReadingMode::Armed | ReadingMode::Playing)
    }

    fn is_duplicate(&self, text: &str) -> bool {
        let candidate = LastSelection {
            fingerprint: selection_hash(text),
            at: Instant::now(),
        };
        let mut last = self.last_selection.lock();
        let duplicate = last.as_ref().is_some_and(|previous| {
            previous.fingerprint == candidate.fingerprint
                && candidate.at.duration_since(previous.at) < DUPLICATE_WINDOW
        });
        if !duplicate {
            *last = Some(candidate);
        }
        duplicate
    }

    fn set_notice(&self, kind: NoticeKind, code: Option<String>, message: String) {
        if matches!(kind, NoticeKind::Warning | NoticeKind::Error) {
            self.diagnostics.record(
                "notice",
                code.as_deref(),
                if kind == NoticeKind::Error {
                    "error surfaced"
                } else {
                    "warning surfaced"
                },
            );
        }
        *self.notice.lock() = Some(AppNotice {
            kind,
            code,
            message,
        });
        self.emit_status();
    }

    fn clear_notice(&self) {
        *self.notice.lock() = None;
        self.emit_status();
    }

    fn emit_status(&self) -> AppStatus {
        let status = self.status();
        if let Some(hook) = self.status_hook.lock().clone() {
            hook(status.clone());
        }
        let _ = self.app.emit("app-state", status.clone());
        status
    }
}

fn parse_shortcut(value: &str) -> Result<Shortcut, String> {
    value
        .parse()
        .map_err(|_| "快捷键格式无效，例如 Command+Option+R 或 Ctrl+Alt+R。".to_owned())
}

fn supported_voice_instruction(
    speaker: &str,
    instruction: Option<String>,
    enabled: bool,
) -> Option<String> {
    (enabled && find_voice(speaker).is_some())
        .then_some(instruction)
        .flatten()
}

fn validate_shortcut_value(value: &str) -> Result<ShortcutValidation, ShortcutError> {
    let parts: Vec<_> = value
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let (key, modifiers) = parts
        .split_last()
        .ok_or_else(|| ShortcutError::new("invalid_combination", "请按下修饰键和一个普通按键。"))?;
    if modifiers.is_empty() || modifiers.iter().any(|part| !is_modifier(part)) || is_modifier(key) {
        return Err(ShortcutError::new(
            "invalid_combination",
            "快捷键必须包含 Command、Ctrl、Option/Alt 或 Shift，以及一个普通按键。",
        ));
    }

    let mut normalized_modifiers = Vec::new();
    for expected in ["Command", "Ctrl", "Option", "Alt", "Shift"] {
        if modifiers
            .iter()
            .any(|part| normalize_modifier(part) == Some(expected))
        {
            normalized_modifiers.push(expected);
        }
    }
    if normalized_modifiers.contains(&"Option") && normalized_modifiers.contains(&"Alt") {
        return Err(ShortcutError::new(
            "invalid_combination",
            "Option 和 Alt 不能同时使用。",
        ));
    }

    let normalized_key = normalize_key(key);
    let normalized = normalized_modifiers
        .into_iter()
        .chain(std::iter::once(normalized_key.as_str()))
        .collect::<Vec<_>>()
        .join("+");
    if is_reserved_shortcut(&normalized) {
        return Err(ShortcutError::new(
            "reserved_combination",
            "该组合键由系统或常用窗口操作保留，请换一个组合。",
        ));
    }
    parse_shortcut(&normalized)
        .map_err(|message| ShortcutError::new("invalid_combination", message))?;
    Ok(ShortcutValidation {
        shortcut: normalized,
    })
}

fn is_modifier(value: &str) -> bool {
    normalize_modifier(value).is_some()
}

fn normalize_modifier(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "command" | "cmd" | "meta" | "super" => Some("Command"),
        "control" | "ctrl" => Some("Ctrl"),
        "option" => Some("Option"),
        "alt" => Some("Alt"),
        "shift" => Some("Shift"),
        _ => None,
    }
}

fn normalize_key(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "esc" => "Escape".to_owned(),
        "del" => "Delete".to_owned(),
        "spacebar" | " " => "Space".to_owned(),
        _ if value.chars().count() == 1 => value.to_uppercase(),
        _ => value.to_owned(),
    }
}

fn is_reserved_shortcut(value: &str) -> bool {
    matches!(
        value,
        "Command+Space"
            | "Command+Tab"
            | "Command+Q"
            | "Command+W"
            | "Command+Option+Escape"
            | "Alt+Tab"
            | "Ctrl+Alt+Delete"
            | "Command+L"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        supported_voice_instruction, validate_shortcut_value, Duration, Instant, LastSelection,
        DUPLICATE_WINDOW,
    };
    use crate::voice_catalog::DEFAULT_SPEAKER;

    #[test]
    fn duplicate_window_is_short_lived() {
        let now = Instant::now();
        let previous = LastSelection {
            fingerprint: 42,
            at: now,
        };
        assert!(now.duration_since(previous.at) < DUPLICATE_WINDOW);
        assert!(now + Duration::from_millis(1201) > now + DUPLICATE_WINDOW);
    }

    #[test]
    fn shortcut_validation_normalizes_modifiers_and_key() {
        let result = validate_shortcut_value("cmd+option+r").unwrap();
        assert_eq!(result.shortcut, "Command+Option+R");
    }

    #[test]
    fn shortcut_validation_requires_a_modifier_and_regular_key() {
        assert_eq!(
            validate_shortcut_value("R").unwrap_err().code,
            "invalid_combination"
        );
        assert_eq!(
            validate_shortcut_value("Command").unwrap_err().code,
            "invalid_combination"
        );
    }

    #[test]
    fn shortcut_validation_blocks_reserved_combinations() {
        assert_eq!(
            validate_shortcut_value("Command+Space").unwrap_err().code,
            "reserved_combination"
        );
    }

    #[test]
    fn voice_instruction_is_only_sent_for_bundled_seed_tts_two_voices() {
        let instruction = Some("请用温柔的语气说话。".to_owned());
        assert_eq!(
            supported_voice_instruction(DEFAULT_SPEAKER, instruction.clone(), true),
            instruction
        );
        assert!(supported_voice_instruction("custom-speaker", instruction.clone(), true).is_none());
        assert!(supported_voice_instruction(DEFAULT_SPEAKER, instruction, false).is_none());
    }
}

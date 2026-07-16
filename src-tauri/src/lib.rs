mod audio;
mod diagnostics;
mod selection;
mod settings;
mod state;
mod tts;
mod voice_catalog;

use std::sync::Arc;

use state::{AppState, AppStatus, NoticeKind, ReadingMode, ShortcutError, ShortcutValidation};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    App, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{Builder as GlobalShortcutBuilder, ShortcutState};

use crate::{
    settings::{AppSettings, SettingsPatch},
    voice_catalog::VoiceCatalog,
};

#[tauri::command]
fn get_status(state: State<'_, Arc<AppState>>) -> AppStatus {
    state.status()
}

#[tauri::command]
fn get_settings(state: State<'_, Arc<AppState>>) -> AppSettings {
    state.settings()
}

#[tauri::command]
fn get_voice_catalog() -> VoiceCatalog {
    voice_catalog::catalog().clone()
}

#[tauri::command]
fn update_settings(
    patch: SettingsPatch,
    state: State<'_, Arc<AppState>>,
) -> Result<AppSettings, String> {
    state.update_settings(patch)
}

#[tauri::command]
fn save_api_key(api_key: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.save_api_key(&api_key)
}

#[tauri::command]
fn toggle_reading(state: State<'_, Arc<AppState>>) -> Result<AppStatus, String> {
    state.toggle_reading()
}

#[tauri::command]
async fn preview_voice(speaker_id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.preview_voice(speaker_id).await
}

#[tauri::command]
fn stop_playback(state: State<'_, Arc<AppState>>) -> AppStatus {
    state.stop_playback()
}

#[tauri::command]
fn request_accessibility(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.request_accessibility()
}

#[tauri::command]
fn clear_api_key(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.clear_api_key()
}

#[tauri::command]
fn validate_shortcut(
    shortcut: String,
    state: State<'_, Arc<AppState>>,
) -> Result<ShortcutValidation, ShortcutError> {
    state.validate_shortcut(&shortcut)
}

#[tauri::command]
fn begin_shortcut_recording(state: State<'_, Arc<AppState>>) -> Result<(), ShortcutError> {
    state.begin_shortcut_recording()
}

#[tauri::command]
fn commit_shortcut_recording(
    shortcut: String,
    state: State<'_, Arc<AppState>>,
) -> Result<AppSettings, ShortcutError> {
    state.commit_shortcut_recording(&shortcut)
}

#[tauri::command]
fn cancel_shortcut_recording(state: State<'_, Arc<AppState>>) -> Result<(), ShortcutError> {
    state.cancel_shortcut_recording()
}

#[tauri::command]
fn export_diagnostics(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    state.export_diagnostics()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--from-autostart"]),
        ))
        .plugin(
            GlobalShortcutBuilder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let state = app.state::<Arc<AppState>>();
                        let _ = state.toggle_reading();
                    }
                })
                .build(),
        )
        .setup(|app| {
            let state = Arc::new(AppState::new(app.handle().clone())?);
            app.manage(Arc::clone(&state));
            state.install_global_shortcut()?;
            let tray = Arc::new(setup_tray(app)?);
            let hook_state = Arc::clone(&state);
            let hook_tray = Arc::clone(&tray);
            state.set_status_hook(Arc::new(move |status| {
                hook_tray.update(&status, &hook_state.settings());
            }));
            state.start_selection_listener();

            if launched_from_login() && !state.needs_onboarding() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Arc<AppState>>();
                let _ = state.cancel_shortcut_recording();
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_settings,
            get_voice_catalog,
            update_settings,
            save_api_key,
            toggle_reading,
            preview_voice,
            stop_playback,
            request_accessibility,
            clear_api_key,
            validate_shortcut,
            begin_shortcut_recording,
            commit_shortcut_recording,
            cancel_shortcut_recording,
            export_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Xuandu desktop application");
}

struct TrayUi<R: tauri::Runtime> {
    icon: TrayIcon<R>,
    toggle: MenuItem<R>,
    stop: MenuItem<R>,
    current_voice: MenuItem<R>,
    launch_at_login: CheckMenuItem<R>,
}

impl<R: tauri::Runtime> TrayUi<R> {
    fn update(&self, status: &AppStatus, settings: &AppSettings) {
        let reading_is_on = matches!(status.mode, ReadingMode::Armed | ReadingMode::Playing);
        let status_text = tray_status_text(status);
        let shortcut = settings
            .shortcut
            .replace("Command", "⌘")
            .replace("Option", "⌥");
        let _ = self.toggle.set_text(if reading_is_on {
            "关闭朗读"
        } else {
            "开启朗读"
        });
        let _ = self.stop.set_enabled(status.mode == ReadingMode::Playing);
        let _ = self.current_voice.set_text(format!(
            "当前音色：{}",
            voice_catalog::find(settings.active_speaker())
                .map(|voice| voice.name.as_str())
                .unwrap_or("自定义音色")
        ));
        let _ = self.launch_at_login.set_checked(settings.launch_at_login);
        let _ = self
            .icon
            .set_tooltip(Some(format!("选读 · {status_text} · {shortcut}")));
        let _ = self.icon.set_icon(Some(tray_icon_for(status)));
    }
}

fn setup_tray<R: tauri::Runtime>(app: &App<R>) -> tauri::Result<TrayUi<R>> {
    let toggle = MenuItem::with_id(app, "toggle", "开启朗读", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止播放", false, None::<&str>)?;
    let current_voice = MenuItem::with_id(app, "current-voice", "当前音色", false, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "打开设置", true, None::<&str>)?;
    let launch_at_login = CheckMenuItem::with_id(
        app,
        "launch-at-login",
        "登录时启动",
        true,
        false,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "退出选读", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator_bottom = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &stop,
            &separator,
            &current_voice,
            &settings,
            &launch_at_login,
            &separator_bottom,
            &quit,
        ],
    )?;

    let icon = TrayIconBuilder::with_id("xuandu-tray")
        .icon(tray_icon_for(&AppStatus {
            mode: ReadingMode::Off,
            has_api_key: false,
            accessibility_trusted: false,
            notice: None,
        }))
        .icon_as_template(false)
        .tooltip("选读")
        .menu(&menu)
        .on_menu_event(|app, event| {
            let state = app.state::<Arc<AppState>>();
            match event.id.as_ref() {
                "toggle" => {
                    let _ = state.toggle_reading();
                }
                "stop" => {
                    state.stop_playback();
                }
                "settings" => state.show_settings(),
                "launch-at-login" => {
                    let enabled = !state.settings().launch_at_login;
                    let _ = state.update_settings(SettingsPatch {
                        launch_at_login: Some(enabled),
                        ..Default::default()
                    });
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .build(app)?;
    Ok(TrayUi {
        icon,
        toggle,
        stop,
        current_voice,
        launch_at_login,
    })
}

fn launched_from_login() -> bool {
    std::env::args().any(|argument| argument == "--from-autostart")
}

fn show_settings_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn tray_status_text(status: &AppStatus) -> &'static str {
    if matches!(
        status.notice.as_ref().map(|notice| notice.kind),
        Some(NoticeKind::Error)
    ) {
        return "需要处理";
    }
    match status.mode {
        ReadingMode::Off => "朗读已关闭",
        ReadingMode::Armed => "等待选中文字",
        ReadingMode::Playing => "正在朗读",
        ReadingMode::Error => "需要处理",
    }
}

fn tray_icon_for(status: &AppStatus) -> Image<'static> {
    let color = if matches!(
        status.notice.as_ref().map(|notice| notice.kind),
        Some(NoticeKind::Error)
    ) {
        [183, 76, 61, 255]
    } else {
        match status.mode {
            ReadingMode::Off => [128, 132, 124, 255],
            ReadingMode::Armed => [45, 97, 72, 255],
            ReadingMode::Playing => [190, 126, 41, 255],
            ReadingMode::Error => [183, 76, 61, 255],
        }
    };
    let size = 20_u32;
    let mut pixels = vec![0_u8; (size * size * 4) as usize];
    let stroke = color;
    for y in 0..size {
        for x in 0..size {
            let wave = match status.mode {
                ReadingMode::Off => y == 10 && (4..=15).contains(&x),
                ReadingMode::Armed => matches!(
                    (x, y),
                    (4, 8..=11) | (8, 6..=13) | (12, 4..=15) | (16, 7..=12)
                ),
                ReadingMode::Playing => matches!(
                    (x, y),
                    (3, 7..=12) | (7, 4..=15) | (11, 2..=17) | (15, 5..=14) | (18, 8..=11)
                ),
                ReadingMode::Error => x == y || x + y == size - 1,
            };
            let selected_text = (3..=16).contains(&x) && (y == 2 || y == 17);
            if wave || selected_text {
                let index = ((y * size + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&stroke);
            }
        }
    }
    Image::new_owned(pixels, size, size)
}

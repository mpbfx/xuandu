use std::{collections::HashSet, fs, path::PathBuf};

use keyring::Entry;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::voice_catalog::{find as find_voice, DEFAULT_SPEAKER};

const KEYRING_SERVICE: &str = "com.mpbfx.xuandu";
const KEYRING_ACCOUNT: &str = "volcengine-api-key";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub speaker_id: String,
    pub custom_speaker_id: Option<String>,
    pub speech_rate: i32,
    pub loudness_rate: i32,
    pub shortcut: String,
    #[serde(default)]
    pub favorite_speaker_ids: Vec<String>,
    #[serde(default)]
    pub launch_at_login: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            speaker_id: DEFAULT_SPEAKER.to_owned(),
            custom_speaker_id: None,
            speech_rate: 0,
            loudness_rate: 0,
            shortcut: default_shortcut().to_owned(),
            favorite_speaker_ids: Vec::new(),
            launch_at_login: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub speaker_id: Option<String>,
    pub custom_speaker_id: Option<String>,
    pub speech_rate: Option<i32>,
    pub loudness_rate: Option<i32>,
    pub shortcut: Option<String>,
    pub favorite_speaker_ids: Option<Vec<String>>,
    pub launch_at_login: Option<bool>,
}

impl AppSettings {
    pub fn apply_patch(&self, patch: SettingsPatch) -> Result<Self, String> {
        let mut next = self.clone();

        if let Some(speaker_id) = patch.speaker_id {
            next.speaker_id = speaker_id.trim().to_owned();
        }
        if let Some(custom_speaker_id) = patch.custom_speaker_id {
            next.custom_speaker_id = normalize_optional_text(&custom_speaker_id);
        }
        if let Some(speech_rate) = patch.speech_rate {
            next.speech_rate = speech_rate;
        }
        if let Some(loudness_rate) = patch.loudness_rate {
            next.loudness_rate = loudness_rate;
        }
        if let Some(shortcut) = patch.shortcut {
            next.shortcut = shortcut.trim().to_owned();
        }
        if let Some(favorites) = patch.favorite_speaker_ids {
            next.favorite_speaker_ids = normalize_favorites(favorites);
        }
        if let Some(launch_at_login) = patch.launch_at_login {
            next.launch_at_login = launch_at_login;
        }

        next.validate()?;
        Ok(next)
    }

    pub fn active_speaker(&self) -> &str {
        self.custom_speaker_id
            .as_deref()
            .filter(|speaker| !speaker.is_empty())
            .unwrap_or(&self.speaker_id)
    }

    fn validate(&self) -> Result<(), String> {
        if self.speaker_id.is_empty() && self.custom_speaker_id.is_none() {
            return Err("请选择音色，或填写自定义 Speaker ID。".to_owned());
        }
        if !(-50..=100).contains(&self.speech_rate) {
            return Err("语速必须在 0.5× 到 2.0× 之间。".to_owned());
        }
        if !(-50..=100).contains(&self.loudness_rate) {
            return Err("音量必须在 0.5× 到 2.0× 之间。".to_owned());
        }
        if self.shortcut.is_empty() {
            return Err("请设置一个模式切换快捷键。".to_owned());
        }
        if self.custom_speaker_id.is_none() && find_voice(&self.speaker_id).is_none() {
            return Err("所选音色不在内置的中文 2.0 目录中。".to_owned());
        }
        Ok(())
    }
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let directory = app
            .path()
            .app_config_dir()
            .map_err(|error| format!("无法定位本机设置目录：{error}"))?;
        Ok(Self {
            path: directory.join("settings.json"),
        })
    }

    pub fn load(&self) -> AppSettings {
        let Ok(contents) = fs::read_to_string(&self.path) else {
            return AppSettings::default();
        };
        let mut settings = serde_json::from_str(&contents).unwrap_or_default();
        if migrate_legacy_settings(&mut settings) {
            let _ = self.save(&settings);
        }
        settings
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), String> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| "本机设置目录无效。".to_owned())?;
        fs::create_dir_all(directory).map_err(|error| format!("无法创建设置目录：{error}"))?;

        let temporary = self.path.with_extension("json.tmp");
        let contents = serde_json::to_vec_pretty(settings)
            .map_err(|error| format!("无法编码设置：{error}"))?;
        fs::write(&temporary, contents).map_err(|error| format!("无法写入设置：{error}"))?;
        fs::rename(temporary, &self.path).map_err(|error| format!("无法保存设置：{error}"))
    }
}

fn normalize_optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn normalize_favorites(favorites: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    favorites
        .into_iter()
        .map(|speaker_id| speaker_id.trim().to_owned())
        .filter(|speaker_id| find_voice(speaker_id).is_some())
        .filter(|speaker_id| seen.insert(speaker_id.clone()))
        .collect()
}

fn migrate_legacy_settings(settings: &mut AppSettings) -> bool {
    let mut changed = false;
    if settings.custom_speaker_id.is_none()
        && (settings.speaker_id.ends_with("_mars_bigtts")
            || settings.speaker_id.ends_with("_moon_bigtts"))
    {
        settings.speaker_id = DEFAULT_SPEAKER.to_owned();
        changed = true;
    }
    let favorites = normalize_favorites(std::mem::take(&mut settings.favorite_speaker_ids));
    if settings.favorite_speaker_ids != favorites {
        settings.favorite_speaker_ids = favorites;
        changed = true;
    }
    changed
}

#[derive(Default)]
pub struct SecretStore;

impl SecretStore {
    fn entry(&self) -> Result<Entry, String> {
        Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|error| format!("无法访问系统钥匙串：{error}"))
    }

    pub fn get(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(value) if value.trim().is_empty() => Ok(None),
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("无法读取系统钥匙串：{error}")),
        }
    }

    pub fn set(&self, value: &str) -> Result<(), String> {
        let value = value.trim();
        if value.is_empty() {
            return Err("请输入 API Key。".to_owned());
        }
        let entry = self.entry()?;
        entry
            .set_password(value)
            .map_err(|error| format!("无法保存到系统钥匙串：{error}"))?;

        let stored = entry
            .get_password()
            .map_err(|error| format!("系统钥匙串未能确认 API Key：{error}"))?;
        if stored == value {
            Ok(())
        } else {
            Err("系统钥匙串未能确认 API Key；未更新设置。".to_owned())
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("无法清除系统钥匙串：{error}")),
        }
    }
}

#[cfg(target_os = "macos")]
pub const fn default_shortcut() -> &'static str {
    "Command+Option+R"
}

#[cfg(not(target_os = "macos"))]
pub const fn default_shortcut() -> &'static str {
    "Ctrl+Alt+R"
}

#[cfg(test)]
mod tests {
    use super::{migrate_legacy_settings, AppSettings, SettingsPatch, DEFAULT_SPEAKER};

    #[test]
    fn patch_trims_custom_speaker_and_keeps_catalog_voice() {
        let settings = AppSettings::default()
            .apply_patch(SettingsPatch {
                custom_speaker_id: Some(" custom-speaker ".to_owned()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(settings.active_speaker(), "custom-speaker");
    }

    #[test]
    fn empty_custom_speaker_clears_the_override() {
        let settings = AppSettings::default()
            .apply_patch(SettingsPatch {
                custom_speaker_id: Some("custom-speaker".to_owned()),
                ..Default::default()
            })
            .unwrap()
            .apply_patch(SettingsPatch {
                custom_speaker_id: Some(String::new()),
                ..Default::default()
            })
            .unwrap();

        assert!(settings.custom_speaker_id.is_none());
        assert_eq!(settings.active_speaker(), DEFAULT_SPEAKER);
    }

    #[test]
    fn invalid_rate_is_rejected() {
        let result = AppSettings::default().apply_patch(SettingsPatch {
            speech_rate: Some(101),
            ..Default::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn legacy_seed_tts_one_speaker_moves_to_seed_tts_two() {
        let mut settings = AppSettings {
            speaker_id: "zh_male_beijingxiaoye_mars_bigtts".to_owned(),
            ..AppSettings::default()
        };

        assert!(migrate_legacy_settings(&mut settings));
        assert_eq!(settings.speaker_id, DEFAULT_SPEAKER);
    }

    #[test]
    fn favorites_are_deduplicated_and_limited_to_catalog_entries() {
        let settings = AppSettings::default()
            .apply_patch(SettingsPatch {
                favorite_speaker_ids: Some(vec![
                    DEFAULT_SPEAKER.to_owned(),
                    "missing".to_owned(),
                    DEFAULT_SPEAKER.to_owned(),
                ]),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(settings.favorite_speaker_ids, vec![DEFAULT_SPEAKER]);
    }
}

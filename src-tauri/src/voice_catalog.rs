use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub const DEFAULT_SPEAKER: &str = "zh_female_tianmeitaozi_uranus_bigtts";
pub const DEFAULT_RESOURCE_ID: &str = "seed-tts-2.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCatalog {
    pub catalog_version: String,
    pub model: String,
    pub language: String,
    pub voices: Vec<VoiceCatalogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCatalogEntry {
    pub speaker_id: String,
    pub name: String,
    pub gender: String,
    pub category: String,
    pub tags: Vec<String>,
    pub model: String,
    pub resource_id: String,
    pub recommended: bool,
}

pub fn catalog() -> &'static VoiceCatalog {
    static CATALOG: OnceLock<VoiceCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/voices.seed-tts-2.0.zh.json"))
            .expect("the bundled Seed TTS 2.0 Chinese voice catalog is valid")
    })
}

pub fn find(speaker_id: &str) -> Option<&'static VoiceCatalogEntry> {
    catalog()
        .voices
        .iter()
        .find(|voice| voice.speaker_id == speaker_id)
}

pub fn resource_id_for(speaker_id: &str) -> &'static str {
    find(speaker_id)
        .map(|voice| voice.resource_id.as_str())
        .unwrap_or(DEFAULT_RESOURCE_ID)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{catalog, find, DEFAULT_RESOURCE_ID, DEFAULT_SPEAKER};

    #[test]
    fn bundled_catalog_contains_a_broad_chinese_seed_tts_two_set() {
        let catalog = catalog();
        assert_eq!(catalog.model, DEFAULT_RESOURCE_ID);
        assert_eq!(catalog.language, "zh-CN");
        assert!(catalog.voices.len() >= 90);
        assert!(catalog
            .voices
            .iter()
            .all(|voice| voice.model == DEFAULT_RESOURCE_ID));
        assert!(catalog
            .voices
            .iter()
            .all(|voice| voice.resource_id == DEFAULT_RESOURCE_ID));
        assert!(find(DEFAULT_SPEAKER).is_some());
    }

    #[test]
    fn catalog_speaker_ids_are_unique() {
        let ids: HashSet<_> = catalog()
            .voices
            .iter()
            .map(|voice| voice.speaker_id.as_str())
            .collect();
        assert_eq!(ids.len(), catalog().voices.len());
    }
}

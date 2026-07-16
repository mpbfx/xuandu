use std::hash::{Hash, Hasher};

pub const MIN_SELECTION_CHARS: usize = 2;
pub const MAX_SELECTION_CHARS: usize = 5_000;

#[derive(Debug, PartialEq, Eq)]
pub enum PreparedSelection {
    Ignore,
    TooLong,
    Speak(String),
}

pub fn prepare_selection(raw: &str) -> PreparedSelection {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = normalized.chars().count();

    if length < MIN_SELECTION_CHARS {
        return PreparedSelection::Ignore;
    }
    if length > MAX_SELECTION_CHARS {
        return PreparedSelection::TooLong;
    }
    PreparedSelection::Speak(normalized)
}

pub fn selection_hash(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(target_os = "macos")]
#[path = "selection/macos.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "selection/windows.rs"]
mod platform;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[path = "selection/unsupported.rs"]
mod platform;

pub use platform::{
    is_accessibility_trusted, request_accessibility, selected_text, start_mouse_release_listener,
};

#[cfg(test)]
mod tests {
    use super::{prepare_selection, selection_hash, PreparedSelection};

    #[test]
    fn normalizes_whitespace_without_storing_a_history() {
        assert_eq!(
            prepare_selection("  你好\n\n世界  "),
            PreparedSelection::Speak("你好 世界".to_owned())
        );
    }

    #[test]
    fn ignores_one_character_and_rejects_overlong_selection() {
        assert_eq!(prepare_selection("你"), PreparedSelection::Ignore);
        assert_eq!(
            prepare_selection(&"字".repeat(5_001)),
            PreparedSelection::TooLong
        );
    }

    #[test]
    fn hashes_equal_selection_consistently() {
        assert_eq!(selection_hash("相同文本"), selection_hash("相同文本"));
    }
}

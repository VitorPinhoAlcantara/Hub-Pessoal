use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use iced::Font;
use iced::font::Weight;

pub const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
pub const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");

pub const UI: Font = Font::with_name("Inter");

pub const CHOICES: [&str; 10] = [
    "Inter",
    "Segoe UI",
    "Segoe UI Variable Text",
    "Arial",
    "Calibri",
    "Verdana",
    "Tahoma",
    "Trebuchet MS",
    "Georgia",
    "Consolas",
];

static SELECTED: OnceLock<Font> = OnceLock::new();

pub fn select(family: &str) -> Font {
    let family = family.trim();
    let font = if family.is_empty() || family == "Inter" {
        UI
    } else {
        Font::with_name(Box::leak(family.to_string().into_boxed_str()))
    };
    *SELECTED.get_or_init(|| font)
}

static INTERNED: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);

pub fn intern(name: &str) -> &'static str {
    let mut guard = INTERNED.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(value) = map.get(name) {
        return value;
    }
    let value: &'static str = Box::leak(name.to_string().into_boxed_str());
    map.insert(name.to_string(), value);
    value
}

pub fn family(name: &str) -> Font {
    let name = name.trim();
    if name.is_empty() || name == "Inter" {
        UI
    } else {
        Font::with_name(intern(name))
    }
}

pub fn installed_families() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let families: BTreeSet<String> = db
        .faces()
        .filter_map(|face| {
            face.families
                .first()
                .map(|(name, _)| name.trim().to_string())
        })
        .filter(|name| !name.is_empty() && !name.starts_with('.') && !name.starts_with('@'))
        .collect();
    families.into_iter().collect()
}

pub fn ui() -> Font {
    SELECTED.get().copied().unwrap_or(UI)
}

pub fn ui_medium() -> Font {
    Font {
        weight: Weight::Medium,
        ..ui()
    }
}

pub const ALL: [&[u8]; 2] = [INTER_REGULAR, INTER_MEDIUM];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fonts_are_truetype() {
        for bytes in ALL {
            assert_eq!(&bytes[..4], &[0, 1, 0, 0]);
            assert!(bytes.len() > 100_000);
        }
    }
}

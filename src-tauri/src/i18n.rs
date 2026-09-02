//! Translation for the native side.
//!
//! Only the tray menu and its tooltip live here — everything else the user
//! reads is drawn by the settings window, which has its own table in
//! `src/lib/i18n.ts`. Two tables is the honest arrangement: these strings go
//! into Win32 menus at startup and never re-render, and sharing one file across
//! the process boundary would cost a build step to save eight lines.

use crate::config::Language;

use windows::Win32::Globalization::GetUserDefaultUILanguage;

/// The language actually in force, with `Auto` resolved against Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    En,
    Ru,
}

impl Resolved {
    pub fn of(pref: Language) -> Self {
        match pref {
            Language::En => Self::En,
            Language::Ru => Self::Ru,
            Language::Auto => system(),
        }
    }
}

/// Windows' UI language, reduced to the two we speak.
///
/// The low 10 bits of a LANGID are the primary language; `0x19` is Russian.
/// Sublanguage (ru-RU vs ru-KZ) makes no difference to any string here.
fn system() -> Resolved {
    const LANG_RUSSIAN: u16 = 0x19;
    let langid = unsafe { GetUserDefaultUILanguage() };
    if langid & 0x3ff == LANG_RUSSIAN { Resolved::Ru } else { Resolved::En }
}

/// One tray string.
pub fn tray(lang: Resolved, key: Key) -> &'static str {
    match (lang, key) {
        (Resolved::En, Key::Show) => "Show now",
        (Resolved::Ru, Key::Show) => "Показать",
        (Resolved::En, Key::Share) => "Copy share card",
        (Resolved::Ru, Key::Share) => "Скопировать карточку",
        (Resolved::En, Key::Settings) => "Settings…",
        (Resolved::Ru, Key::Settings) => "Настройки…",
        (Resolved::En, Key::Quit) => "Quit Lumen",
        (Resolved::Ru, Key::Quit) => "Выйти из Lumen",
        (Resolved::En, Key::TooltipAt) => "Lumen — settings:",
        (Resolved::Ru, Key::TooltipAt) => "Lumen — настройки:",
        (Resolved::En, Key::TooltipNone) => "Lumen — settings are not persisted (no writable location)",
        (Resolved::Ru, Key::TooltipNone) => "Lumen — настройки не сохраняются (нет доступной папки)",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Show,
    Share,
    Settings,
    Quit,
    TooltipAt,
    TooltipNone,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_ignore_the_system() {
        assert_eq!(Resolved::of(Language::Ru), Resolved::Ru);
        assert_eq!(Resolved::of(Language::En), Resolved::En);
    }

    #[test]
    fn every_key_has_both_languages() {
        for key in [Key::Show, Key::Share, Key::Settings, Key::Quit, Key::TooltipAt, Key::TooltipNone]
        {
            assert!(!tray(Resolved::En, key).is_empty());
            let ru = tray(Resolved::Ru, key);
            assert!(!ru.is_empty());
            // A Cyrillic string is the only evidence that a key was actually
            // translated rather than copied across.
            assert!(
                ru.chars().any(|c| ('\u{400}'..'\u{500}').contains(&c)),
                "{key:?} is not translated"
            );
        }
    }
}

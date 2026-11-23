//! User-facing text, keyed by its English form. Every label goes through [`tr`], so the app can
//! take the Craft suite's shared translations (craft-i18n) without touching the call sites; until
//! then the English key is the text.

/// The text for `english` in the user's language.
pub fn tr(english: &str) -> String {
    english.to_string()
}

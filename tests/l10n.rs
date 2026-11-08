//! Translations: every catalog string renders in every UI font, and the
//! language picker switches the UI.
//!
//! kittest panics on a missing glyph, so a Chinese string whose characters
//! the bundled Noto Sans SC subset lacks fails here; rebuild the subset
//! with `make cjk-font`.
//!
//! The language is process-wide, so every test that changes it lives in
//! this binary, and they run one at a time (`LANGUAGE`).

use egui::Key;
use egui_kittest::Harness;
use oxrok_ui::l10n::{self, Language, tr};
use oxrok_ui::ui::theme::ThemeKind;
use oxrok_ui::ui::typography::{MONO, SANS, SANS_BOLD, SANS_MEDIUM, family};
use std::sync::Mutex;

static LANGUAGE: Mutex<()> = Mutex::new(());

#[test]
fn translates_and_falls_back() {
    let _guard = LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
    l10n::set(Language::Es);
    assert_eq!(tr("Save"), "Guardar");
    assert_eq!(tr("Capture"), "Captura");
    assert_eq!(tr("this string is not translated"), "this string is not translated");
    l10n::set(Language::ZhHans);
    assert_eq!(tr("Save"), "保存");
    assert_eq!(tr("Theme"), "主题");
    l10n::set(Language::En);
    assert_eq!(tr("Save"), "Save");
}

#[test]
fn every_translation_has_glyphs() {
    let mut installed = false;
    let mut h = Harness::new_ui(|ui| {
        if !installed {
            oxrok_ui::ui::install(ui.ctx(), ThemeKind::Dark.theme());
            installed = true;
            return;
        }
        for lang in [Language::Es, Language::ZhHans] {
            let names = Language::ALL.iter().map(|l| l.name());
            for text in l10n::catalog(lang).chain(names) {
                for f in [SANS, SANS_MEDIUM, SANS_BOLD, MONO] {
                    ui.label(egui::RichText::new(text).family(family(f)));
                }
            }
        }
    });
    h.run_steps(3);
}

/// Settings → Appearance → Language switches the whole UI at once.
#[cfg(feature = "mock")]
#[test]
fn language_picker_switches_the_ui() {
    use egui_kittest::kittest::Queryable;
    use oxrok_ui::app::LaApp;
    use oxrok_ui::backend::MockBackend;

    let _guard = LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
    l10n::set(Language::En);
    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .build_eframe(|cc| LaApp::new_for_test(cc, MockBackend::seeded(1)));
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Comma);
    h.run();
    let pick = |h: &mut Harness<'_, LaApp>, name: &str| {
        // The accessible name follows the current language.
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, tr("Language")).click();
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::ListBoxOption, name).click();
        h.run_steps(3);
    };

    pick(&mut h, "简体中文");
    assert_eq!(h.state().settings().language, Language::ZhHans);
    assert_eq!(l10n::current(), Language::ZhHans);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "外观"); // Appearance
    h.snapshot("settings_language_zh");

    pick(&mut h, "Español");
    assert_eq!(l10n::current(), Language::Es);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Apariencia");
    h.snapshot("settings_language_es");

    pick(&mut h, "English");
    assert_eq!(l10n::current(), Language::En);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Appearance");
}

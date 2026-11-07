//! egui widgets and views. Depends on `la-core` and the
//! `la-backend` traits only, never on libsigrok.

pub mod clock;
pub mod colors;
pub mod gallery;
pub mod icons;
pub mod theme;
pub mod toast;
pub mod typography;
pub mod waveform;
pub mod widgets;

pub use theme::{Theme, ThemeExt, Token};
pub use typography::Text;

/// Installs fonts, text styles, spacing and `theme` into `ctx`.
pub fn install(ctx: &egui::Context, theme: Theme) {
    ctx.set_fonts(typography::font_definitions());
    theme::set_theme(ctx, theme);
    ctx.all_styles_mut(|style| {
        typography::apply_text_styles(style);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(20.0, 12.0);
    });
}

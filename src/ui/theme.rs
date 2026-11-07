//! Color tokens and egui visuals.
//!
//! Every UI color comes from a [`Token`]. Only the dark theme
//! exists for now; the light theme arrives in 0.2.0.

use egui::{Color32, Context, CornerRadius, Id, Shadow, Stroke, Ui, Visuals};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    Canvas,
    Deck,
    Card,
    Field,
    Lift,
    Ink1,
    Ink2,
    Ink3,
    Ink4,
    Ink5,
    Hair,
    Seam,
    RingFocus,
    RingHover,
    RingHot,
    Flare,
    FlareBright,
    FlareDim,
    FlareLink,
    InkOnFlare,
    Slate,
    InkOnSlate,
    Mark,
    Alert,
    Good,
    Caution,
    InkOnCaution,
    Halt,
    InkOnSwatch,
    Console,
    Ambient,
}

impl Token {
    pub const ALL: [Token; 31] = [
        Token::Canvas,
        Token::Deck,
        Token::Card,
        Token::Field,
        Token::Lift,
        Token::Ink1,
        Token::Ink2,
        Token::Ink3,
        Token::Ink4,
        Token::Ink5,
        Token::Hair,
        Token::Seam,
        Token::RingFocus,
        Token::RingHover,
        Token::RingHot,
        Token::Flare,
        Token::FlareBright,
        Token::FlareDim,
        Token::FlareLink,
        Token::InkOnFlare,
        Token::Slate,
        Token::InkOnSlate,
        Token::Mark,
        Token::Alert,
        Token::Good,
        Token::Caution,
        Token::InkOnCaution,
        Token::Halt,
        Token::InkOnSwatch,
        Token::Console,
        Token::Ambient,
    ];
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    Dark,
    Light,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 2] = [ThemeKind::Light, ThemeKind::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "Dark",
            ThemeKind::Light => "Light",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            ThemeKind::Dark => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::Dark,
        }
    }

    pub fn theme(self) -> Theme {
        match self {
            ThemeKind::Dark => Theme::dark(),
            ThemeKind::Light => Theme::light(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub kind: ThemeKind,
    pub tokens: HashMap<Token, Color32>,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Theme {
    /// Dark column.
    pub fn dark() -> Self {
        use Token::*;
        let tokens = [
            (Canvas, hex(0x101014)),
            (Deck, hex(0x17171D)),
            (Card, hex(0x1F2026)),
            (Field, hex(0x2A2B33)),
            (Lift, hex(0x383A45)),
            (Ink1, hex(0xFFFFFF)),
            (Ink2, hex(0xD8D8DE)),
            (Ink3, hex(0x8A8A96)),
            (Ink4, hex(0x5A5A66)),
            (Ink5, hex(0x44444E)),
            (Hair, hex(0x2A2B34)),
            (Seam, hex(0x4A4B56)),
            (RingFocus, hex(0xFFFFFF)),
            (RingHover, hex(0xFFFFFF)),
            (RingHot, hex(0xFFFFFF)),
            (Flare, hex(0x2E6BE6)),
            (FlareBright, hex(0x4A82EC)),
            (FlareDim, hex(0x1E4FB0)),
            (FlareLink, hex(0x7FA8F0)),
            (InkOnFlare, hex(0xFFFFFF)),
            (Slate, hex(0xD0D0D6)),
            (InkOnSlate, hex(0x101014)),
            (Mark, hex(0x27B0A8)),
            (Alert, hex(0xE8384F)),
            (Good, hex(0x2FA84F)),
            (Caution, hex(0xF2C14E)),
            (InkOnCaution, hex(0x111111)),
            (Halt, hex(0xE8552E)),
            (InkOnSwatch, hex(0x111111)),
            (Console, hex(0x101014)),
            (Ambient, hex(0x000000)),
        ]
        .into_iter()
        .collect();
        Self { kind: ThemeKind::Dark, tokens }
    }

    /// Light column.
    pub fn light() -> Self {
        use Token::*;
        let mut tokens = Self::dark().tokens;
        for (t, c) in [
            (Canvas, hex(0xF2F2F5)),
            (Deck, hex(0xFFFFFF)),
            (Card, hex(0xFFFFFF)),
            (Field, hex(0xFFFFFF)),
            (Lift, hex(0xE8E8EC)),
            (Ink1, hex(0x14141A)),
            (Ink2, hex(0x2A2A33)),
            (Ink3, hex(0x5A5A64)),
            (Ink4, hex(0x8A8A94)),
            (Ink5, hex(0xADADB4)),
            (Hair, hex(0xD0D0D6)),
            (Seam, hex(0xDCDCE2)),
            (RingFocus, hex(0x14141A)),
            (RingHover, hex(0x14141A)),
            (RingHot, hex(0x000000)),
            (FlareBright, hex(0x4A82EC)),
            (FlareLink, hex(0x1E4FB0)),
            (Slate, hex(0x2A2A33)),
            (InkOnSlate, hex(0xFFFFFF)),
            (Console, hex(0xFFFFFF)),
        ] {
            tokens.insert(t, c);
        }
        Self { kind: ThemeKind::Light, tokens }
    }

    /// Shadow alpha: 45 % dark, 7 % (box) / 9 % (shallow) light.
    fn shadow_alpha(&self, shallow: bool) -> f32 {
        match (self.kind, shallow) {
            (ThemeKind::Dark, _) => 0.45,
            (ThemeKind::Light, false) => 0.07,
            (ThemeKind::Light, true) => 0.09,
        }
    }

    pub fn color(&self, token: Token) -> Color32 {
        self.tokens[&token]
    }

    /// `box-shadow`.
    pub fn box_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 3],
            blur: 18,
            spread: 6,
            color: self.color(Token::Ambient).gamma_multiply(self.shadow_alpha(false)),
        }
    }

    /// `shallow-box-shadow`.
    pub fn shallow_shadow(&self) -> Shadow {
        Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: self.color(Token::Ambient).gamma_multiply(self.shadow_alpha(true)),
        }
    }

    /// Egui visuals derived from the tokens.
    pub fn visuals(&self) -> Visuals {
        use Token::*;
        let c = |t| self.color(t);
        let mut v = match self.kind {
            ThemeKind::Dark => Visuals::dark(),
            ThemeKind::Light => Visuals::light(),
        };
        v.panel_fill = c(Deck);
        v.window_fill = c(Card);
        v.extreme_bg_color = c(Field);
        v.faint_bg_color = c(Card);
        v.code_bg_color = c(Field);
        v.window_stroke = Stroke::new(1.0, c(Seam));
        v.selection.bg_fill = c(Mark).gamma_multiply(0.4);
        v.selection.stroke = Stroke::new(1.0, c(Ink1));
        v.hyperlink_color = c(FlareLink);
        v.warn_fg_color = c(Caution);
        v.error_fg_color = c(Alert);
        v.window_shadow = self.box_shadow();
        v.popup_shadow = self.shallow_shadow();
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(4);

        let radius = CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = c(Deck);
        w.noninteractive.weak_bg_fill = c(Deck);
        w.noninteractive.bg_stroke = Stroke::new(1.0, c(Hair));
        w.noninteractive.fg_stroke = Stroke::new(1.0, c(Ink2));
        w.inactive.bg_fill = c(Field);
        w.inactive.weak_bg_fill = c(Field);
        w.inactive.bg_stroke = Stroke::new(1.0, c(Seam));
        w.inactive.fg_stroke = Stroke::new(1.0, c(Ink2));
        w.hovered.bg_fill = c(Lift);
        w.hovered.weak_bg_fill = c(Lift);
        w.hovered.bg_stroke = Stroke::new(1.0, c(RingHover));
        w.hovered.fg_stroke = Stroke::new(1.0, c(Ink1));
        w.active.bg_fill = c(Lift);
        w.active.weak_bg_fill = c(Lift);
        w.active.bg_stroke = Stroke::new(1.0, c(RingFocus));
        w.active.fg_stroke = Stroke::new(1.0, c(Ink1));
        w.open.bg_fill = c(Card);
        w.open.weak_bg_fill = c(Card);
        w.open.bg_stroke = Stroke::new(1.0, c(Seam));
        w.open.fg_stroke = Stroke::new(1.0, c(Ink1));
        for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            s.corner_radius = radius;
            s.expansion = 0.0;
        }
        v
    }
}

fn theme_id() -> Id {
    Id::unique("crate::ui::theme")
}

/// Makes `theme` the current theme of `ctx` and applies its visuals.
pub fn set_theme(ctx: &Context, theme: Theme) {
    let (pref, egui_theme) = match theme.kind {
        ThemeKind::Dark => (egui::ThemePreference::Dark, egui::Theme::Dark),
        ThemeKind::Light => (egui::ThemePreference::Light, egui::Theme::Light),
    };
    ctx.set_theme(pref);
    ctx.set_visuals_of(egui_theme, theme.visuals());
    ctx.data_mut(|d| d.insert_temp(theme_id(), Arc::new(theme)));
}

/// Token access for custom widgets.
pub trait ThemeExt {
    fn la_theme(&self) -> Arc<Theme>;

    fn token(&self, token: Token) -> Color32 {
        self.la_theme().color(token)
    }
}

impl ThemeExt for Context {
    fn la_theme(&self) -> Arc<Theme> {
        self.data(|d| d.get_temp::<Arc<Theme>>(theme_id())).unwrap_or_else(|| Arc::new(Theme::dark()))
    }
}

impl ThemeExt for Ui {
    fn la_theme(&self) -> Arc<Theme> {
        self.ctx().la_theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_has_a_value_in_both_themes() {
        for t in [Theme::dark(), Theme::light()] {
            for token in Token::ALL {
                assert!(t.tokens.contains_key(&token), "{token:?}");
            }
            assert_eq!(t.tokens.len(), Token::ALL.len());
        }
        let l = Theme::light();
        assert_eq!(l.color(Token::Canvas), Color32::from_rgb(0xF2, 0xF2, 0xF5));
        assert_eq!(l.color(Token::Flare), Theme::dark().color(Token::Flare));
        let t = Theme::dark();
        assert_eq!(t.color(Token::Mark), Color32::from_rgb(0x27, 0xB0, 0xA8));
    }

    #[test]
    fn visuals_follow_rust_030() {
        let t = Theme::dark();
        let v = t.visuals();
        assert_eq!(v.panel_fill, t.color(Token::Deck));
        assert_eq!(v.window_fill, t.color(Token::Card));
        assert_eq!(v.extreme_bg_color, t.color(Token::Field));
        assert_eq!(v.widgets.inactive.bg_fill, t.color(Token::Field));
        assert_eq!(v.widgets.hovered.bg_stroke.color, t.color(Token::RingHover));
        assert_eq!(v.hyperlink_color, t.color(Token::FlareLink));
        assert_eq!(v.window_corner_radius, CornerRadius::same(4));
    }
}

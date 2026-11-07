//! Fonts and text styles.
//!
//! Inter (Regular/Medium/Bold) and JetBrains Mono, both SIL OFL 1.1,
//! bundled under `assets/fonts/`.

use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use std::sync::Arc;

pub const SANS: &str = "sans";
pub const SANS_MEDIUM: &str = "sans-medium";
pub const SANS_BOLD: &str = "sans-bold";
pub const MONO: &str = "mono";

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

pub fn family(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// Font definitions with the four named families. egui's built-in fonts stay
/// as fallbacks for symbols our fonts lack.
pub fn font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let fallbacks = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in [
        ("Inter-Regular", INTER_REGULAR),
        ("Inter-Medium", INTER_MEDIUM),
        ("Inter-Bold", INTER_BOLD),
        ("JetBrainsMono-Regular", JETBRAINS_MONO),
    ] {
        defs.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let chain = |first: &str| std::iter::once(first.to_owned()).chain(fallbacks.iter().cloned()).collect::<Vec<_>>();
    defs.families.insert(family(SANS), chain("Inter-Regular"));
    defs.families.insert(family(SANS_MEDIUM), chain("Inter-Medium"));
    defs.families.insert(family(SANS_BOLD), chain("Inter-Bold"));
    defs.families.insert(family(MONO), chain("JetBrainsMono-Regular"));
    defs.families.insert(FontFamily::Proportional, chain("Inter-Regular"));
    defs.families.insert(FontFamily::Monospace, chain("JetBrainsMono-Regular"));
    defs
}

/// Text style tokens.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Text {
    Display,
    Headline,
    Subhead,
    Lead,
    LeadPara,
    Prose,
    ProsePara,
    CaptionBold,
    CaptionMed,
    Caption,
    Micro,
    ValueXL,
    ValueL,
    Value,
    ValueS,
}

impl Text {
    pub const ALL: [Text; 15] = [
        Text::Display,
        Text::Headline,
        Text::Subhead,
        Text::Lead,
        Text::LeadPara,
        Text::Prose,
        Text::ProsePara,
        Text::CaptionBold,
        Text::CaptionMed,
        Text::Caption,
        Text::Micro,
        Text::ValueXL,
        Text::ValueL,
        Text::Value,
        Text::ValueS,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Text::Display => "display",
            Text::Headline => "headline",
            Text::Subhead => "subhead",
            Text::Lead => "lead",
            Text::LeadPara => "lead_para",
            Text::Prose => "prose",
            Text::ProsePara => "prose_para",
            Text::CaptionBold => "caption_bold",
            Text::CaptionMed => "caption_med",
            Text::Caption => "caption",
            Text::Micro => "micro",
            Text::ValueXL => "value_xl",
            Text::ValueL => "value_l",
            Text::Value => "value",
            Text::ValueS => "value_s",
        }
    }

    /// Size and family per (line height is handled by layout).
    pub fn font_id(self) -> FontId {
        let (size, fam) = match self {
            Text::Display => (21.0, SANS_MEDIUM),
            Text::Headline => (16.0, SANS_MEDIUM),
            Text::Subhead => (13.0, SANS_BOLD),
            Text::Lead | Text::LeadPara => (13.0, SANS_MEDIUM),
            Text::Prose | Text::ProsePara => (13.0, SANS),
            Text::CaptionBold => (11.0, SANS_BOLD),
            Text::CaptionMed => (11.0, SANS_MEDIUM),
            Text::Caption => (11.0, SANS),
            Text::Micro => (8.0, SANS),
            Text::ValueXL => (21.0, SANS),
            Text::ValueL => (16.0, SANS_MEDIUM),
            Text::Value => (11.0, SANS),
            Text::ValueS => (10.0, SANS_BOLD),
        };
        FontId::new(size, family(fam))
    }

    pub fn style(self) -> TextStyle {
        TextStyle::Name(self.name().into())
    }
}

/// Registers the text styles and maps egui's built-in styles onto them.
pub fn apply_text_styles(style: &mut egui::Style) {
    for t in Text::ALL {
        style.text_styles.insert(t.style(), t.font_id());
    }
    style.text_styles.insert(TextStyle::Body, Text::Prose.font_id());
    style.text_styles.insert(TextStyle::Button, Text::Lead.font_id());
    style.text_styles.insert(TextStyle::Heading, Text::Display.font_id());
    style.text_styles.insert(TextStyle::Small, Text::Caption.font_id());
    style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, family(MONO)));
}

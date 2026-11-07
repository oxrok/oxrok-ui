//! A page showing every widget, used for development and snapshot tests.

use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use crate::ui::widgets::{BinarySwitch, Button, Checkbox, Dropdown};
use egui::{RichText, Ui};

#[derive(Clone, Debug, Default)]
pub struct GalleryState {
    pub primary_clicks: u32,
    pub tertiary_selected: bool,
    pub checked: bool,
    pub switch_on: bool,
    pub choice: usize,
}

pub const GALLERY_OPTIONS: [&str; 4] = ["1 MS/s", "8 MS/s", "24 MS/s", "100 MS/s"];

pub fn gallery(ui: &mut Ui, state: &mut GalleryState) {
    egui::Frame::new().fill(ui.token(Token::Canvas)).inner_margin(16).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        gallery_contents(ui, state);
    });
}

fn gallery_contents(ui: &mut Ui, state: &mut GalleryState) {
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 20.0);
    let heading = |ui: &mut Ui, s: &str| {
        ui.label(RichText::new(s).text_style(Text::Headline.style()).color(ui.token(Token::Ink1)));
    };
    heading(ui, "Buttons");
    ui.horizontal(|ui| {
        if ui.add(Button::primary("Save")).clicked() {
            state.primary_clicks += 1;
        }
        ui.add(Button::secondary("Cancel"));
        ui.add_enabled(false, Button::primary("Disabled"));
        if ui.add(Button::tertiary("All").selected(state.tertiary_selected)).clicked() {
            state.tertiary_selected = !state.tertiary_selected;
        }
        ui.add(Button::tertiary("Clear"));
    });
    heading(ui, "Toggles");
    ui.horizontal(|ui| {
        ui.add(Checkbox::new(&mut state.checked, "Glitch filter"));
        ui.add(BinarySwitch::new(&mut state.switch_on, "Trim after capture"));
        let mut off = false;
        ui.add_enabled(false, Checkbox::new(&mut off, "Unavailable"));
    });
    heading(ui, "Dropdown");
    ui.scope(|ui| {
        ui.set_max_width(260.0);
        ui.add(Dropdown::new("Sample rate", &mut state.choice, &GALLERY_OPTIONS));
    });
}

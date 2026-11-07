//! Channel colors: resistor color order by index modulo 8.

use crate::ui::theme::{Theme, ThemeKind};
use egui::Color32;

const DARK: [u32; 8] = [0xA8B8C8, 0x4FC4B0, 0xE87A6A, 0xE0B84A, 0x5AA8E8, 0xA8C84A, 0xC85AA8, 0x7A88C8];

/// Color of device channel `index` (digital and analog channel N share it).
/// The light theme changes only index 0 (neutral).
pub fn channel_color(theme: &Theme, index: u16) -> Color32 {
    let rgb = match (theme.kind, index % 8) {
        (ThemeKind::Light, 0) => 0x2A2A33,
        _ => DARK[index as usize % 8],
    };
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_repeat_every_8() {
        let t = Theme::dark();
        assert_eq!(channel_color(&t, 2), Color32::from_rgb(0xE8, 0x7A, 0x6A));
        assert_eq!(channel_color(&t, 10), channel_color(&t, 2));
    }
}

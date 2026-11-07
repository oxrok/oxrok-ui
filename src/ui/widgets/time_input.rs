//! Time input: parses times, shows the formatted value
//! when not focused, commits on Enter or blur, flashes the error style for
//! 600 ms and reverts on invalid input.

use crate::core::format::{TimeUnit, parse_time, si};
use crate::ui::theme::{ThemeExt, Token};
use crate::ui::typography::Text;
use egui::{Id, Key, Stroke, TextEdit, Ui};
use std::time::Duration;

const ERROR_FLASH: Duration = Duration::from_millis(600);

#[derive(Clone, Default)]
struct EditState {
    text: String,
    editing: bool,
    error_until: Option<Duration>,
}

/// Returns `true` when `value` (seconds) changed. `label` is the accessible name.
pub fn time_input(ui: &mut Ui, label: &str, value: &mut f64, min: f64) -> bool {
    let parse = |t: &str| parse_time(t, TimeUnit::S).ok().filter(|t| t.is_finite() && *t > 0.0).map(|t| t.max(min));
    parsed_input(ui, label, value, &|v| si(v, "s", None), &parse)
}

/// A number with an optional unit and SI prefix ("1.5 V", "500 mV"),
/// clamped to `[min, max]` and snapped to `step` when given.
pub fn number_input(ui: &mut Ui, label: &str, value: &mut f64, unit: &str, range: Option<(f64, f64, f64)>) -> bool {
    let decimals = range.map_or(6, |(_, _, step)| decimals_for(step));
    let format = |v: f64| {
        let s = format!("{v:.decimals$}");
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
        if unit.is_empty() { s } else { format!("{s} {unit}") }
    };
    let parse = |t: &str| {
        let v = parse_si(t, unit)?;
        Some(match range {
            Some((min, max, step)) => {
                let v = v.clamp(min, max);
                if step > 0.0 { (min + ((v - min) / step).round() * step).clamp(min, max) } else { v }
            }
            None => v,
        })
    };
    parsed_input(ui, label, value, &format, &parse)
}

/// Decimals that show a value on a grid of `step`.
fn decimals_for(step: f64) -> usize {
    (0..9).find(|&d| (step * 10f64.powi(d as i32)).fract().abs() < 1e-6).unwrap_or(9)
}

/// Text committed on Enter or blur; `true` when it changed.
pub fn text_input(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let id = Id::unique(("crate::ui::text_input", label));
    let mut st: EditState = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    if !st.editing {
        st.text = value.clone();
    }
    let resp = edit_box(ui, id, &mut st.text, false);
    resp.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(true, st.text.clone(), st.text.clone(), "");
        info.label = Some(label.to_owned());
        info
    });
    let mut changed = false;
    if resp.gained_focus() {
        st.editing = true;
    }
    if resp.lost_focus() || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
        st.editing = false;
        changed = st.text != *value;
        *value = st.text.clone();
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, st));
    changed
}

fn edit_box(ui: &mut Ui, id: Id, text: &mut String, error: bool) -> egui::Response {
    let theme = ui.la_theme();
    ui.scope(|ui| {
        if error {
            let v = ui.visuals_mut();
            v.extreme_bg_color = theme.color(Token::Alert).gamma_multiply(0.2);
            v.widgets.inactive.bg_stroke = Stroke::new(1.0, theme.color(Token::Alert));
        }
        ui.add(
            TextEdit::singleline(text)
                .id(id.with("edit"))
                .align(egui::Align2::CENTER_CENTER)
                .font(Text::Prose.font_id())
                .desired_width(ui.available_width())
                .min_size(egui::vec2(0.0, 36.0)),
        )
    })
    .inner
}

/// Behavior for any parsed value: shows `format(value)` when not
/// focused, commits on Enter or blur, flashes and reverts on bad input.
pub fn parsed_input(
    ui: &mut Ui,
    label: &str,
    value: &mut f64,
    format: &dyn Fn(f64) -> String,
    parse: &dyn Fn(&str) -> Option<f64>,
) -> bool {
    let now = crate::ui::clock::now(ui.ctx());
    let id = Id::unique(("crate::ui::time_input", label));
    let mut st: EditState = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    if !st.editing {
        st.text = format(*value);
    }
    let error = st.error_until.is_some_and(|t| t > now);
    if error {
        ui.ctx().request_repaint_after(st.error_until.unwrap() - now);
    }
    let resp = edit_box(ui, id, &mut st.text, error);
    resp.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(true, st.text.clone(), st.text.clone(), "");
        info.label = Some(label.to_owned());
        info
    });
    let mut changed = false;
    if resp.gained_focus() {
        st.editing = true;
    }
    if resp.lost_focus() || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
        st.editing = false;
        match parse(&st.text) {
            Some(v) if v.is_finite() => {
                changed = v != *value;
                *value = v;
            }
            _ => st.error_until = Some(now + ERROR_FLASH),
        }
        st.text = format(*value);
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, st));
    changed
}

/// "1.5", "1.5 V", "500mV", "-2 V": a number with an optional SI prefix
/// and an optional `unit`.
pub fn parse_si(input: &str, unit: &str) -> Option<f64> {
    let mut t = input.trim();
    if !unit.is_empty()
        && let Some(stripped) = t.strip_suffix(unit)
    {
        t = stripped.trim_end();
    }
    let (num, mul) = match t.chars().last()? {
        'p' => (&t[..t.len() - 1], 1e-12),
        'n' => (&t[..t.len() - 1], 1e-9),
        'u' => (&t[..t.len() - 1], 1e-6),
        'µ' => (&t[..t.len() - 'µ'.len_utf8()], 1e-6),
        'm' => (&t[..t.len() - 1], 1e-3),
        'k' => (&t[..t.len() - 1], 1e3),
        'M' => (&t[..t.len() - 1], 1e6),
        'G' => (&t[..t.len() - 1], 1e9),
        _ => (t, 1.0),
    };
    num.trim().parse::<f64>().ok().map(|v| v * mul)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_numbers() {
        assert_eq!(parse_si("1.5", "V"), Some(1.5));
        assert_eq!(parse_si("1.5 V", "V"), Some(1.5));
        assert_eq!(parse_si("500mV", "V"), Some(0.5));
        assert_eq!(parse_si("-2 V", "V"), Some(-2.0));
        assert_eq!(parse_si("abc", "V"), None);
        assert_eq!(decimals_for(0.01), 2);
        assert_eq!(decimals_for(1.0), 0);
        assert_eq!(decimals_for(0.05), 2);
    }
}

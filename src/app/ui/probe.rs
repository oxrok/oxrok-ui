//! Probe profiles in the UI: row and chip colors, grouped chip
//! grids, and the *Probe Colors* editor.

use super::dialogs::modal_frame;
use crate::app::LaApp;
use crate::app::profiles::ProfileSource;
use crate::app::session::{Row, Session};
use crate::core::ChannelKind;
use crate::core::probe::{self, Color, Group, MIN_CONTRAST, Match, ProbeChannel, ProbeProfile};
use crate::l10n::tr;
use crate::ui::Token;
use crate::ui::colors::channel_color;
use crate::ui::theme::{Theme, ThemeExt};
use crate::ui::toast::{Toast, ToastKind};
use crate::ui::typography::Text;
use crate::ui::waveform::RowLook;
use crate::ui::widgets::{Button, ChipGroup, ChipItem};
use egui::{Color32, Context, Id, Modal, Popup, Role, ScrollArea, Sense, Stroke, Ui, WidgetInfo, vec2};

fn c32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

fn low_contrast(theme: &Theme, rgb: [u8; 3]) -> bool {
    let bg = theme.color(Token::Canvas);
    probe::contrast_ratio(rgb, [bg.r(), bg.g(), bg.b()]) < MIN_CONTRAST
}

/// Resolution (profile wire, else the fallback palette) with the
/// Contrast rule.
pub(crate) fn row_look(theme: &Theme, s: &Session, row: &Row) -> RowLook {
    let probe = s.probe.as_ref().map(|(p, _)| p);
    let ch = probe.and_then(|p| p.channel(&row.ident));
    // The row's own color, then the wire, then the palette.
    let swatch = match row.color {
        Some(rgb) => c32(rgb),
        None => {
            // Groups take the palette from blue on.
            let index = if row.channel.is_bus() { row.channel.index + 6 } else { row.channel.index };
            ch.and_then(|c| c.wire.as_ref()).map_or_else(|| channel_color(theme, index), |w| c32(w.rgb))
        }
    };
    let marker = probe.zip(ch).and_then(|(p, c)| p.marker(c)).map(|m| c32(m.rgb));
    let outline = low_contrast(theme, [swatch.r(), swatch.g(), swatch.b()]);
    RowLook { swatch, ink: if outline { theme.color(Token::Ink2) } else { swatch }, marker, outline }
}

/// "brown wire, blue marker" (tooltip), when a profile describes the channel.
pub(crate) fn describe(s: &Session, row: &Row) -> Option<String> {
    let (p, _) = s.probe.as_ref()?;
    let c = p.channel(&row.ident)?;
    let wire = c.wire.as_ref().map(|w| format!("{} wire", w.spec));
    let marker = p.marker(c).map(|m| format!("{} marker", m.spec));
    let parts: Vec<String> = wire.into_iter().chain(marker).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// Short label: the index for `D<n>`/`A<n>`, else the backend name.
fn short_label(row: &Row) -> String {
    let n = row.channel.index.to_string();
    if row.ident == row.channel.short_name() { n } else { row.ident.clone() }
}

pub(crate) fn chip(theme: &Theme, s: &Session, row: &Row, enabled: bool) -> ChipItem {
    let look = row_look(theme, s, row);
    let tooltip = match describe(s, row) {
        Some(d) => format!("{} · {d}", row.display()),
        None => row.display(),
    };
    ChipItem {
        index: row.channel.index,
        short: short_label(row),
        name: tooltip,
        label: row.identifier(),
        enabled,
        toggleable: true,
        color: Some(look.swatch),
        marker: look.marker,
        outline: look.outline,
    }
}

/// Chip sections by profile group, unlisted channels last under
/// *Other*. Returns the sections and the rows in chip order.
pub(crate) fn chip_groups<'a>(theme: &Theme, s: &'a Session, kind: ChannelKind) -> (Vec<ChipGroup>, Vec<&'a Row>) {
    let rows: Vec<&Row> = s.rows.iter().filter(|r| r.channel.kind == kind).collect();
    let Some((p, _)) = s.probe.as_ref() else {
        let items = rows.iter().map(|r| chip(theme, s, r, s.settings.is_enabled(r.channel))).collect();
        return (vec![ChipGroup { name: None, marker: None, marker_outline: false, items }], rows);
    };
    let mut groups = Vec::new();
    let mut order = Vec::new();
    for (gi, g) in p.groups.iter().enumerate() {
        let members: Vec<&Row> = p
            .channels
            .iter()
            .filter(|c| c.group == gi)
            .filter_map(|c| rows.iter().copied().find(|r| r.ident == c.name))
            .collect();
        if members.is_empty() {
            continue;
        }
        let marker = g.marker.as_ref().map(|m| (c32(m.rgb), low_contrast(theme, m.rgb)));
        groups.push(ChipGroup {
            name: Some(g.name.clone()),
            marker: marker.map(|m| m.0),
            marker_outline: marker.is_some_and(|m| m.1),
            items: members.iter().map(|r| chip(theme, s, r, s.settings.is_enabled(r.channel))).collect(),
        });
        order.extend(members);
    }
    let others: Vec<&Row> = rows.iter().copied().filter(|r| p.channel(&r.ident).is_none()).collect();
    if !others.is_empty() {
        groups.push(ChipGroup {
            name: Some("Other".into()),
            marker: None,
            marker_outline: false,
            items: others.iter().map(|r| chip(theme, s, r, s.settings.is_enabled(r.channel))).collect(),
        });
        order.extend(others);
    }
    (groups, order)
}

// ---- editor ----

pub(crate) struct ProbeEditor {
    pub draft: ProbeProfile,
    pub source: Option<ProfileSource>,
    /// The color being picked, and the hex field's text.
    pub picking: Option<Target>,
    pub hex: String,
}

/// What the inline color picker edits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Wire(usize),
    Marker(usize),
    GroupMarker(usize),
}

impl Target {
    fn get(self, p: &ProbeProfile) -> Option<Color> {
        match self {
            Target::Wire(c) => p.channels.get(c).and_then(|c| c.wire.clone()),
            Target::Marker(c) => p.channels.get(c).and_then(|c| c.marker.clone()),
            Target::GroupMarker(g) => p.groups.get(g).and_then(|g| g.marker.clone()),
        }
    }

    fn set(self, p: &mut ProbeProfile, v: Option<Color>) {
        match self {
            Target::Wire(c) => p.channels[c].wire = v,
            Target::Marker(c) => p.channels[c].marker = v,
            Target::GroupMarker(g) => p.groups[g].marker = v,
        }
    }

    fn describe(self, p: &ProbeProfile) -> String {
        match self {
            Target::Wire(c) => tr("{} wire").replacen("{}", &p.channels[c].name, 1),
            Target::Marker(c) => tr("{} marker").replacen("{}", &p.channels[c].name, 1),
            Target::GroupMarker(g) => tr("{} marker").replacen("{}", &p.groups[g].name, 1),
        }
    }

    fn none_label(self) -> &'static str {
        match self {
            Target::Wire(_) => tr("Default"),
            Target::Marker(_) => tr("Group default"),
            Target::GroupMarker(_) => tr("No marker"),
        }
    }
}

impl LaApp {
    /// Opens the editor on the session's profile, or a seeded new one
    /// (*Create profile*). Every device channel is listed: channels the
    /// profile doesn't cover go to an *Other* group.
    pub(crate) fn open_probe_editor(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let d = &s.device;
        let (mut draft, source) = match &s.probe {
            Some((p, src)) => (p.clone(), Some(*src)),
            None => {
                let m = Match {
                    driver: d.driver.clone(),
                    model: Some(d.model.clone()).filter(|m| !m.is_empty()),
                    ..Match::default()
                };
                let chans: Vec<(String, u16)> = s.rows.iter().map(|r| (r.ident.clone(), r.channel.index)).collect();
                (ProbeProfile::seed(&d.display_name(), m, &chans), None)
            }
        };
        let missing: Vec<&Row> = s.rows.iter().filter(|r| draft.channel(&r.ident).is_none()).collect();
        if !missing.is_empty() {
            let gi = draft.groups.len();
            draft.groups.push(Group { name: "Other".into(), marker: None });
            for r in missing {
                draft.channels.push(ProbeChannel {
                    name: r.ident.clone(),
                    group: gi,
                    wire: None,
                    marker: None,
                    label: None,
                });
            }
        }
        self.ui.probe_editor = Some(ProbeEditor { draft, source, picking: None, hex: String::new() });
    }

    pub(crate) fn probe_editor_ui(&mut self, ctx: &Context) {
        let theme = ctx.la_theme();
        let Some(ed) = self.ui.probe_editor.as_mut() else { return };
        let device = self.session.as_ref().map(|s| s.device.display_name()).unwrap_or_default();
        let mut action = None;
        let resp = Modal::new(Id::unique("probe_editor")).frame(modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(620.0);
            let c = |t| theme.color(t);
            ui.label(egui::RichText::new(tr("Probe Colors")).text_style(Text::Display.style()).color(c(Token::Ink1)));
            ui.add_space(4.0);
            let m = &ed.draft.matcher;
            let matched = [
                Some(tr("driver {}").replacen("{}", &m.driver, 1)),
                m.vendor.as_ref().map(|v| tr("vendor {}").replacen("{}", v, 1)),
                m.model.as_ref().map(|v| tr("model {}").replacen("{}", v, 1)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
            ui.label(
                egui::RichText::new(tr("{} · applies to {}").replacen("{}", &device, 1).replacen("{}", &matched, 1))
                    .color(c(Token::Ink3)),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(tr("Profile name"));
                let r = ui.add(egui::TextEdit::singleline(&mut ed.draft.name).desired_width(300.0));
                super::label_text_edit(&r, &ed.draft.name, tr("Profile name"));
            });
            ui.add_space(16.0);
            // The body scrolls so the modal fits small windows (min 500 pt)
            // and the buttons below stay visible.
            // Header, picker, footer and padding take about 440 pt.
            let max_h = (ctx.content_rect().height() - 440.0).max(120.0);
            ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                groups_ui(ui, ed, &theme);
                ui.add_space(16.0);
                channels_ui(ui, ed, &theme);
            });
            // Always present so the modal doesn't move when a color is selected.
            ui.add_space(12.0);
            picker_ui(ui, ed, &theme);
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                // *Reset to built-in* for a user copy, *Delete* for a user-only profile.
                let delete = match ed.source {
                    Some(ProfileSource::UserOverride) => Some(tr("Reset to built-in")),
                    Some(ProfileSource::User) => Some(tr("Delete profile")),
                    _ => None,
                };
                if let Some(label) = delete
                    && ui.add(Button::secondary(label)).clicked()
                {
                    action = Some(EditorAction::DeleteUser);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Button::primary(tr("Save"))).clicked() {
                        action = Some(EditorAction::Save);
                    }
                    if ui.add(Button::secondary(tr("Cancel"))).clicked() {
                        action = Some(EditorAction::Cancel);
                    }
                });
            });
        });
        if resp.should_close() && action.is_none() && !Popup::is_any_open(ctx) {
            action = Some(EditorAction::Cancel);
        }
        let Some(action) = action else { return };
        let ed = self.ui.probe_editor.take().unwrap();
        let result = match action {
            EditorAction::Cancel => return,
            EditorAction::Save => self.profiles.save(&ed.draft).map(|_| tr("Probe colors saved")),
            EditorAction::DeleteUser => self.profiles.delete_user(&ed.draft.matcher).map(|_| match ed.source {
                Some(ProfileSource::UserOverride) => tr("Probe colors reset to the built-in profile"),
                _ => tr("Probe profile deleted"),
            }),
        };
        match result {
            Ok(msg) => {
                // Saved changes apply right away.
                if let Some(s) = self.session.as_mut() {
                    let p = self.profiles.select(&s.device);
                    s.set_probe(p);
                }
                self.toast(Toast::new(ToastKind::Success, msg));
            }
            Err(e) => {
                self.toast(Toast::new(ToastKind::Error, e.to_string()).heading(tr("Could not save the probe profile")));
                self.ui.probe_editor = Some(ed);
            }
        }
    }
}

enum EditorAction {
    Save,
    DeleteUser,
    Cancel,
}

fn section(ui: &mut Ui, text: &str, theme: &Theme) {
    ui.label(egui::RichText::new(text).text_style(Text::Subhead.style()).color(theme.color(Token::Ink2)));
    ui.add_space(6.0);
}

fn groups_ui(ui: &mut Ui, ed: &mut ProbeEditor, theme: &Theme) {
    section(ui, tr("Groups"), theme);
    let mut op: Option<(usize, i32)> = None; // (group, -1 up / 1 down / 0 delete)
    let p = &mut ed.draft;
    let n = p.groups.len();
    for gi in 0..n {
        ui.horizontal(|ui| {
            let t = Target::GroupMarker(gi);
            if color_button(
                ui,
                &t.describe(p),
                p.groups[gi].marker.as_ref(),
                t.none_label(),
                ed.picking == Some(t),
                theme,
            ) {
                ed.picking = Some(t);
                ed.hex = p.groups[gi].marker.as_ref().map(Color::hex).unwrap_or_default();
            }
            let r = ui.add(egui::TextEdit::singleline(&mut p.groups[gi].name).desired_width(200.0));
            super::label_text_edit(
                &r,
                &p.groups[gi].name,
                &tr("Group {} name").replacen("{}", &(gi + 1).to_string(), 1),
            );
            let count = p.channels.iter().filter(|c| c.group == gi).count();
            ui.label(
                egui::RichText::new(tr("{} channels").replacen("{}", &count.to_string(), 1))
                    .color(theme.color(Token::Ink3)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if n > 1
                    && ui.add(Button::tertiary(tr("Delete"))).on_hover_text(tr("Its channels move to Other")).clicked()
                {
                    op = Some((gi, 0));
                }
                if gi + 1 < n && ui.add(Button::tertiary("↓")).clicked() {
                    op = Some((gi, 1));
                }
                if gi > 0 && ui.add(Button::tertiary("↑")).clicked() {
                    op = Some((gi, -1));
                }
            });
        });
    }
    if ui.add(Button::tertiary(tr("+ Add group"))).clicked() {
        p.groups.push(Group { name: format!("Group {}", p.groups.len() + 1), marker: None });
    }
    if op.is_some() {
        // Indices change: close the picker.
        ed.picking = None;
    }
    match op {
        Some((gi, 0)) => delete_group(&mut ed.draft, gi),
        Some((gi, d)) => swap_groups(&mut ed.draft, gi, (gi as i32 + d) as usize),
        None => {}
    }
}

fn swap_groups(p: &mut ProbeProfile, a: usize, b: usize) {
    p.groups.swap(a, b);
    for c in &mut p.channels {
        if c.group == a {
            c.group = b;
        } else if c.group == b {
            c.group = a;
        }
    }
    sort_channels(p);
}

fn delete_group(p: &mut ProbeProfile, gi: usize) {
    p.groups.remove(gi);
    let other = match p.groups.iter().position(|g| g.name == "Other") {
        Some(o) => o,
        None => {
            p.groups.push(Group { name: "Other".into(), marker: None });
            p.groups.len() - 1
        }
    };
    for c in &mut p.channels {
        if c.group == gi {
            c.group = other;
        } else if c.group > gi {
            c.group -= 1;
        }
    }
    sort_channels(p);
}

/// Keeps channels in group order (stable within a group).
fn sort_channels(p: &mut ProbeProfile) {
    p.channels.sort_by_key(|c| c.group);
}

fn channels_ui(ui: &mut Ui, ed: &mut ProbeEditor, theme: &Theme) {
    section(ui, tr("Channels"), theme);
    let p = &mut ed.draft;
    let group_names: Vec<String> = p.groups.iter().map(|g| g.name.clone()).collect();
    let mut moved = false;
    ui.scope(|ui| {
        egui::Grid::new("probe_channels").num_columns(5).spacing([12.0, 6.0]).show(ui, |ui| {
            for h in [tr("Channel"), tr("Group"), tr("Wire"), tr("Marker"), tr("Label")] {
                ui.label(egui::RichText::new(h).color(theme.color(Token::Ink3)));
            }
            ui.end_row();
            for ci in 0..p.channels.len() {
                let name = p.channels[ci].name.clone();
                ui.label(egui::RichText::new(&name).text_style(Text::Value.style()));
                let mut g = p.channels[ci].group;
                let combo = egui::ComboBox::from_id_salt(("probe_group", ci))
                    .width(120.0)
                    .selected_text(group_names.get(g).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (gi, gname) in group_names.iter().enumerate() {
                            ui.selectable_value(&mut g, gi, gname);
                        }
                    });
                let group_label = tr("{} group").replacen("{}", &name, 1);
                combo.response.widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, &group_label));
                if g != p.channels[ci].group {
                    p.channels[ci].group = g;
                    moved = true;
                }
                for t in [Target::Wire(ci), Target::Marker(ci)] {
                    // A marker shows the group's when the channel has none.
                    let shown = t.get(p).or_else(|| match t {
                        Target::Marker(_) => p.marker(&p.channels[ci]).cloned(),
                        _ => None,
                    });
                    if color_button(ui, &t.describe(p), shown.as_ref(), t.none_label(), ed.picking == Some(t), theme) {
                        ed.picking = Some(t);
                        ed.hex = t.get(p).as_ref().map(Color::hex).unwrap_or_default();
                    }
                }
                let mut label = p.channels[ci].label.clone().unwrap_or_default();
                let r = ui.add(egui::TextEdit::singleline(&mut label).desired_width(110.0).hint_text(&name));
                super::label_text_edit(&r, &label, &tr("{} label").replacen("{}", &name, 1));
                if r.changed() {
                    p.channels[ci].label = Some(label).filter(|l| !l.trim().is_empty());
                }
                ui.end_row();
            }
        });
    });
    if moved {
        ed.picking = None;
        sort_channels(&mut ed.draft);
    }
}

/// A swatch button naming its color; `true` when clicked (selects it for
/// the inline picker).
fn color_button(
    ui: &mut Ui,
    label: &str,
    value: Option<&Color>,
    none_label: &str,
    selected: bool,
    theme: &Theme,
) -> bool {
    let text = value.map_or(none_label.to_owned(), |c| c.spec.clone());
    let (rect, resp) = ui.allocate_exact_size(vec2(110.0, 24.0), Sense::click());
    let resp = resp.on_hover_text(label);
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, label));
    let painter = ui.painter();
    let border =
        if selected { Stroke::new(2.0, theme.color(Token::Mark)) } else { Stroke::new(1.0, theme.color(Token::Seam)) };
    painter.rect_stroke(rect, 3.0, border, egui::StrokeKind::Inside);
    let sw = egui::Rect::from_min_size(rect.min + vec2(5.0, 5.0), vec2(14.0, 14.0));
    match value {
        Some(c) => {
            painter.rect_filled(sw, 2.0, c32(c.rgb));
            painter.rect_stroke(sw, 2.0, Stroke::new(1.0, theme.color(Token::Seam)), egui::StrokeKind::Inside);
        }
        None => {
            painter.line_segment([sw.left_bottom(), sw.right_top()], Stroke::new(1.0, theme.color(Token::Ink4)));
        }
    }
    painter.text(
        sw.right_center() + vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &text,
        Text::Caption.font_id(),
        theme.color(Token::Ink2),
    );
    resp.clicked()
}

/// The inline picker for the selected color: named swatches, the profile's
/// colors, a hex field and "none".
fn picker_ui(ui: &mut Ui, ed: &mut ProbeEditor, theme: &Theme) {
    const HEIGHT: f32 = 96.0;
    let p = &mut ed.draft;
    egui::Frame::new().stroke(Stroke::new(1.0, theme.color(Token::Seam))).corner_radius(4).inner_margin(10).show(
        ui,
        |ui| {
            ui.set_min_height(HEIGHT);
            let Some(t) = ed.picking else {
                ui.label(
                    egui::RichText::new(tr("Select a wire or marker above to change its color."))
                        .color(theme.color(Token::Ink3)),
                );
                return;
            };
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr("Color for {}").replacen("{}", &t.describe(p), 1))
                        .text_style(Text::Lead.style()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Button::tertiary(tr("Done"))).clicked() {
                        ed.picking = None;
                    }
                });
            });
            ui.add_space(6.0);
            let current = t.get(p);
            let names: Vec<String> = crate::core::probe::NAMED_COLORS
                .iter()
                .map(|(n, _)| n.to_string())
                .filter(|n| n != "grey")
                .chain(p.colors.keys().cloned())
                .collect();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for n in names {
                    let Ok(color) = p.color(&n) else { continue };
                    let (r, sresp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                    let sresp = sresp.on_hover_text(&n);
                    sresp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &n));
                    ui.painter().rect_filled(r, 3.0, c32(color.rgb));
                    let on = current.as_ref().is_some_and(|c| c.spec == n);
                    let stroke = if on {
                        Stroke::new(2.0, theme.color(Token::Mark))
                    } else {
                        Stroke::new(1.0, theme.color(Token::Seam))
                    };
                    ui.painter().rect_stroke(r, 3.0, stroke, egui::StrokeKind::Inside);
                    if sresp.clicked() {
                        ed.hex = color.hex();
                        t.set(p, Some(color));
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut ed.hex).hint_text("#RRGGBB").desired_width(90.0));
                super::label_text_edit(&r, &ed.hex, tr("Hex color"));
                let valid = p.color(ed.hex.trim()).ok().filter(|_| ed.hex.trim().starts_with('#'));
                if ui.add_enabled(valid.is_some(), Button::tertiary(tr("Apply"))).clicked()
                    || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid.is_some())
                {
                    t.set(p, valid);
                }
                if ui.add(Button::tertiary(t.none_label())).clicked() {
                    t.set(p, None);
                }
            });
        },
    );
}

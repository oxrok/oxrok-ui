//! The trigger: an icon per stage at the right of each digital row's
//! label, the stage letters above them, and the Inspector's Trigger tab.

use super::graph::{Layout, mode_name};
use super::hint;
use super::inspector::divider;
use crate::app::LaApp;
use crate::app::trigger::{self, TriggerError};
use crate::backend::HwTriggerCaps;
use crate::core::capture::CaptureMode;
use crate::core::trigger::{Combine, DigitalTrigger, Hold, Match, Stage, StageCount, ValueRange};
use crate::core::{ChannelId, ChannelKind};
use crate::l10n::tr;
use crate::ui::theme::ThemeExt;
use crate::ui::typography::Text;
use crate::ui::widgets::{Button, Checkbox, Dropdown, number_input, time_input, whole_input};
use crate::ui::{Token, icons};
use egui::{Align2, Color32, Painter, Rect, Role, Sense, Ui, WidgetInfo, pos2, vec2};

pub(crate) const TRIGGER_PANEL: &str = "trigger";
/// One trigger icon's cell.
pub(crate) const CELL: f32 = 18.0;
/// Between the icons and the plot.
const PAD: f32 = 4.0;
const STAGE_LETTERS: [&str; 2] = ["A", "B"];

/// What the rows show of the trigger, for one frame.
pub(crate) struct TriggerCols {
    /// Icon columns: the stages.
    pub stages: usize,
    pub trigger: DigitalTrigger,
    /// Why the trigger cannot be changed now.
    pub locked: Option<&'static str>,
    /// Trigger mode is on.
    pub armed: bool,
    /// Why the trigger cannot be used as it is.
    pub error: Option<String>,
}

impl TriggerCols {
    /// The width the icons take at the right of the label column.
    pub fn width(&self) -> f32 {
        self.stages as f32 * CELL + PAD
    }

    /// The cell of `stage` on a row whose label ends at `right`, around
    /// the line `cy`.
    pub fn cell(&self, right: f32, cy: f32, stage: usize) -> Rect {
        let left = right - self.width() + stage as f32 * CELL;
        Rect::from_min_size(pos2(left, cy - CELL / 2.0), vec2(CELL, CELL))
    }

    /// What the icon of `ch` in `stage` is called.
    pub fn name(&self, stage: usize, ident: &str) -> String {
        if self.stages > 1 {
            tr("Trigger {} on {}").replacen("{}", STAGE_LETTERS[stage], 1).replacen("{}", ident, 1)
        } else {
            tr("Trigger on {}").replacen("{}", ident, 1)
        }
    }
}

/// Whether `ch` is part of `stage`'s value range.
fn in_range(stage: &Stage, ch: ChannelId) -> bool {
    stage.range.as_ref().is_some_and(|r| r.channels.contains(&ch))
}

/// The glyph of `ch` in `stage`, and what it stands for.
fn glyph(stage: &Stage, ch: ChannelId) -> (Icon, String) {
    let pulse = stage.pulse_edge().filter(|(c, _)| *c == ch);
    match (stage.get(ch), pulse) {
        (_, Some((_, Match::Falling))) => (icons::trig_pulse_high, tr("High pulse").to_owned()),
        (_, Some((_, Match::Rising))) => (icons::trig_pulse_low, tr("Low pulse").to_owned()),
        (Some(Match::Rising), _) => (icons::trig_rising, Match::Rising.label().to_owned()),
        (Some(Match::Falling), _) => (icons::trig_falling, Match::Falling.label().to_owned()),
        (Some(Match::Edge), _) => (icons::trig_either, Match::Edge.label().to_owned()),
        (Some(Match::High), _) => (icons::trig_high, Match::High.label().to_owned()),
        (Some(Match::Low), _) => (icons::trig_low, Match::Low.label().to_owned()),
        (None, _) if in_range(stage, ch) => (icons::trig_range, tr("Part of the value range").to_owned()),
        (None, _) => (icons::trig_none, tr("No condition").to_owned()),
    }
}

/// An icon drawn in its bounding rectangle (see `crate::ui::icons`).
type Icon = fn(&Painter, Rect, Color32);

/// One icon: the glyph, the stage letter in the corner when there are
/// two stages, and a dot when the stage has a count, a duration or a
/// value range.
fn paint_icon(p: &Painter, cell: Rect, icon: Icon, ink: Color32, letter: Option<&str>, extras: bool) {
    let right = if letter.is_some() { 7.0 } else { 3.0 };
    let r = Rect::from_min_max(cell.min + vec2(2.0, 4.0), cell.max - vec2(right, 4.0));
    icon(p, r, ink);
    if let Some(letter) = letter {
        p.text(
            cell.right_bottom() + vec2(-0.5, -1.0),
            Align2::RIGHT_BOTTOM,
            letter,
            egui::FontId::proportional(8.0),
            ink,
        );
    }
    if extras {
        p.circle_filled(cell.right_top() + vec2(-3.0, 4.0), 1.5, ink);
    }
}

/// `stage` in words: its conditions, value range, count and duration.
pub(crate) fn stage_summary(stage: &Stage, name: &dyn Fn(ChannelId) -> String) -> String {
    let mut parts: Vec<String> = stage
        .conditions
        .iter()
        .map(|&(c, m)| {
            let what = match stage.pulse_edge() {
                Some((pc, Match::Falling)) if pc == c => tr("High pulse"),
                Some((pc, Match::Rising)) if pc == c => tr("Low pulse"),
                _ => m.label(),
            };
            format!("{} {what}", name(c))
        })
        .collect();
    if let Some(r) = &stage.range
        && let (Some(first), Some(last)) = (r.channels.first(), r.channels.last())
    {
        let values = if r.low == r.high { r.low.to_string() } else { format!("{}..{}", r.low, r.high) };
        let side = if r.inside { tr("within") } else { tr("outside") };
        parts.push(format!("{}–{} {side} {values}", name(*first), name(*last)));
    }
    let mut text = parts.join(", ");
    if let Some(c) = stage.count.filter(|c| c.n > 1 || c.consecutive) {
        text += &format!(" ×{}", c.n);
        if c.consecutive {
            text += &format!(" {}", tr("in a row"));
        }
    }
    let time = |s: f64| crate::core::format::si(s, "s", None);
    match stage.hold {
        Some(Hold::Longer(s)) => text += &format!(", > {}", time(s)),
        Some(Hold::Shorter(s)) => text += &format!(", < {}", time(s)),
        Some(Hold::Between(a, b)) => text += &format!(", {}–{}", time(a), time(b)),
        None => {}
    }
    text
}

/// A row of exclusive choices: `(text, why it cannot be picked)`.
/// Returns whether the selection changed.
fn choice(ui: &mut Ui, label: &str, options: &[(&str, Option<String>)], selected: &mut usize) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (k, (text, why)) in options.iter().enumerate() {
            let on = *selected == k;
            // What is on can always be seen and left.
            let enabled = why.is_none() || on;
            let r = ui.add_enabled(enabled, Button::tertiary(text).selected(on));
            r.widget_info(|| WidgetInfo::selected(Role::RadioButton, enabled, on, format!("{label}: {text}")));
            let r = match why {
                Some(why) => r.on_disabled_hover_text(why),
                None => r,
            };
            if r.clicked() && !on {
                *selected = k;
                changed = true;
            }
        }
    });
    changed
}

fn caption(ui: &mut Ui, text: &str) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).text_style(Text::Lead.style()).color(theme.color(Token::Ink1)));
}

fn note(ui: &mut Ui, text: &str, token: Token) {
    let theme = ui.la_theme();
    ui.label(egui::RichText::new(text).color(theme.color(token)));
}

/// Why the device at hand cannot do something, if it cannot: the
/// device's trigger lacks it (`has` is false), or there is none.
fn lacking(hw: Option<&HwTriggerCaps>, has: impl Fn(&HwTriggerCaps) -> bool, e: TriggerError) -> Option<String> {
    match hw {
        None => Some(TriggerError::NeedsDevice.text()),
        Some(c) if !has(c) => Some(e.text()),
        Some(_) => None,
    }
}

impl LaApp {
    /// The trigger columns of this frame's rows; `None` without digital
    /// rows, or on a file whose capture had no trigger.
    pub(crate) fn trigger_cols(&self) -> Option<TriggerCols> {
        let s = self.session.as_ref()?;
        if !s.visible_rows().any(|r| r.channel.kind == ChannelKind::Digital) {
            return None;
        }
        let stages_set = |t: &DigitalTrigger| t.stages.iter().rposition(|s| !s.is_empty()).map_or(0, |k| k + 1);
        if s.is_read_only() {
            let trigger = s.snapshot.as_ref()?.trigger.clone()?;
            let stages = stages_set(&trigger);
            return (stages > 0).then(|| TriggerCols {
                stages,
                trigger,
                locked: Some(tr("The trigger this capture was recorded with.")),
                armed: true,
                error: None,
            });
        }
        let hw = s.caps.as_ref().and_then(|c| c.hw_trigger.as_ref());
        let trigger = s.settings.trigger.clone();
        let armed = s.settings.mode == CaptureMode::Triggered;
        let locked = if s.caps.is_none() {
            Some(tr("Opening device…"))
        } else if s.state.is_busy() {
            Some(tr("Stop the capture to change the trigger."))
        } else {
            None
        };
        let error = (armed && s.caps.is_some())
            .then(|| crate::app::plan_trigger(&trigger, s.caps.as_ref(), 0, s.settings.sample_rate).err())
            .flatten()
            .map(|e| e.text());
        Some(TriggerCols { stages: trigger::stages(hw).max(stages_set(&trigger)), trigger, locked, armed, error })
    }

    /// The click targets and the icons of the rows' trigger cells, made
    /// after the labels' so a label's drag does not take them. Returns
    /// the cell clicked, and the stage whose details were asked for.
    pub(crate) fn trigger_cells(
        &self,
        ui: &Ui,
        painter: &Painter,
        clip: Rect,
        cols: &TriggerCols,
        cells: &[(ChannelId, String, usize, Rect)],
    ) -> (Option<(ChannelId, usize)>, Option<usize>) {
        let theme = ui.la_theme();
        let (mut cycle, mut details) = (None, None);
        for (ch, ident, k, cell) in cells {
            let stage = &cols.trigger.stages[*k];
            // A locked cell takes the click for its details: those of a
            // recorded trigger are read there.
            let resp =
                ui.interact(cell.intersect(clip), ui.make_persistent_id(("trigger_cell", *ch, *k)), Sense::click());
            let name = cols.name(*k, ident);
            let (icon, state) = glyph(stage, *ch);
            let set = stage.get(*ch).is_some() || in_range(stage, *ch);
            let enabled = cols.locked.is_none();
            resp.widget_info(|| {
                let mut info = WidgetInfo::labeled(Role::Button, enabled, &name);
                info.current_text_value = Some(state.clone());
                info
            });
            if resp.hovered() && enabled {
                painter.rect_filled(cell.shrink(1.0), 3.0, theme.color(Token::Lift));
            }
            let ink = match (set, cols.armed, &cols.error) {
                (false, ..) if resp.hovered() && enabled => theme.color(Token::Ink3),
                (false, ..) => theme.color(Token::Ink5),
                (true, true, Some(_)) => theme.color(Token::Alert),
                (true, true, None) => theme.color(Token::Ink1),
                // Set, but the capture mode is another.
                (true, false, _) => theme.color(Token::Ink4),
            };
            let letter = (cols.stages > 1 && set).then(|| STAGE_LETTERS[*k]);
            let extras = set && (stage.count.is_some() || stage.hold.is_some() || stage.range.is_some());
            paint_icon(painter, *cell, icon, ink, letter, extras);
            let hint = match (cols.locked, &cols.error) {
                (Some(why), _) => format!("{why}\n{}", tr("Right-click for details.")),
                (None, Some(e)) if set => e.clone(),
                (None, _) => tr("Click to change, right-click for details.").to_owned(),
            };
            let resp = resp.on_hover_text(format!("{name}: {state}\n{hint}"));
            if resp.clicked() && enabled {
                cycle = Some((*ch, *k));
            }
            if resp.secondary_clicked() {
                details = Some(*k);
            }
        }
        (cycle, details)
    }

    /// The stage letters above two trigger columns; each opens its
    /// stage's details.
    pub(crate) fn trigger_header(&mut self, ui: &Ui, l: &Layout, cols: &TriggerCols) {
        if cols.stages < 2 {
            return;
        }
        let theme = ui.la_theme();
        for (k, letter) in STAGE_LETTERS.iter().enumerate().take(cols.stages) {
            let cell = cols.cell(l.label_header.right(), l.label_header.bottom() - 10.0, k);
            let cell = Rect::from_center_size(cell.center(), vec2(CELL, 16.0));
            let resp = ui.interact(cell, ui.make_persistent_id(("trigger_stage", k)), Sense::click());
            let label = tr("Trigger stage {} details").replacen("{}", letter, 1);
            resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
            let set = !cols.trigger.stages[k].is_empty();
            let token = match (resp.hovered(), set && cols.armed) {
                (true, _) => Token::Ink1,
                (false, true) => Token::Ink2,
                (false, false) => Token::Ink4,
            };
            ui.painter().text(
                cell.center() - vec2(2.0, 0.0),
                Align2::CENTER_CENTER,
                *letter,
                Text::ValueS.font_id(),
                theme.color(token),
            );
            if resp.on_hover_text(label.clone()).clicked() {
                self.show_trigger_tab(k);
            }
        }
    }

    /// A click on a row's trigger icon: the next condition the device
    /// has, or none after the last. Setting one turns Trigger mode on.
    pub(crate) fn cycle_trigger(&mut self, ch: ChannelId, stage: usize) {
        let Some(s) = self.session.as_mut() else { return };
        if s.is_read_only() || s.state.is_busy() {
            return;
        }
        let allowed = trigger::matches(s.caps.as_ref().and_then(|c| c.hw_trigger.as_ref()));
        let next = Match::next(s.settings.trigger.get(stage, ch), &allowed);
        s.settings.trigger.set(stage, ch, next);
        if next.is_some() {
            s.settings.mode = CaptureMode::Triggered;
        }
        self.mark_settings_dirty();
    }

    /// Opens the Inspector on the Trigger tab, on `stage`.
    pub(crate) fn show_trigger_tab(&mut self, stage: usize) {
        self.ui.trigger_stage = stage;
        self.toggle_panel(TRIGGER_PANEL);
    }

    /// The Trigger tab's header: Reset.
    pub(crate) fn trigger_panel_actions(&mut self, ui: &mut Ui) {
        let Some(s) = self.session.as_mut() else { return };
        let locked = s.is_read_only() || s.state.is_busy() || s.caps.is_none();
        let reset = ui.add_enabled(!locked, Button::tertiary(tr("Reset")));
        if reset.on_hover_text(tr("Restore the default trigger")).clicked() {
            s.settings.trigger = DigitalTrigger::reset(&s.settings.enabled);
            self.mark_settings_dirty();
        }
    }

    /// The Trigger tab: the trigger in words, how long the capture is
    /// around it, what a stage has beyond its conditions, and how two
    /// stages combine. What the device cannot do is disabled and says
    /// why.
    pub(crate) fn trigger_panel(&mut self, ui: &mut Ui) {
        let theme = ui.la_theme();
        let Some(s) = self.session.as_ref() else { return };
        if let Some(err) = &s.open_error {
            note(ui, err, Token::Alert);
            return;
        }
        // A loaded file shows the trigger it was recorded with, read-only.
        let read_only = s.is_read_only();
        if s.caps.is_none() && !read_only {
            hint(ui, tr("Opening device…"));
            return;
        }
        let recorded = s.snapshot.as_ref().and_then(|snap| snap.trigger.clone());
        let mut t = match (read_only, recorded) {
            (false, _) => s.settings.trigger.clone(),
            (true, Some(t)) => t,
            (true, None) => {
                hint(ui, tr("This capture was recorded without a trigger."));
                return;
            }
        };
        let before = t.clone();
        let busy = s.state.is_busy();
        let locked = read_only || busy;
        let caps = s.caps.clone();
        let hw = caps.as_ref().and_then(|c| c.hw_trigger.as_ref());
        let rate = s.settings.sample_rate;
        let mode = s.settings.mode;
        let armed = read_only || mode == CaptureMode::Triggered;
        let any_digital = s.settings.enabled.iter().any(|c| c.kind == ChannelKind::Digital);
        // The enabled digital channels, in the device's order.
        let mut channels: Vec<(ChannelId, String)> = s
            .rows
            .iter()
            .filter(|r| r.channel.kind == ChannelKind::Digital && s.settings.is_enabled(r.channel))
            .map(|r| (r.channel, r.display()))
            .collect();
        channels.sort_by_key(|c| c.0);
        let idents: Vec<(ChannelId, String)> = s.rows.iter().map(|r| (r.channel, r.identifier())).collect();
        let name = move |c: ChannelId| idents.iter().find(|x| x.0 == c).map_or_else(|| c.short_name(), |x| x.1.clone());
        let stages = trigger::stages(hw).max(t.stages.iter().rposition(|s| !s.is_empty()).map_or(0, |n| n + 1));
        let mut k = if stages > 1 { self.ui.trigger_stage.min(1) } else { 0 };
        // A device that waits for the trigger keeps only the pre-trigger
        // time before it; one the app scans keeps it all.
        let pre_help = if !caps.as_ref().is_some_and(|c| c.supports_continuous) {
            tr(
                "This device records into its own memory, so it keeps only this much data before the trigger. Long durations use more device memory; with RLE the result can be longer.",
            )
        } else if crate::app::waits_in_device(&t, caps.as_ref(), rate) {
            tr("The device waits for this trigger itself, so the capture keeps only this much data before it.")
        } else {
            tr(
                "The trigger is looked for after this much data, so that it is there before the trigger. The capture keeps everything since Start that fits the memory buffer.",
            )
        };

        // Why the trigger cannot be changed, or is not used.
        let mut use_trigger_mode = false;
        if read_only {
            hint(ui, tr("The trigger this capture was recorded with."));
            ui.add_space(12.0);
        } else if busy {
            hint(ui, tr("Stop the capture to change the trigger."));
            ui.add_space(12.0);
        } else if !armed {
            hint(ui, &tr("The capture mode is {}, so this trigger is not used.").replacen("{}", mode_name(mode), 1));
            ui.add_space(6.0);
            let r = ui
                .add_enabled(any_digital, Button::secondary(tr("Use Trigger Mode")))
                .on_disabled_hover_text(tr("Enable a digital channel first."));
            use_trigger_mode = r.clicked();
            ui.add_space(14.0);
        }
        // The trigger in words: a line per stage.
        let shown = if stages > 1 { stages } else { usize::from(!t.stages[0].is_empty()) };
        for (letter, stage) in STAGE_LETTERS.iter().zip(&t.stages).take(shown) {
            ui.horizontal_wrapped(|ui| {
                if stages > 1 {
                    let letter = egui::RichText::new(*letter).text_style(Text::ValueS.style());
                    ui.label(letter.color(theme.color(Token::Ink3)));
                }
                let text = stage_summary(stage, &name);
                if text.is_empty() {
                    note(ui, tr("No condition"), Token::Ink4);
                } else {
                    note(ui, &text, Token::Ink2);
                }
            });
        }
        // What looks for the trigger, or why nothing can.
        let planned = !read_only && (armed || !t.is_empty());
        if planned {
            match crate::app::plan_trigger(&t, caps.as_ref(), 0, rate) {
                Ok(e) if e.device().is_some() => note(ui, tr("The device waits for this trigger."), Token::Ink3),
                Ok(_) => note(ui, tr("The app looks for this trigger in the data."), Token::Ink3),
                Err(e) => note(ui, &e.text(), Token::Alert),
            }
        }
        if planned || shown > 0 {
            ui.add_space(20.0);
            divider(ui);
        }
        ui.add_enabled_ui(!locked, |ui| {
            ui.horizontal(|ui| {
                caption(ui, tr("Capture duration before trigger"));
                super::panels::help_icon(ui, pre_help);
            });
            ui.add_space(6.0);
            time_input(ui, tr("Capture duration before trigger"), &mut t.pre_trigger_seconds, 0.0);
            ui.add_space(10.0);
            caption(ui, tr("Capture duration after trigger"));
            ui.add_space(6.0);
            time_input(ui, tr("Capture duration after trigger"), &mut t.post_trigger_seconds, 0.0);
            ui.add_space(20.0);
            divider(ui);
            // The stage, where there are two.
            if stages > 1 || hw.is_none() {
                let b = lacking(hw, |c| c.stages() > 1, TriggerError::Stages).filter(|_| stages < 2);
                choice(ui, tr("Stage"), &[(tr("Stage A"), None), (tr("Stage B"), b)], &mut k);
                ui.add_space(10.0);
            }
            if t.stages[k].conditions.is_empty() {
                note(ui, tr("No condition yet: click a channel's trigger icon to set one."), Token::Ink3);
                ui.add_space(14.0);
            }
            stage_extras(ui, &mut t.stages[k], hw, &channels);
            ui.add_space(16.0);
            let clear = ui.add_enabled(t.stages[k] != Stage::default(), Button::secondary(tr("Clear Stage")));
            if clear.clicked() {
                t.stages[k] = Stage::default();
            }
            // How the two stages combine.
            if stages > 1 {
                ui.add_space(20.0);
                caption(ui, tr("Stages A and B"));
                ui.add_space(6.0);
                let both = t.stages.iter().all(|s| !s.is_empty());
                let need_both = (!both).then(|| tr("Set a condition in both stages first.").to_owned());
                let or = need_both.clone().or_else(|| lacking(hw, |c| c.or, TriggerError::Or));
                let and = need_both.clone().or_else(|| lacking(hw, |c| c.and, TriggerError::And));
                let mut c = match t.combine {
                    Combine::Then => 0,
                    Combine::Or => 1,
                    Combine::And => 2,
                };
                let options = [(tr("A, then B"), need_both), (tr("A or B"), or), (tr("A and B"), and)];
                if choice(ui, tr("Stages A and B"), &options, &mut c) {
                    t.combine = [Combine::Then, Combine::Or, Combine::And][c];
                }
            }
        });
        self.ui.trigger_stage = k;
        if !read_only && t != before {
            if let Some(s) = self.session.as_mut() {
                s.settings.trigger = t;
            }
            self.mark_settings_dirty();
        }
        if use_trigger_mode {
            self.set_capture_mode(CaptureMode::Triggered);
        }
    }
}

/// A stage's count, duration or pulse width, and value range.
fn stage_extras(ui: &mut Ui, stage: &mut Stage, hw: Option<&HwTriggerCaps>, channels: &[(ChannelId, String)]) {
    let has_edge = stage.has_edge();
    // Two inputs side by side, with a dash between them, fit the panel
    // at its narrowest.
    let half = ((ui.available_width() - 24.0) / 2.0).max(60.0);
    let (input_w, dropdown_w) = (half.min(130.0), half.min(170.0));
    // ---- count ----
    let mut counted = stage.count.is_some();
    let why = lacking(hw, |c| c.count, TriggerError::Count).filter(|_| !counted);
    ui.horizontal(|ui| {
        let r = ui.add_enabled(why.is_none(), Checkbox::new(&mut counted, tr("Trigger on the Nth match")));
        if let Some(why) = &why {
            r.on_disabled_hover_text(why);
        }
    });
    if counted != stage.count.is_some() {
        stage.count = counted.then_some(StageCount { n: 2, consecutive: false });
    }
    if let Some(c) = stage.count.as_mut() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut n = f64::from(c.n);
            ui.allocate_ui(vec2(110.0, 40.0), |ui| {
                if number_input(ui, tr("Match count"), &mut n, "", Some((1.0, 1_000_000.0, 1.0))) {
                    c.n = n as u32;
                }
            });
            ui.add_space(12.0);
            let why = lacking(hw, |c| c.consecutive, TriggerError::Consecutive)
                .or_else(|| (!has_edge).then(|| TriggerError::ConsecutiveNeedsEdge.text()))
                .filter(|_| !c.consecutive);
            let r = ui.add_enabled(why.is_none(), Checkbox::new(&mut c.consecutive, tr("In a row")));
            if let Some(why) = &why {
                r.on_disabled_hover_text(why);
            }
        });
    }
    ui.add_space(12.0);
    // ---- duration: a pulse width on an edge, a hold on levels ----
    let label = if has_edge { tr("Pulse width") } else { tr("Held for a time") };
    let mut held = stage.hold.is_some();
    // The app finds pulse widths itself; a hold of levels is the device's.
    let why = if has_edge { None } else { lacking(hw, |c| c.duration, TriggerError::Duration) }
        .or_else(|| stage.count.is_some().then(|| TriggerError::CountAndHold.text()))
        .filter(|_| !held);
    ui.horizontal(|ui| {
        let r = ui.add_enabled(why.is_none(), Checkbox::new(&mut held, label));
        if let Some(why) = &why {
            r.on_disabled_hover_text(why);
        }
    });
    if held != stage.hold.is_some() {
        stage.hold = held.then_some(if has_edge { Hold::Between(1e-3, 10e-3) } else { Hold::Longer(1e-3) });
    }
    if let Some(hold) = stage.hold.as_mut() {
        ui.add_space(6.0);
        let (mut kind, mut a, mut b) = match *hold {
            Hold::Longer(s) => (0, s, s.max(1e-9) * 10.0),
            Hold::Shorter(s) => (1, s, s.max(1e-9) * 10.0),
            Hold::Between(a, b) => (2, a, b),
        };
        let levels = (!has_edge).then(|| TriggerError::HoldLevels.text());
        let options = [(tr("Longer than"), None), (tr("Shorter than"), levels.clone()), (tr("Between"), levels)];
        let mut changed = choice(ui, label, &options, &mut kind);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.allocate_ui(vec2(input_w, 40.0), |ui| {
                let name = if kind == 2 { tr("Pulse width minimum") } else { tr("Duration") };
                changed |= time_input(ui, name, &mut a, 0.0);
            });
            if kind == 2 {
                ui.label("–");
                ui.allocate_ui(vec2(input_w, 40.0), |ui| {
                    changed |= time_input(ui, tr("Pulse width maximum"), &mut b, 0.0);
                });
            }
        });
        if changed {
            *hold = match kind {
                0 => Hold::Longer(a),
                1 => Hold::Shorter(a),
                _ => Hold::Between(a.min(b), a.max(b)),
            };
        }
    }
    ui.add_space(12.0);
    // ---- value range ----
    let mut ranged = stage.range.is_some();
    let why = lacking(hw, |c| c.range, TriggerError::Range)
        .or_else(|| (channels.len() < 2).then(|| tr("A value range needs two or more channels.").to_owned()))
        .filter(|_| !ranged);
    ui.horizontal(|ui| {
        let r = ui.add_enabled(why.is_none(), Checkbox::new(&mut ranged, tr("Value on a group of channels")));
        if let Some(why) = &why {
            r.on_disabled_hover_text(why);
        }
    });
    if ranged != stage.range.is_some() {
        stage.range = ranged.then(|| {
            let group: Vec<ChannelId> = channels.iter().take(4).map(|c| c.0).collect();
            let high = (1u64 << group.len()) - 1;
            ValueRange { channels: group, low: 0, high, inside: true }
        });
        if stage.is_empty() {
            *stage = Stage::default();
        }
    }
    if let Some(r) = stage.range.as_mut() {
        ui.add_space(6.0);
        let names: Vec<&str> = channels.iter().map(|c| c.1.as_str()).collect();
        let at = |c: Option<&ChannelId>| c.and_then(|c| channels.iter().position(|x| x.0 == *c)).unwrap_or(0);
        let (mut first, mut last) = (at(r.channels.first()), at(r.channels.last()));
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.allocate_ui(vec2(dropdown_w, 40.0), |ui| {
                changed |= ui.add(Dropdown::new(tr("Least significant channel"), &mut first, &names)).changed();
            });
            ui.label("–");
            ui.allocate_ui(vec2(dropdown_w, 40.0), |ui| {
                changed |= ui.add(Dropdown::new(tr("Most significant channel"), &mut last, &names)).changed();
            });
        });
        if changed && !channels.is_empty() {
            // At most 63 channels make a value.
            let a = first.min(last);
            let b = first.max(last).min(a + 62);
            r.channels = channels[a..=b].iter().map(|c| c.0).collect();
        }
        let max = (1u64 << r.channels.len().min(63)) - 1;
        (r.low, r.high) = (r.low.min(max), r.high.min(max));
        ui.add_space(6.0);
        let mut side = usize::from(!r.inside);
        if choice(ui, tr("Value range"), &[(tr("Within"), None), (tr("Outside"), None)], &mut side) {
            r.inside = side == 0;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.allocate_ui(vec2(input_w, 40.0), |ui| {
                if whole_input(ui, tr("Lowest value"), &mut r.low, max) {
                    r.high = r.high.max(r.low);
                }
            });
            ui.label("–");
            ui.allocate_ui(vec2(input_w, 40.0), |ui| {
                if whole_input(ui, tr("Highest value"), &mut r.high, max) {
                    r.low = r.low.min(r.high);
                }
            });
        });
        note(ui, &tr("of 0–{}").replacen("{}", &max.to_string(), 1), Token::Ink3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        let d = ChannelId::digital;
        let name = |c: ChannelId| c.short_name();
        let mut t = DigitalTrigger::on(d(0), Match::Rising);
        t.set(0, d(1), Some(Match::High));
        assert_eq!(stage_summary(&t.stages[0], &name), "D0 Rising Edge, D1 High");
        t.stages[0].count = Some(StageCount { n: 3, consecutive: true });
        assert_eq!(stage_summary(&t.stages[0], &name), "D0 Rising Edge, D1 High ×3 in a row");
        let p = DigitalTrigger::pulse(d(2), true, 1e-3, 2e-3);
        assert_eq!(stage_summary(&p.stages[0], &name), "D2 High pulse, 1 ms–2 ms");
        let r = Stage {
            range: Some(ValueRange { channels: vec![d(4), d(5), d(6)], low: 2, high: 5, inside: false }),
            hold: Some(Hold::Longer(50e-6)),
            ..Stage::default()
        };
        assert_eq!(stage_summary(&r, &name), "D4–D6 outside 2..5, > 50 µs");
    }
}

//! Bubbles on the waveform: lanes above each channel's trace.

use super::markers::c32;
use crate::app::LaApp;
use crate::core::{CaptureTime, ChannelId, ViewState};
use crate::decode::format::{BubbleGroup, count_badge, fit_text, merge_bubbles, multi_text};
use crate::decode::runner::Output;
use crate::decode::{Radix, RadixSupport, all_rows};
use crate::ui::Token;
use crate::ui::theme::Theme;
use crate::ui::typography::Text;
use crate::ui::widgets::{menu_check_item, menu_item, popover_frame};
use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Popup, Rect, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2,
    vec2,
};
use std::collections::HashMap;
use std::sync::Arc;

/// 16 pt bubbles with 4 pt padding.
pub(crate) const LANE_H: f32 = 20.0;
const BUBBLE_H: f32 = 16.0;
/// Text width estimate per character.
const CHAR_PX: f32 = 6.5;
/// Above this many frames in view, a lane draws one summary band.
const MAX_FRAMES_DRAWN: usize = 20_000;

/// One lane: an analyzer's annotation row on a channel.
#[derive(Clone)]
pub(crate) struct Lane {
    pub analyzer: u32,
    pub name: String,
    pub row: usize,
    pub row_desc: String,
    /// The class named like the row (`rx-data` in row `rx-data`): a merged
    /// bubble spells only these when it has any.
    pub main_class: Option<usize>,
    pub color: [u8; 3],
    pub radix: Radix,
    /// Which radixes the decoder allows.
    pub support: RadixSupport,
    pub out: Arc<Output>,
}

/// A radix menu item; without raw values only Native is enabled.
pub(crate) fn radix_item(ui: &mut Ui, support: Option<RadixSupport>, r: Radix, checked: bool) -> egui::Response {
    let ok = r == Radix::Native || support != Some(RadixSupport::NativeOnly);
    let resp = ui.add_enabled_ui(ok, |ui| menu_check_item(ui, r.label(), checked, None)).inner;
    resp.on_disabled_hover_text("This decoder formats its own values")
}

/// A bubble action, applied after painting.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BubbleAct {
    /// Zoom to samples `[a, b)` with padding.
    Zoom(u64, u64),
    Center(CaptureTime),
    Radix(u32, Radix),
    Copy(String),
    Table(u32, u32),
}

impl LaApp {
    /// Center the nearest analyzer result before / after the view
    /// center, over every lane of channel `id`.
    pub(crate) fn step_result(&mut self, id: ChannelId, next: bool) {
        let lanes = self.lane_map();
        let (Some(lanes), Some(s)) = (lanes.get(&id), self.session.as_ref()) else { return };
        let Some(rate) = s.capture.as_ref().and_then(|c| c.read().sample_rate()) else { return };
        let c = s.view.target().center_time().to_sample_floor(rate).max(0) as u64;
        let mid = |f: &crate::decode::Frame| f.start + (f.end - f.start) / 2;
        let best = lanes
            .iter()
            .filter_map(|lane| {
                let frames = lane.out.frames.read();
                let f = if next { frames.next_in_row(lane.row, c) } else { frames.prev_in_row(lane.row, c) };
                f.map(|(_, f)| mid(f))
            })
            .reduce(|a, b| if next { a.min(b) } else { a.max(b) });
        if let Some(m) = best {
            self.center_on(CaptureTime::from_sample(m, rate));
        }
    }

    /// The lanes of each channel, in analyzer then row order.
    pub(crate) fn lane_map(&self) -> HashMap<ChannelId, Vec<Lane>> {
        let mut map: HashMap<ChannelId, Vec<Lane>> = HashMap::new();
        let Some(s) = self.session.as_ref() else { return map };
        let list = self.decoder_list();
        for a in &s.analyzers {
            let Some(info) = list.iter().find(|d| d.id == a.decoder) else { continue };
            let rows = all_rows(info);
            for (row, ch) in a.lanes(info) {
                map.entry(ch).or_default().push(Lane {
                    analyzer: a.id,
                    name: a.name.clone(),
                    row,
                    row_desc: rows[row].1.clone(),
                    main_class: info.annotations.iter().position(|c| c.0 == rows[row].0),
                    color: a.color,
                    radix: a.radix,
                    support: crate::decode::radix_support(info),
                    out: a.out.clone(),
                });
            }
        }
        map
    }

    /// The label column part of the lanes, one line per lane.
    pub(crate) fn paint_lane_labels(painter: &Painter, rect: Rect, lanes: &[Lane], stripe: Color32, theme: &Theme) {
        painter.rect_filled(rect, 0.0, theme.color(Token::Deck));
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(5.0, rect.height())), 0.0, stripe);
        painter.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, theme.color(Token::Seam)));
        for (k, lane) in lanes.iter().enumerate() {
            let cy = rect.top() + LANE_H * k as f32 + LANE_H / 2.0;
            let sq = Rect::from_center_size(pos2(rect.left() + 15.0, cy), vec2(7.0, 7.0));
            painter.rect_filled(sq, 1.0, c32(lane.color));
            let text = format!("{} {}", lane.name, lane.row_desc);
            let mut job =
                egui::text::LayoutJob::simple_singleline(text, Text::CaptionMed.font_id(), theme.color(Token::Ink3));
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 28.0).max(0.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(rect.left() + 23.0, cy - g.size().y / 2.0), g, Color32::PLACEHOLDER);
        }
    }

    /// The bubbles of `lanes` over `cell` (the plot part).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_lanes(
        &self,
        ui: &Ui,
        painter: &Painter,
        cell: Rect,
        lanes: &[Lane],
        view: &ViewState,
        rate: u64,
        theme: &Theme,
        acts: &mut Vec<BubbleAct>,
    ) {
        let x_of = |s: u64| cell.left() + view.time_to_px(CaptureTime::from_sample(s, rate)) as f32;
        let s0 = view.left_time.to_sample_floor(rate).max(0) as u64;
        let s1 = view.right_time().to_sample_floor(rate).max(0) as u64 + 1;
        let clip = painter.with_clip_rect(cell.intersect(painter.clip_rect()));
        let font = FontId::new(12.0, Text::Prose.font_id().family);
        for (k, lane) in lanes.iter().enumerate() {
            let y = cell.top() + LANE_H * k as f32 + (LANE_H - BUBBLE_H) / 2.0;
            let frames = lane.out.frames.read();
            let in_view: Vec<(u32, &crate::decode::Frame)> =
                frames.in_row(lane.row, s0, s1).take(MAX_FRAMES_DRAWN + 1).collect();
            let color = c32(lane.color);
            if in_view.len() > MAX_FRAMES_DRAWN {
                // Too dense to draw one by one: a band from the first frame on.
                let x0 = x_of(in_view[0].1.start).max(cell.left());
                let band = Rect::from_x_y_ranges(x0..=cell.right(), y..=y + BUBBLE_H);
                clip.rect_filled(band, 2.0, color.gamma_multiply(0.85));
                clip.text(
                    band.center(),
                    Align2::CENTER_CENTER,
                    "many results",
                    font.clone(),
                    theme.color(Token::InkOnSwatch),
                );
                continue;
            }
            let xs: Vec<(f32, f32)> = in_view.iter().map(|(_, f)| (x_of(f.start), x_of(f.end))).collect();
            for g in merge_bubbles(&xs) {
                self.paint_group(ui, &clip, cell, y, lane, &in_view, &g, &font, theme, acts);
            }
            // Previous / next result beyond the view.
            for next in [false, true] {
                let target = if next { frames.next_in_row(lane.row, s1) } else { frames.prev_in_row(lane.row, s0) };
                let Some((_, f)) = target else { continue };
                let c = pos2(if next { cell.right() - 10.0 } else { cell.left() + 10.0 }, y + BUBBLE_H / 2.0);
                let r = Rect::from_center_size(c, vec2(16.0, 16.0));
                let label = format!("{} {} result", lane.name, if next { "next" } else { "previous" });
                let resp =
                    ui.interact(r, ui.make_persistent_id(("lane_nav", lane.analyzer, lane.row, next)), Sense::click());
                resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
                clip.circle_filled(c, 8.0, theme.color(Token::Field));
                clip.circle_stroke(c, 8.0, Stroke::new(1.0, theme.color(Token::Seam)));
                crate::ui::icons::left_arrow(&clip, c, 7.0, theme.color(Token::Ink2));
                if next {
                    // Mirror the arrow: a right-pointing one.
                    clip.circle_filled(c, 7.0, theme.color(Token::Field));
                    let pts = vec![c + vec2(-2.1, -3.5), c + vec2(-2.1, 3.5), c + vec2(2.1, 0.0)];
                    clip.add(egui::Shape::convex_polygon(pts, theme.color(Token::Ink2), Stroke::NONE));
                }
                if resp.on_hover_text(if next { "Next result" } else { "Previous result" }).clicked() {
                    let mid = f.start + (f.end - f.start) / 2;
                    acts.push(BubbleAct::Center(CaptureTime::from_sample(mid, rate)));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_group(
        &self,
        ui: &Ui,
        p: &Painter,
        cell: Rect,
        y: f32,
        lane: &Lane,
        frames: &[(u32, &crate::decode::Frame)],
        g: &BubbleGroup,
        font: &FontId,
        theme: &Theme,
        acts: &mut Vec<BubbleAct>,
    ) {
        let color = c32(lane.color);
        let full = Rect::from_x_y_ranges(g.x0..=g.x1.max(g.x0), y..=y + BUBBLE_H);
        let rect = full.intersect(Rect::from_x_y_ranges(cell.x_range(), full.y_range()));
        if rect.width() <= 0.0 {
            return;
        }
        let group: Vec<&crate::decode::Frame> = frames[g.first..=g.last].iter().map(|(_, f)| *f).collect();
        let error = group.iter().any(|f| f.error);
        let id = ui.make_persistent_id(("bubble", lane.analyzer, lane.row, frames[g.first].0));
        let resp = ui.interact(rect, id, Sense::click());
        if full.width() < 3.0 {
            // A 1 pt tick.
            p.line_segment([pos2(full.left(), y), pos2(full.left(), y + BUBBLE_H)], Stroke::new(1.0, color));
            return;
        }
        let alpha = if resp.hovered() { 1.0 } else { 0.85 };
        p.rect(
            rect,
            CornerRadius::same(2),
            color.gamma_multiply(alpha),
            Stroke::new(1.0, color.gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
        if error {
            p.rect_stroke(rect, 2.0, Stroke::new(2.0, theme.color(Token::Alert)), StrokeKind::Inside);
        }
        let ink = theme.color(Token::InkOnSwatch);
        let (text, tip) = if g.count() == 1 {
            let f = group[0];
            let t = fit_text(&f.texts, rect.width() - 6.0, CHAR_PX).map(str::to_owned);
            let t = match t {
                None if f.error => Some("!".to_owned()),
                t => t,
            };
            (t, f.longest().to_owned())
        } else {
            // multi-bubble: count badge and joined short strings.
            // Data frames only, when the group has any (not start/stop bits).
            let main: Vec<&&crate::decode::Frame> = group.iter().filter(|f| Some(f.class) == lane.main_class).collect();
            let joined = if main.is_empty() {
                multi_text(group.iter().map(|f| f.shortest()), lane.radix)
            } else {
                multi_text(main.iter().map(|f| f.shortest()), lane.radix)
            };
            let badge = count_badge(g.count());
            let bw = badge.len() as f32 * 6.0 + 8.0;
            let bc = pos2(rect.left() + bw / 2.0 + 1.0, rect.center().y);
            p.rect_filled(
                Rect::from_center_size(bc, vec2(bw, BUBBLE_H - 4.0)),
                6.0,
                color.gamma_multiply(0.5).to_opaque(),
            );
            p.text(bc, Align2::CENTER_CENTER, &badge, FontId::new(10.0, font.family.clone()), ink);
            let room = rect.width() - bw - 6.0;
            let n = (room / CHAR_PX).floor().max(0.0) as usize;
            let shown: String = if joined.chars().count() > n {
                joined.chars().take(n.saturating_sub(1)).chain(std::iter::once('…')).collect()
            } else {
                joined.clone()
            };
            let first80: String = joined.chars().take(80).collect();
            ((n > 1).then_some(shown), format!("[{} results] {first80}", g.count()))
        };
        if let Some(t) = &text {
            let center = if g.count() > 1 { pos2(rect.center().x + 8.0, rect.center().y) } else { rect.center() };
            p.with_clip_rect(rect).text(center, Align2::CENTER_CENTER, t, font.clone(), ink);
        }
        let shown = text.clone().unwrap_or_default();
        resp.widget_info(|| WidgetInfo::labeled(Role::Label, true, &shown));
        let cmd = super::cmd_key();
        let resp = resp.on_hover_ui(|ui| {
            ui.label(&tip);
            ui.label(egui::RichText::new(format!("{cmd}+click to jump to that row in the table")).weak());
        });
        let (first, last) = (group[0], group[group.len() - 1]);
        if resp.double_clicked() {
            acts.push(BubbleAct::Zoom(first.start, last.end));
        } else if resp.clicked() && ui.input(|i| i.modifiers.command) {
            acts.push(BubbleAct::Table(lane.analyzer, frames[g.first].0));
        }
        // right-click: radix and Copy.
        Popup::context_menu(&resp).frame(popover_frame(ui)).width(180.0).show(|ui| {
            for r in Radix::ALL {
                if radix_item(ui, Some(lane.support), r, lane.radix == r).clicked() {
                    acts.push(BubbleAct::Radix(lane.analyzer, r));
                }
            }
            if menu_item(ui, "Copy", None).clicked() {
                acts.push(BubbleAct::Copy(text.clone().unwrap_or(tip.clone())));
            }
        });
    }

    /// Applies what the bubbles asked for.
    pub(crate) fn apply_bubble_acts(&mut self, ui: &Ui, acts: Vec<BubbleAct>, rate: u64) {
        for a in acts {
            match a {
                BubbleAct::Zoom(a, b) => {
                    self.zoom_on(CaptureTime::from_sample(a, rate), CaptureTime::from_sample(b, rate))
                }
                BubbleAct::Center(t) => self.center_on(t),
                BubbleAct::Radix(id, r) => {
                    if let Some(x) = self.session.as_mut().and_then(|s| s.analyzers.iter_mut().find(|x| x.id == id)) {
                        x.radix = r;
                    }
                    self.restart_analyzer(id);
                }
                BubbleAct::Copy(t) => {
                    ui.ctx().copy_text(t);
                    self.toast(crate::ui::toast::Toast::new(crate::ui::toast::ToastKind::Success, "Copied"));
                }
                BubbleAct::Table(analyzer, frame) => self.reveal_in_table(analyzer, frame),
            }
        }
    }
}

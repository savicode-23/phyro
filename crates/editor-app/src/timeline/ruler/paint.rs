use super::{
    keys::key_at, layout::Band, Gesture, Grid, Row, Timeline, Tracks, ACCENT, KEY, LABEL_WIDTH,
};
use crate::theme::{PETROL, RAISED as HEADER_FILL, SELECTED, SURFACE};
use crate::timeline::FrameSelection;
use document_core::Clip;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Painter, Rect, Sense, Ui};

const LINE: Color32 = Color32::from_rgb(57, 65, 71);
pub(super) const TWEEN_BAR: Color32 = Color32::from_rgb(145, 117, 74);
const FRAME_HEADER: Color32 = Color32::from_rgb(54, 92, 86);
const FRAME_EDGE: Color32 = Color32::from_rgb(117, 158, 145);
const FRAME_INK: Color32 = Color32::from_rgb(207, 219, 214);

impl Timeline {
    pub(super) fn paint_sheet(
        &mut self,
        ui: &mut Ui,
        grid: &Grid,
        clip: &Clip,
        tracks: &Tracks,
        group: (egui::Id, bool),
    ) {
        let grid_view = grid.frame_area();
        if !grid_view.is_positive() {
            return;
        }
        let header = Rect::from_min_max(
            grid.view.min,
            pos2(grid.view.right(), grid.rows_top().min(grid.view.bottom())),
        );
        let header_painter = ui.painter_at(header);
        header_painter.rect_filled(header, 0., HEADER_FILL);
        let selection = self.focused_frames(clip);
        paint_frame_summary(
            &header_painter,
            header.with_max_x(grid_view.left()),
            selection,
            ui.visuals().text_color(),
        );
        let ruler = header.intersect(grid.time_view());
        let head = ui.painter_at(ruler);
        let first = grid.tick(grid_view.left(), clip.frames);
        let last = grid.tick(grid_view.right(), clip.frames);
        let body = grid.body_clip();
        let time_body = body.intersect(grid_view);
        let painter = ui.painter_at(time_body);
        if body.is_positive() {
            ui.painter_at(body).rect_filled(body, 0., SURFACE);
        }
        for range in selected_columns(selection, grid, first, last, ruler.y_range()) {
            head.rect_filled(range, 0., FRAME_HEADER);
        }
        let digit_width = ui.fonts(|f| f.glyph_width(&FontId::monospace(12.), '0'));
        let max_width = digit_width * (clip.frames.ilog10() + 1) as f32;
        let current_width = digit_width * ((self.frame + 1).ilog10() + 1) as f32;
        let label_spacing = max_width + 3.;
        let stride = [1, 2, 5, 10, 20]
            .into_iter()
            .find(|&step| step as f32 * grid.cell >= label_spacing)
            .unwrap_or(20);
        let playhead_x = grid.x(self.frame) + grid.cell * 0.5;
        let margin = current_width * 0.5 + 1.;
        let playhead_label_x = if ruler.width() > margin * 2. {
            playhead_x.clamp(ruler.left() + margin, ruler.right() - margin)
        } else {
            ruler.center().x
        };
        let number_ink = if selection.is_some() {
            FRAME_INK
        } else {
            ui.visuals().text_color()
        };
        for tick in first..=last {
            let x = grid.x(tick);
            let cell =
                Rect::from_min_max(pos2(x, header.top()), pos2(x + grid.cell, header.bottom()));
            if tick == self.frame && selection.is_none() {
                head.rect_filled(cell.shrink2(vec2(1., 1.)), 3., SELECTED);
            }
            let numbered = tick == 0 || (tick + 1).is_multiple_of(stride);
            if grid.cell >= 8. || numbered {
                head.vline(x, ruler.y_range(), (1., LINE));
                if body.is_positive() {
                    painter.vline(x, time_body.y_range(), (1., LINE));
                }
            }
            // Keep the active frame legible at either end, even in a three-point cell.
            let current = tick == self.frame;
            let clear_of_playhead = !(first..=last).contains(&self.frame)
                || (cell.center().x - playhead_label_x).abs()
                    >= (current_width + max_width) * 0.5 + 3.;
            if !current && numbered && clear_of_playhead {
                head.text(
                    cell.center(),
                    Align2::CENTER_CENTER,
                    tick + 1,
                    FontId::monospace(12.),
                    number_ink,
                );
            }
        }
        let playhead_ink = if selection.is_some() {
            FRAME_INK
        } else {
            ACCENT
        };
        if (first..=last).contains(&self.frame) {
            let at = pos2(playhead_label_x, ruler.center().y);
            if selection.is_some() {
                head.rect_filled(
                    Rect::from_center_size(at, vec2(current_width + 4., ruler.height() - 4.)),
                    3.,
                    PETROL,
                );
            }
            head.text(
                at,
                Align2::CENTER_CENTER,
                self.frame + 1,
                FontId::monospace(12.),
                playhead_ink,
            );
        }
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let marquee = match (self.gesture, pointer) {
            (Some(Gesture::BoxKeys { origin, .. }), Some(pos)) => {
                Some(Rect::from_two_pos(grid.content.min + origin, pos))
            }
            _ => None,
        };
        let shift = match (self.gesture, pointer) {
            (Some(Gesture::MoveKeys { from }), Some(pos)) => {
                grid.tick(pos.x, clip.frames) as i32 - from as i32
            }
            _ => 0,
        };
        let object = tracks.object(clip);
        for band in tracks.bands(
            body.top() - grid.content.top(),
            body.bottom() - grid.content.top(),
        ) {
            let row_rect = grid.band(&band);
            if !row_rect.intersect(body).is_positive() {
                continue;
            }
            let center = row_rect.center().y;
            let row_painter = ui.painter_at(row_rect.intersect(time_body));
            match band.row {
                Row::Layer(index) => {
                    let layer = &clip.cels[&tracks.drawing_start].layers[index];
                    paint_drawings(&row_painter, grid, row_rect, clip, layer.id, first..=last);
                }
                Row::Object => {
                    row_painter.rect_filled(row_rect, 0., HEADER_FILL);
                }
                row => {
                    super::tweens::visible(row, clip, tracks, first..=last, |a, b| {
                        let left = (grid.x(a) + grid.cell * 0.5).max(time_body.left());
                        let right = (grid.x(b) + grid.cell * 0.5).min(time_body.right());
                        if left < right {
                            row_painter.rect_filled(
                                Rect::from_min_max(
                                    pos2(left, center - 2.5),
                                    pos2(right, center + 2.5),
                                ),
                                1.,
                                TWEEN_BAR,
                            );
                        }
                    });
                    for tick in first..=last {
                        let keyed = key_at(row, clip, object, tracks.bone, &tracks.exposure, tick);
                        if keyed {
                            let at = pos2(grid.x(tick) + grid.cell * 0.5, center);
                            let selected = self.keys.marks.contains(&(row, tick))
                                || marquee.is_some_and(|r| r.expand(2.).contains(at));
                            paint_key(&row_painter, at, selected, grid.cell);
                        }
                    }
                    if shift != 0 {
                        let from = first.saturating_add_signed(-shift);
                        let until = last.saturating_add_signed(-shift);
                        for &(_, tick) in self.keys.marks.range((row, from)..=(row, until)) {
                            if let Some(to) = tick
                                .checked_add_signed(shift)
                                .filter(|t| tracks.row_exposure(row).contains(t))
                            {
                                row_painter.circle_stroke(
                                    pos2(grid.x(to) + grid.cell * 0.5, center),
                                    3.5,
                                    (1.3, ACCENT),
                                );
                            }
                        }
                    }
                }
            }
            self.paint_label(ui, grid, tracks, (&band, clip), group.0, group.1);
            ui.painter_at(body)
                .hline(body.x_range(), row_rect.bottom(), (1., LINE));
        }
        // Paint above drawing cells and row backgrounds so every selected column
        // stays visible on populated layers and in the space below the last row.
        if time_body.is_positive() {
            for range in selected_columns(
                self.focused_frames(clip),
                grid,
                first,
                last,
                time_body.y_range(),
            ) {
                painter.rect(
                    range,
                    0.,
                    FRAME_EDGE.gamma_multiply(0.22),
                    (1.25, FRAME_EDGE),
                    egui::StrokeKind::Inside,
                );
            }
        }
        let x = grid.x(self.frame) + grid.cell * 0.5;
        let grid_painter = ui.painter_at(grid_view);
        grid_painter.vline(x, header.bottom()..=grid_view.bottom(), (1.5, playhead_ink));
        grid_painter.circle_filled(pos2(x, header.bottom() - 1.), 2.8, playhead_ink);
        ui.painter_at(grid.view)
            .vline(grid.time_view().left(), grid.view.y_range(), (1., LINE));
        if matches!(
            self.gesture,
            Some(Gesture::Boundary { .. } | Gesture::MoveKeys { .. })
        ) {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let x = grid.x(grid.tick(pos.x, clip.frames)) + grid.cell * 0.5;
                grid_painter.vline(x, grid_view.y_range(), (1., KEY));
            }
        }
        if let Some(outline) = marquee {
            painter.rect_filled(outline, 0., ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(outline, 0., (1., ACCENT), egui::StrokeKind::Inside);
        }
    }

    fn paint_label(
        &mut self,
        ui: &mut Ui,
        grid: &Grid,
        tracks: &Tracks,
        (band, clip): (&Band, &Clip),
        disclosure: egui::Id,
        collapsed: bool,
    ) {
        let row = grid.band(band);
        let label = Rect::from_min_max(row.min, pos2(row.left() + LABEL_WIDTH, row.bottom()));
        let visible = label.intersect(grid.body_clip());
        if !visible.is_positive() {
            return;
        }
        let painter = ui.painter_at(visible);
        let text = match band.row {
            Row::Object => tracks.object_name.as_str(),
            Row::Property(c) => c.label(),
            Row::Layer(index) => &clip.cels[&tracks.drawing_start].layers[index].name,
            Row::Bone => "Bone angle",
            Row::Child => "Child frame",
        };
        let mut color = ui.visuals().text_color();
        let mut indent = 7.;
        match band.row {
            Row::Layer(index) => {
                let layer = &clip.cels[&tracks.drawing_start].layers[index];
                let active = self.active_layer == layer.id;
                let response = ui.interact(
                    visible,
                    ui.id().with(("timeline-layer", layer.id)),
                    Sense::click(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        active,
                        text,
                    )
                });
                if active {
                    painter.rect_filled(label, 0., SELECTED);
                    color = ACCENT;
                } else if response.hovered() || response.has_focus() {
                    painter.rect_filled(label, 0., ui.visuals().widgets.hovered.weak_bg_fill);
                }
                if response.on_hover_text(text).clicked() {
                    self.layer_request = Some(layer.id);
                    self.active_layer = layer.id;
                    self.focus_row_label(tracks.scope);
                }
                if !layer.visible || layer.locked {
                    color = ui.visuals().weak_text_color();
                }
            }
            Row::Object => {
                let response = ui.interact(visible, disclosure.with("toggle"), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::CollapsingHeader,
                        ui.is_enabled(),
                        !collapsed,
                        text,
                    )
                });
                let fill = if response.hovered() || response.has_focus() {
                    ui.visuals().widgets.hovered.weak_bg_fill
                } else {
                    HEADER_FILL
                };
                painter.rect_filled(label, 0., fill);
                let center = label.left_center() + vec2(7., 0.);
                let chevron = if collapsed {
                    [vec2(-2., -4.), vec2(2., 0.), vec2(-2., 4.)]
                } else {
                    [vec2(-4., -2.), vec2(0., 2.), vec2(4., -2.)]
                };
                painter.add(egui::Shape::line(
                    chevron.into_iter().map(|delta| center + delta).collect(),
                    (1.5, color),
                ));
                if response.on_hover_text(text).clicked() {
                    self.focus_row_label(tracks.scope);
                    ui.data_mut(|d| d.insert_temp(disclosure, !collapsed));
                }
                indent = 18.;
            }
            Row::Property(channel) => {
                let response = ui.interact(
                    visible,
                    ui.id().with(("property-row", text)),
                    Sense::click(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        channel == self.channel,
                        text,
                    )
                });
                if response.hovered() || response.has_focus() {
                    painter.rect_filled(label, 0., ui.visuals().widgets.hovered.weak_bg_fill);
                }
                if response.clicked() {
                    self.focus_row_label(tracks.scope);
                    self.channel = channel;
                }
                if channel == self.channel {
                    color = KEY;
                }
                indent = 18.;
            }
            Row::Bone | Row::Child => {
                let response =
                    ui.interact(visible, ui.id().with(("track-label", text)), Sense::click());
                if response.clicked() {
                    self.focus_row_label(tracks.scope);
                }
                if band.row == Row::Bone {
                    response.on_hover_text(format!("{} rotation", tracks.bone_name));
                }
                indent = 18.;
            }
        }
        painter.text(
            label.left_center() + vec2(indent, 0.),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.),
            color,
        );
    }
}

fn paint_drawings(
    painter: &Painter,
    grid: &Grid,
    row: Rect,
    clip: &Clip,
    layer: u64,
    visible: std::ops::RangeInclusive<u32>,
) {
    let band = row.y_range().shrink(3.);
    for (start, end) in visible_holds(clip, layer, visible.clone()) {
        let rect = Rect::from_x_y_ranges(grid.x(start)..=grid.x(end), band);
        let drawing = clip
            .drawing_at(start)
            .objects
            .iter()
            .any(|o| o.layer == layer);
        let fill = if drawing {
            Color32::from_rgb(56, 78, 73)
        } else {
            Color32::from_rgb(55, 63, 67)
        };
        painter.rect_filled(rect, 0., fill);

        // Mark the actual start once, even when a long hold enters from offscreen.
        if visible.contains(&start) {
            let x = grid.x(start) + grid.cell * 0.5;
            let marker = if drawing { FRAME_INK } else { FRAME_EDGE };
            if grid.cell >= 8. {
                painter.circle(
                    pos2(x, band.max - 5.),
                    2.5,
                    if drawing {
                        marker
                    } else {
                        Color32::TRANSPARENT
                    },
                    (1., marker),
                );
            } else {
                painter.vline(x, band.max - 7. ..=band.max - 2., (1., marker));
            }
        }
    }

    // Batch the visible seams into one mesh per row. Align their edges to physical
    // pixels so even the narrowest three-point frames have a crisp interior.
    let mut seams = egui::Mesh::default();
    let scale = painter.pixels_per_point();
    let first = (*visible.start()).max(1);
    let count = (visible.end() + 1).saturating_sub(first) as usize;
    seams.reserve_vertices(count * 4);
    seams.reserve_triangles(count * 2);
    for tick in first..=*visible.end() {
        let beat = tick.is_multiple_of(5);
        let width = if beat && grid.cell >= 8. { 2. } else { 1. };
        let width = (width * scale).round().max(1.) / scale;
        let left = ((grid.x(tick) - width * 0.5) * scale).round() / scale;
        let ink = if beat {
            Color32::from_rgb(22, 30, 32)
        } else {
            Color32::from_rgb(32, 42, 44)
        };
        seams.add_colored_rect(Rect::from_x_y_ranges(left..=left + width, band), ink);
    }
    if !seams.is_empty() {
        painter.add(egui::Shape::mesh(seams));
    }
}

fn paint_frame_summary(
    painter: &Painter,
    gutter: Rect,
    selection: Option<&FrameSelection>,
    text_color: Color32,
) {
    let label = match selection {
        Some(frames) if frames.ranges().len() > 1 => format!(
            "{} frames · {} ranges",
            frames.count(),
            frames.ranges().len()
        ),
        Some(frames) => match frames.ranges().next() {
            Some((lo, hi)) if lo == hi => format!("Frame {} selected", lo + 1),
            Some((lo, hi)) => format!("{}–{} · {} frames", lo + 1, hi + 1, frames.count()),
            None => "No frames selected".to_owned(),
        },
        None => "Frame".to_owned(),
    };
    let ink = if selection.is_some_and(|frames| frames.count() > 0) {
        FRAME_INK
    } else {
        text_color
    };
    let painter = painter.with_clip_rect(gutter);
    let mut galley = painter.layout_no_wrap(label, FontId::proportional(12.), ink);
    let available = (gutter.width() - 14.).max(1.);
    if galley.size().x > available {
        let size = 12. * available / galley.size().x;
        galley = painter.layout_no_wrap(galley.text().to_owned(), FontId::proportional(size), ink);
    }
    let at = gutter.left_center() + vec2(7., -galley.size().y * 0.5);
    painter.galley(at, galley, ink);
}

fn selected_columns<'a>(
    selection: Option<&'a FrameSelection>,
    grid: &'a Grid,
    first: u32,
    last: u32,
    y: egui::emath::Rangef,
) -> impl Iterator<Item = Rect> + 'a {
    selection
        .into_iter()
        .flat_map(move |frames| frames.visible_ranges(first, last))
        .map(move |(lo, hi)| Rect::from_x_y_ranges(grid.x(lo)..=grid.x(hi + 1), y))
}

/// Include a held drawing that began before the viewport. Both iterators stop
/// when the visible starts run out, even when thousands of later cels exist.
pub(super) fn visible_holds(
    clip: &Clip,
    layer: u64,
    visible: std::ops::RangeInclusive<u32>,
) -> impl Iterator<Item = (u32, u32)> + '_ {
    let mut starts = clip.layer_drawing_starts(layer, visible).peekable();
    std::iter::from_fn(move || {
        let start = starts.next()?;
        let end = starts
            .peek()
            .copied()
            .unwrap_or_else(|| start + clip.layer_exposure(start, layer).1);
        Some((start, end))
    })
}

fn paint_key(painter: &Painter, center: egui::Pos2, selected: bool, cell: f32) {
    let dx = vec2((cell * 0.5 - 0.5).min(4.), 0.);
    let dy = vec2(0., 4.);
    painter.add(egui::Shape::convex_polygon(
        vec![center - dy, center + dx, center + dy, center - dx],
        if selected { ACCENT } else { KEY },
        egui::Stroke::NONE,
    ));
    if selected && dx.x >= 2. {
        painter.circle_filled(center, 1.7, Color32::from_rgb(255, 247, 223));
    }
}

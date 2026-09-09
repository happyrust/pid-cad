//! P&ID legend list — docked panel.
//!
//! `PIDLEGEND ON` (and `REPORT` / `LIST`) stash the sheet's [`Recognition`] on
//! its tab; this panel lists it: every recognised symbol grouped by class, and
//! every pipe line number with its runs. Clicking a row zooms the model-space
//! camera to that symbol or that line's whole extent — the panel is the index,
//! the drawing stays the document.
//!
//! The panel follows the active tab. A tab that has not run PIDLEGEND yet
//! shows a hint instead of a list. A list whose drawing has moved on since it
//! was read (`stale`) is marked out of date and greyed: its rows may point at
//! entities that moved or went. Clicking a pipe row reads the sheet again
//! before it selects; `PIDLEGEND LIST` refreshes the list itself.

use crate::app::Message;
use crate::io::pid_legend::Recognition;
use crate::ui::dock::{DockMsg, PanelId};
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, tooltip};
use iced::{Background, Element, Fill, Length, Theme};

/// World-space rectangle a row's click should zoom to: the box padded so the
/// target sits well inside the view instead of touching its edges.
///
/// `bbox` is `(min_x, min_y, max_x, max_y)` in drawing units; `units_per_mm`
/// converts the paper-mm floor. Padding is the larger of 120 % of the box's
/// bigger side and 4 paper-mm, each side, so a tiny valve still gets a
/// readable neighbourhood and a long pipe run keeps a proportional margin.
pub fn jump_rect(bbox: (f64, f64, f64, f64), units_per_mm: f64) -> ((f64, f64), (f64, f64)) {
    let (min_x, min_y, max_x, max_y) = bbox;
    let side = (max_x - min_x).max(max_y - min_y);
    let pad = (side * 1.2).max(4.0 * units_per_mm.max(f64::EPSILON));
    (
        (min_x - pad, min_y - pad),
        (max_x + pad, max_y + pad),
    )
}

/// Union of `rects`, or `None` when the iterator is empty.
fn union(rects: impl Iterator<Item = (f64, f64, f64, f64)>) -> Option<(f64, f64, f64, f64)> {
    rects.fold(None, |acc, r| match acc {
        None => Some(r),
        Some(a) => Some((a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3))),
    })
}

fn swatch(color: [u8; 3]) -> Element<'static, Message> {
    container(iced::widget::Space::new())
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(iced::Color::from_rgb8(
                color[0], color[1], color[2],
            ))),
            border: iced::Border {
                color: iced::Color::from_rgb8(60, 60, 60),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.6)),
    }
}

/// Row text, greyed when `dim` -- an untagged symbol, or every row once the
/// drawing has moved on since the list was read.
fn row_text<'a>(s: impl Into<String>, size: f32, dim: bool) -> iced::widget::Text<'a> {
    let t = text(s.into()).size(size);
    if dim {
        t.style(muted)
    } else {
        t
    }
}

/// One clickable row. `indent` mimics a tree without one.
fn click_row<'a>(
    label: Element<'a, Message>,
    indent: f32,
    message: Message,
) -> Element<'a, Message> {
    mouse_area(
        container(label)
            .width(Fill)
            .padding(iced::Padding {
                top: 2.0,
                right: 6.0,
                bottom: 2.0,
                left: indent,
            }),
    )
    .interaction(iced::mouse::Interaction::Pointer)
    .on_press(message)
    .into()
}

/// A row whose click zooms the camera to `target`.
fn jump_row<'a>(
    label: Element<'a, Message>,
    target: ((f64, f64), (f64, f64)),
) -> Element<'a, Message> {
    click_row(
        label,
        18.0,
        Message::PidLegendJump {
            min: target.0,
            max: target.1,
        },
    )
}

/// Build the docked panel element from the active tab's recognition. `stale`
/// says the drawing has changed since the recognition was read.
pub fn view(
    recognition: Option<&Recognition>,
    stale: bool,
    width: f32,
    auto_collapse: bool,
) -> Element<'_, Message> {
    // ── Dock chrome (title, pin, close) — matches the block palette ───────
    let pin_icon = if auto_collapse {
        crate::ui::icons::themed_primary_weak_text(crate::ui::icons::PIN, 12.0)
    } else {
        crate::ui::icons::themed_secondary(crate::ui::icons::PIN, 12.0)
    };
    let pin = button(pin_icon)
        .on_press(Message::Dock(DockMsg::AutoCollapseToggle(PanelId::PidLegend)))
        .style(move |theme: &Theme, status| {
            let mut style = button::subtle(theme, status);
            if auto_collapse {
                let palette = theme.palette();
                style.background = Some(Background::Color(palette.primary.weak.color));
                style.text_color = palette.primary.weak.text;
                style.border.color = palette.primary.base.color;
                style.border.width = 1.0;
            }
            style
        })
        .padding([3, 5]);
    let pin = tooltip(pin, text("Auto").size(10), tooltip::Position::Bottom).gap(4);

    let close = button(crate::ui::icons::themed_secondary(crate::ui::icons::CLOSE, 12.0))
        .on_press(Message::Dock(DockMsg::Close(PanelId::PidLegend)))
        .style(button::subtle)
        .padding([3, 5]);
    let close = tooltip(close, text("Close").size(10), tooltip::Position::Bottom).gap(4);

    let mut title = row![text("P&ID 图例").size(12)].spacing(6).align_y(iced::Center);
    if stale && recognition.is_some() {
        title = title.push(text("已过期").size(10).style(muted));
    }
    let title_bar = mouse_area(
        container(
            row![
                title,
                iced::widget::Space::new().width(Fill),
                pin,
                close,
            ]
            .spacing(3)
            .align_y(iced::Center),
        )
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.palette().background.weak.color)),
            ..Default::default()
        })
        .width(Fill)
        .padding([3, 6]),
    )
    .on_press(Message::Dock(DockMsg::DockGrab(PanelId::PidLegend)))
    .interaction(iced::mouse::Interaction::Grab);

    let Some(rec) = recognition else {
        let hint = container(
            text("先运行 PIDLEGEND ON（或 REPORT / LIST）").size(12).style(muted),
        )
        .center_x(Fill)
        .center_y(Fill)
        .width(Fill)
        .height(Fill);
        return container(column![title_bar, hint].spacing(6).padding(6))
            .width(Length::Fixed(width))
            .height(Fill)
            .style(panel_bg)
            .into();
    };

    let mut col = column![].spacing(2);

    // Once the drawing has moved on, every row is greyed: it may point at an
    // entity that moved or went.
    if stale {
        col = col.push(
            container(
                text("图已改动，列表可能不准 — PIDLEGEND LIST 重新读取")
                    .size(11)
                    .style(muted),
            )
            .padding([4, 6]),
        );
    }

    // ── Symbols by class ──────────────────────────────────────────────────
    let groups = rec.by_class();
    let tagged: usize = rec.symbols.iter().filter(|s| s.tag.is_some()).count();
    col = col.push(
        container(row_text(
            format!("符号 {}（带位号 {}）", rec.symbols.len(), tagged),
            12.0,
            stale,
        ))
        .padding([4, 6]),
    );
    for ((_, label), symbols) in &groups {
        let color = symbols[0].color;
        let wants_tag = symbols[0].wants_tag;
        let with_tag = symbols.iter().filter(|s| s.tag.is_some()).count();
        let counts = if wants_tag {
            format!("×{}  位号 {}/{}", symbols.len(), with_tag, symbols.len())
        } else {
            format!("×{}", symbols.len())
        };
        col = col.push(
            container(
                row![
                    swatch(color),
                    row_text(label.clone(), 12.0, stale),
                    iced::widget::Space::new().width(Fill),
                    text(counts).size(11).style(muted),
                ]
                .spacing(6)
                .align_y(iced::Center),
            )
            .width(Fill)
            .padding([3, 6]),
        );
        for s in symbols {
            let name: String = match &s.tag {
                Some(tag) => tag.clone(),
                None => format!(
                    "({:.0}, {:.0}) mm",
                    s.at.0 / rec.units_per_mm,
                    s.at.1 / rec.units_per_mm
                ),
            };
            let label: Element<'_, Message> = row_text(name, 11.0, stale || s.tag.is_none()).into();
            col = col.push(jump_row(label, jump_rect(s.bbox, rec.units_per_mm)));
        }
    }

    // ── Pipe lines, grouped into families by the coding rule ──────────────
    // A family (service + sequence number) is one physical line however its
    // size or class changes along the way. Clicking a family (or a lone
    // line) selects every stroke of it in the drawing and zooms to it; a
    // line nested under a family header just zooms.
    let by_line = rec.pipes.by_line();
    let families = rec.pipes.by_family();
    let unnumbered: Vec<_> = rec
        .pipes
        .runs
        .iter()
        .filter(|r| r.lines.is_empty())
        .collect();
    if !families.is_empty() || !unnumbered.is_empty() {
        let header = if families.len() == by_line.len() {
            format!("管线 {} 条", by_line.len())
        } else {
            format!("管线 {} 条 · {} 族", by_line.len(), families.len())
        };
        col = col.push(
            container(row_text(header, 12.0, stale)).padding(iced::Padding {
                top: 10.0,
                right: 6.0,
                bottom: 4.0,
                left: 6.0,
            }),
        );
        for (family, lines) in &families {
            let run_count: usize = lines.values().map(|runs| runs.len()).sum();
            let length_mm: f64 = lines
                .values()
                .flat_map(|runs| runs.iter())
                .map(|r| r.length_mm)
                .sum();
            if let [(line, runs)] = lines.iter().collect::<Vec<_>>().as_slice() {
                // The family is one line: one row, worded as the line.
                let label: Element<'_, Message> = row![
                    row_text(line.to_string(), 11.0, stale),
                    iced::widget::Space::new().width(Fill),
                    text(format!("{} 段 · {:.0} mm", runs.len(), length_mm))
                        .size(10)
                        .style(muted),
                ]
                .spacing(6)
                .into();
                col = col.push(click_row(
                    label,
                    18.0,
                    Message::PidLegendPickFamily(family.clone()),
                ));
            } else {
                let label: Element<'_, Message> = row![
                    row_text(family.clone(), 11.0, stale),
                    iced::widget::Space::new().width(Fill),
                    text(format!(
                        "{} 线号 · {} 段 · {:.0} mm",
                        lines.len(),
                        run_count,
                        length_mm
                    ))
                    .size(10)
                    .style(muted),
                ]
                .spacing(6)
                .into();
                col = col.push(click_row(
                    label,
                    18.0,
                    Message::PidLegendPickFamily(family.clone()),
                ));
                for (line, runs) in lines {
                    let length_mm: f64 = runs.iter().map(|r| r.length_mm).sum();
                    if let Some(bbox) = union(runs.iter().filter_map(|r| r.bbox())) {
                        let label: Element<'_, Message> = row![
                            row_text(line.to_string(), 11.0, stale),
                            iced::widget::Space::new().width(Fill),
                            text(format!("{} 段 · {:.0} mm", runs.len(), length_mm))
                                .size(10)
                                .style(muted),
                        ]
                        .spacing(6)
                        .into();
                        col = col.push(click_row(
                            label,
                            30.0,
                            Message::PidLegendJump {
                                min: jump_rect(bbox, rec.units_per_mm).0,
                                max: jump_rect(bbox, rec.units_per_mm).1,
                            },
                        ));
                    }
                }
            }
        }
        if !unnumbered.is_empty() {
            if let Some(bbox) = union(unnumbered.iter().filter_map(|r| r.bbox())) {
                let label: Element<'_, Message> = row![
                    text("（未编号）").size(11).style(muted),
                    iced::widget::Space::new().width(Fill),
                    text(format!("{} 段", unnumbered.len())).size(10).style(muted),
                ]
                .spacing(6)
                .into();
                col = col.push(jump_row(label, jump_rect(bbox, rec.units_per_mm)));
            }
        }
    }

    let body = scrollable(container(col).padding(iced::Padding {
        top: 0.0,
        right: 8.0,
        bottom: 6.0,
        left: 0.0,
    }))
    .width(Fill)
    .height(Fill);

    container(column![title_bar, body].spacing(6).padding(6))
        .width(Length::Fixed(width))
        .height(Fill)
        .style(panel_bg)
        .into()
}

fn panel_bg(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme.palette().background.base.color)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The zoom target keeps a readable margin: never tighter than 4 paper-mm
    /// a side, proportional (120 % of the bigger side) once the target grows.
    #[test]
    fn jump_rect_pads_small_symbols_to_a_readable_window() {
        // A 3×2 drawing-unit valve at 1 unit/mm: pad = 4 mm floor.
        let ((x0, y0), (x1, y1)) = jump_rect((10.0, 20.0, 13.0, 22.0), 1.0);
        assert_eq!((x0, y0), (6.0, 16.0));
        assert_eq!((x1, y1), (17.0, 26.0));

        // A 100-unit pipe run: pad grows with the run (120 mm), floor unused.
        let ((x0, _), (x1, _)) = jump_rect((0.0, 0.0, 100.0, 10.0), 1.0);
        assert_eq!(x0, -120.0);
        assert_eq!(x1, 220.0);
    }
}

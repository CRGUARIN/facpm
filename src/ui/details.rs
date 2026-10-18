//! Right panel: Details (warnings first), Cleanup details and messages.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};

use crate::app::{CleanRow, RightTab, State, Tab};
use crate::backend::repo::human_size;
use crate::backend::warnings::{Severity, Warning};
use crate::pkg::Details;
use crate::theme::Theme;

use super::pkgbuild::draw_pkgbuild;
use super::{fit, labeled_lines, signature, tab_span, wrap};

pub(super) fn draw_right(frame: &mut Frame, area: Rect, st: &mut State) {
    let t = st.theme;
    let title = Line::from(vec![
        Span::raw(" "),
        tab_span(t, "Details", st.right_tab == RightTab::Details),
        Span::raw(" │ "),
        tab_span(t, "PKGBUILD", st.right_tab == RightTab::Pkgbuild),
        Span::raw(" "),
    ]);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.border))
        .title(title);
    let block = match signature(t, area.width) {
        Some(sig) => block.title_bottom(sig),
        None => block,
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);

    st.right_height = inner.height;
    match (st.tab, st.right_tab) {
        (Tab::Clean, RightTab::Pkgbuild) => draw_message(
            frame,
            inner,
            Line::from("Not available in Cleanup").style(Style::new().fg(t.muted)),
        ),
        (Tab::Clean, RightTab::Details) => draw_clean_details(frame, inner, st),
        (_, RightTab::Details) => draw_details(frame, inner, st),
        (_, RightTab::Pkgbuild) => draw_pkgbuild(frame, inner, st),
    }
}

fn draw_details(frame: &mut Frame, area: Rect, st: &mut State) {
    let t = st.theme;
    let width = area.width as usize;
    // Warnings for the selected package come first, then its data.
    let mut lines = Vec::new();
    if matches!(st.tab, Tab::Installed | Tab::Updates)
        && let Some(ws) = st.selected().and_then(|pkg| st.warnings.get(&pkg.name))
    {
        lines = warning_lines(t, ws, width);
        lines.push(Line::raw(""));
    }
    lines.extend(details_lines(t, &st.details, width));
    let max_scroll = lines.len().saturating_sub(area.height as usize) as u16;
    st.right_scroll = st.right_scroll.min(max_scroll);
    frame.render_widget(Paragraph::new(lines).scroll((st.right_scroll, 0)), area);
}

/// Key/value rows with a hanging indent: every line of a value stays aligned in its column.
fn details_lines(t: &Theme, details: &Details, width: usize) -> Vec<Line<'static>> {
    if details.is_empty() {
        return vec![Line::from("No package selected").style(Style::new().fg(t.muted))];
    }
    let longest_key = details
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let key_w = (longest_key + 2).min(width / 2).max(1);
    let val_w = width.saturating_sub(key_w).max(1);

    let mut lines = Vec::new();
    for (key, value) in details {
        let value = if value.trim().is_empty() {
            "—"
        } else {
            value.as_str()
        };
        for (i, part) in wrap(value, val_w).into_iter().enumerate() {
            let head = if i == 0 {
                let key = format!("{} ", fit(key, key_w - 1));
                Span::styled(key, Style::new().fg(t.cyan).bold())
            } else {
                Span::raw(" ".repeat(key_w))
            };
            lines.push(Line::from(vec![head, Span::raw(part)]));
        }
    }
    lines
}

fn warning_lines(t: &Theme, ws: &[Warning], width: usize) -> Vec<Line<'static>> {
    // Fits "⚠ Warning" plus two spaces.
    const KEY_W: usize = 11;
    let val_w = width.saturating_sub(KEY_W).max(1);
    let mut lines = Vec::new();
    for w in ws {
        let (key, color) = match w.severity {
            Severity::Alarm => ("⚠ Alert", t.red),
            Severity::Warn => ("⚠ Warning", t.yellow),
            Severity::Info => ("Note", t.muted),
        };
        for (i, part) in wrap(&w.text, val_w).into_iter().enumerate() {
            let head = if i == 0 {
                Span::styled(fit(key, KEY_W), Style::new().fg(color).bold())
            } else {
                Span::raw(" ".repeat(KEY_W))
            };
            lines.push(Line::from(vec![head, Span::raw(part)]));
        }
    }
    lines
}

fn draw_clean_details(frame: &mut Frame, area: Rect, st: &mut State) {
    let t = st.theme;
    let lines = match st.selected_clean() {
        Some(row) => clean_detail_lines(t, row, &st.orphans, area.width as usize),
        None => vec![Line::from("No item selected").style(Style::new().fg(t.muted))],
    };
    let max_scroll = lines.len().saturating_sub(area.height as usize) as u16;
    st.right_scroll = st.right_scroll.min(max_scroll);
    frame.render_widget(Paragraph::new(lines).scroll((st.right_scroll, 0)), area);
}

fn clean_detail_lines(
    t: &Theme,
    row: &CleanRow,
    orphans: &[(String, u64)],
    width: usize,
) -> Vec<Line<'static>> {
    let task = &row.task;
    let mut lines: Vec<Line<'static>> = vec![Line::from(task.title.clone()).bold()];
    lines.extend(wrap(&task.description, width).into_iter().map(Line::from));
    if let Some(note) = &task.note {
        lines.extend(
            wrap(note, width)
                .into_iter()
                .map(|l| Line::from(l).style(Style::new().fg(t.yellow))),
        );
    }
    lines.push(Line::raw(""));
    let size = task.bytes.map_or_else(|| "unknown".to_string(), human_size);
    lines.push(Line::from(format!("Candidates: {}", task.count)));
    lines.push(Line::from(format!("Size: {size}")));
    if let Some(command) = &row.command {
        lines.extend(labeled_lines("Command: ", command, width));
    }
    if !task.items.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from("Items:").bold());
        for item in &task.items {
            let text = match orphans.iter().find(|(name, _)| name == item) {
                Some((_, bytes)) => format!("{item}  {}", human_size(*bytes)),
                None => item.clone(),
            };
            lines.extend(wrap(&text, width).into_iter().map(Line::from));
        }
    }
    lines
}

pub(super) fn draw_message(frame: &mut Frame, area: Rect, line: Line<'_>) {
    frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), area);
}

#[cfg(test)]
mod tests {
    use crate::app::RightTab;
    use crate::backend::warnings::{Kind, Severity};
    use crate::ui::test_util::{
        TALL, WIDE, clean_state, render_at, test_warning, two_packages_state,
    };

    fn row_of(rows: &[String], text: &str) -> usize {
        rows.iter()
            .position(|r| r.contains(text))
            .unwrap_or_else(|| panic!("missing {text:?}"))
    }

    #[test]
    fn warnings_come_before_the_package_data() {
        let mut st = two_packages_state();
        st.warnings.insert(
            "firefox".to_string(),
            vec![test_warning(
                Kind::MaintainerChanged,
                Severity::Alarm,
                "maintainer changed",
            )],
        );
        st.details = vec![("Name".to_string(), "firefox".to_string())];
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(row_of(&rows, "⚠ Alert") < row_of(&rows, "Name"));
    }

    #[test]
    fn details_scroll_is_clamped_to_the_content() {
        let mut st = two_packages_state();
        st.details = vec![("Name".to_string(), "firefox".to_string())];
        st.right_scroll = 500;
        render_at(&mut st, WIDE, TALL);
        assert_eq!(st.right_scroll, 0);
    }

    #[test]
    fn multiline_values_keep_their_continuation_aligned() {
        let mut st = two_packages_state();
        st.details = vec![(
            "Optional dependencies".to_string(),
            "alpha: first\nbeta: second".to_string(),
        )];
        let rows = render_at(&mut st, WIDE, TALL);
        let first = row_of(&rows, "alpha:");
        let second = row_of(&rows, "beta:");
        assert_eq!(second, first + 1);
        assert_eq!(rows[first].find("alpha:"), rows[second].find("beta:"));
    }

    #[test]
    fn long_keys_keep_a_space_before_the_value_on_narrow_panels() {
        let mut st = two_packages_state();
        st.details = vec![("Optional dependencies".to_string(), "alpha".to_string())];
        let rows = render_at(&mut st, 80, TALL);
        let row = &rows[row_of(&rows, "alpha")];
        assert!(row.contains("… alpha"), "{row}");
    }

    #[test]
    fn clean_details_list_the_items_with_their_sizes() {
        let mut st = clean_state();
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(rows.iter().any(|r| r.contains("Items:")));
        assert!(rows.iter().any(|r| r.contains("go  226.6 MiB")));
    }

    #[test]
    fn clean_pkgbuild_tab_says_it_does_not_apply() {
        let mut st = clean_state();
        st.right_tab = RightTab::Pkgbuild;
        let rows = render_at(&mut st, WIDE, TALL).join("\n");
        assert!(rows.contains("Not available in Cleanup"));
    }
}

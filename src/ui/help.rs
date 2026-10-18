//! Help (`?`) and Arch news (`n`) popups. Both have the same size and scrolling.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::app::{NewsState, State};
use crate::backend::news;
use crate::theme::Theme;

use super::{centered, line_width, wrap};

const HELP_WIDTH_MAX: u16 = 110;
const HELP_TWO_COLUMNS_MIN: usize = 96;
const HELP_COL_GAP: usize = 4;
/// Groups in the left column when there are two; the rest go to the right.
const HELP_LEFT_GROUPS: usize = 3;

struct HelpGroup {
    title: &'static str,
    keys: &'static [(&'static str, &'static str)],
    notes: &'static [&'static str],
}

const HELP_GROUPS: &[HelpGroup] = &[
    HelpGroup {
        title: "List (left panel)",
        keys: &[
            ("j/k ↓/↑", "Move the selection"),
            ("g/G", "Go to the first / last"),
            ("PgUp/PgDn", "Page up / down"),
            ("1 2 3 4", "Installed · Search · Updates · Cleanup"),
            ("h/l ←/→", "Previous / next tab"),
        ],
        notes: &[],
    },
    HelpGroup {
        title: "Right panel",
        keys: &[
            ("p", "Toggle Details and PKGBUILD"),
            ("J/K", "Scroll one line (Shift + j/k)"),
            ("Ctrl-d/Ctrl-u", "Scroll half a page"),
        ],
        notes: &[
            "Scrolling resets when you select another package. Long PKGBUILD lines are cut off.",
        ],
    },
    HelpGroup {
        title: "Typing (filter / search)",
        keys: &[
            (
                "/",
                "Start typing (Search also starts typing when you press 2)",
            ),
            ("Enter", "Finish and keep the text"),
            ("Esc", "Leave typing mode"),
            ("Ctrl-u", "Clear the text"),
        ],
        notes: &["While typing, keys go to the text: press Enter or Esc to use the shortcuts."],
    },
    HelpGroup {
        title: "Packages",
        keys: &[
            ("space", "Mark / unmark (to act on several)"),
            ("i", "Install the marked (or selected)"),
            ("u/x", "Uninstall the marked (or selected)"),
            ("U", "Upgrade the whole system"),
            ("r", "Reload the database / check again"),
        ],
        notes: &[],
    },
    HelpGroup {
        title: "Cleanup (tab 4)",
        keys: &[("c/Enter", "Clean the selected task")],
        notes: &[],
    },
    HelpGroup {
        title: "General",
        keys: &[
            ("t", "Change the color theme"),
            ("n", "Arch Linux news"),
            ("?", "Show / hide this help"),
            ("q", "Quit"),
        ],
        notes: &[],
    },
];

pub(super) fn draw_help(frame: &mut Frame, screen: Rect, st: &mut State) {
    let t = st.theme;
    let width = HELP_WIDTH_MAX.min(screen.width.saturating_sub(4));
    let inner_w = width.saturating_sub(2) as usize;
    let rows = help_rows(t, inner_w);
    let height = (rows.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let inner_h = height.saturating_sub(2);
    st.help_height = inner_h;
    let max_scroll = rows.len().saturating_sub(inner_h as usize) as u16;
    st.help_scroll = st.help_scroll.min(max_scroll);
    let rect = centered(screen, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.accent))
        .style(Style::default().bg(t.surface).fg(t.fg))
        .title(Line::from(format!(" Help · facpm v{} ", env!("CARGO_PKG_VERSION"))).bold())
        .title_bottom(Line::from(" ?/Esc: close ").right_aligned());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(rows)
            .scroll((st.help_scroll, 0))
            .block(block),
        rect,
    );
}

/// Help rows for an inner width: two columns if they fit, one otherwise.
fn help_rows(t: &Theme, inner_w: usize) -> Vec<Line<'static>> {
    if inner_w < HELP_TWO_COLUMNS_MIN {
        return help_column(t, HELP_GROUPS, inner_w);
    }
    let col_w = (inner_w - HELP_COL_GAP) / 2;
    let left = help_column(t, &HELP_GROUPS[..HELP_LEFT_GROUPS], col_w);
    let right = help_column(t, &HELP_GROUPS[HELP_LEFT_GROUPS..], col_w);
    zip_columns(&left, &right, col_w)
}

/// One column: each group with its title, its shortcuts (key in a chip) and its italic notes.
fn help_column(t: &Theme, groups: &[HelpGroup], width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::styled(
            group.title,
            Style::new().bold().fg(t.accent),
        )));
        let chip_w = group
            .keys
            .iter()
            .map(|(keys, _)| keys.chars().count() + 2)
            .max()
            .unwrap_or(0);
        let desc_x = chip_w + 2;
        let desc_w = width.saturating_sub(desc_x).max(1);
        for (keys, desc) in group.keys {
            let chip = format!(" {keys} ");
            let pad = " ".repeat(desc_x - chip.chars().count());
            for (j, part) in wrap(desc, desc_w).into_iter().enumerate() {
                if j == 0 {
                    lines.push(Line::from(vec![
                        Span::styled(chip.clone(), Style::new().fg(t.on_accent).bg(t.cyan)),
                        Span::raw(pad.clone()),
                        Span::raw(part),
                    ]));
                } else {
                    lines.push(Line::raw(format!("{}{part}", " ".repeat(desc_x))));
                }
            }
        }
        for note in group.notes {
            for part in wrap(&format!("note: {note}"), desc_w) {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(desc_x)),
                    Span::styled(part, Style::new().fg(t.muted).italic()),
                ]));
            }
        }
    }
    lines
}

fn zip_columns(
    left: &[Line<'static>],
    right: &[Line<'static>],
    col_w: usize,
) -> Vec<Line<'static>> {
    let rows = left.len().max(right.len());
    (0..rows)
        .map(|i| {
            let mut spans = Vec::new();
            let used = left.get(i).map_or(0, line_width);
            if let Some(line) = left.get(i) {
                spans.extend(line.spans.iter().cloned());
            }
            spans.push(Span::raw(
                " ".repeat(col_w.saturating_sub(used) + HELP_COL_GAP),
            ));
            if let Some(line) = right.get(i) {
                spans.extend(line.spans.iter().cloned());
            }
            Line::from(spans)
        })
        .collect()
}

/// Arch news popup: the same size and scrolling as the help.
pub(super) fn draw_news(frame: &mut Frame, screen: Rect, st: &mut State) {
    let t = st.theme;
    let width = HELP_WIDTH_MAX.min(screen.width.saturating_sub(4));
    let inner_w = width.saturating_sub(2) as usize;
    let rows = news_rows(t, &st.news, inner_w);
    let height = (rows.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let inner_h = height.saturating_sub(2);
    st.help_height = inner_h;
    let max_scroll = rows.len().saturating_sub(inner_h as usize) as u16;
    st.help_scroll = st.help_scroll.min(max_scroll);
    let rect = centered(screen, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.accent))
        .style(Style::default().bg(t.surface).fg(t.fg))
        .title(Line::from(" Arch Linux news ").bold())
        .title_bottom(Line::from(" n/Esc: close ").right_aligned());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(rows)
            .scroll((st.help_scroll, 0))
            .block(block),
        rect,
    );
}

/// News rows, newest first, with date, title, body and link.
fn news_rows(t: &Theme, news: &NewsState, width: usize) -> Vec<Line<'static>> {
    let muted = Style::new().fg(t.muted);
    let NewsState::Loaded { items, since } = news else {
        let text = match news {
            NewsState::Error(_) => "couldn't be downloaded",
            _ => "loading…",
        };
        return vec![Line::from(text).style(muted)];
    };
    if items.is_empty() {
        return vec![Line::from("No Arch news.").style(muted)];
    }
    let mut rows = Vec::new();
    for item in items {
        let new = item.published > *since;
        let intervention = news::is_intervention(&item.title);
        let marker = if new {
            Span::styled("● ", Style::new().fg(t.accent))
        } else {
            Span::raw("  ")
        };
        let title_style = if new && intervention {
            Style::new().fg(t.red).bold()
        } else {
            Style::new().bold()
        };
        // "● " + date + "  ": the title comes after and its lines continue under it.
        let date_w = item.date.chars().count() + 4;
        let title_w = width.saturating_sub(date_w).max(1);
        for (i, part) in wrap(&item.title, title_w).into_iter().enumerate() {
            if i == 0 {
                rows.push(Line::from(vec![
                    marker.clone(),
                    Span::styled(item.date.clone(), muted),
                    Span::raw("  "),
                    Span::styled(part, title_style),
                ]));
            } else {
                rows.push(Line::from(vec![
                    Span::raw(" ".repeat(date_w)),
                    Span::styled(part, title_style),
                ]));
            }
        }
        // Bullets `• ` continue with an indent of 4, under their text.
        for logical in item.body.split('\n') {
            if let Some(rest) = logical.trim_start().strip_prefix("• ") {
                let parts = wrap(rest, width.saturating_sub(4).max(1));
                for (i, part) in parts.into_iter().enumerate() {
                    let lead = if i == 0 { "  • " } else { "    " };
                    rows.push(Line::from(format!("{lead}{part}")));
                }
            } else {
                for part in wrap(logical, width.saturating_sub(2).max(1)) {
                    rows.push(Line::from(format!("  {part}")));
                }
            }
        }
        // The link is muted; if it does not fit, it is split over consecutive lines, never cut.
        for part in wrap(&item.link, width.saturating_sub(2).max(1)) {
            rows.push(Line::from(format!("  {part}")).style(muted));
        }
        rows.push(Line::raw(""));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Modal, NewsState, State};
    use crate::ui::test_util::{
        NEWS_AFTER, TALL, WIDE, news_item, news_loaded, render_at, two_packages_state,
        updates_state,
    };

    fn help_state() -> State {
        let mut st = two_packages_state();
        st.modal = Some(Modal::Help);
        st
    }

    #[test]
    fn help_has_two_columns_from_96_inner_columns() {
        let mut st = help_state();
        let rows = render_at(&mut st, WIDE, 40);
        assert!(
            rows.iter()
                .any(|r| r.contains("List (left panel)") && r.contains("Packages"))
        );
    }

    #[test]
    fn help_has_one_column_at_80_columns() {
        let mut st = help_state();
        let rows = render_at(&mut st, 80, 24);
        assert!(
            !rows
                .iter()
                .any(|r| r.contains("List (left panel)") && r.contains("Packages"))
        );
    }

    #[test]
    fn help_scroll_is_clamped_to_the_content() {
        let mut st = help_state();
        st.help_scroll = 500;
        render_at(&mut st, 80, 24);
        let total = help_rows(st.theme, 74).len();
        assert_eq!(st.help_scroll as usize, total - st.help_height as usize);
    }

    #[test]
    fn news_modal_says_when_it_is_loading_or_failed() {
        let mut st = updates_state();
        st.modal = Some(Modal::News);
        st.news = NewsState::Loading;
        assert!(
            render_at(&mut st, WIDE, TALL)
                .join("\n")
                .contains("loading…")
        );
        st.news = NewsState::Error("x".to_string());
        assert!(
            render_at(&mut st, WIDE, TALL)
                .join("\n")
                .contains("couldn't be downloaded")
        );
    }

    #[test]
    fn long_news_link_wraps_on_consecutive_lines() {
        let link =
            "https://archlinux.org/news/a-long-link-that-does-not-fit-on-one-eighty-column-line/";
        let mut item = news_item("Titulo", NEWS_AFTER);
        item.link = link.to_string();
        let mut st = updates_state();
        st.modal = Some(Modal::News);
        st.news = news_loaded(vec![item]);
        let rows = render_at(&mut st, 80, 24);
        let first = rows
            .iter()
            .position(|r| r.contains(&link[..72]))
            .expect("first part of the link");
        assert!(rows[first + 1].contains(&link[72..]));
    }

    #[test]
    fn bullet_continuations_indent_under_the_text() {
        let mut item = news_item("Titulo", NEWS_AFTER);
        item.body =
            "• first point with enough words for the bullet to wrap over several lines zzfin"
                .to_string();
        let mut st = updates_state();
        st.modal = Some(Modal::News);
        st.news = news_loaded(vec![item]);
        let rows = render_at(&mut st, 80, 24);
        let first = rows
            .iter()
            .position(|r| r.contains("• first"))
            .expect("bullet");
        let text_col = rows[first].find("first").expect("bullet text");
        let last = rows
            .iter()
            .position(|r| r.contains("zzfin"))
            .expect("last part");
        assert!(last > first);
        assert_eq!(rows[last].find("zzfin").map(|c| c >= text_col), Some(true));
    }
}

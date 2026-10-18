//! Left panel: the list for the active tab, with its filter or search and its title.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, Paragraph};

use crate::app::{AurState, CleanRow, NewsState, State, Tab};
use crate::backend::clean::CleanKind;
use crate::backend::news;
use crate::backend::repo::human_size;
use crate::backend::warnings::{self, Kind, Severity};
use crate::pkg::Package;
use crate::theme::Theme;

use super::{fit, row_secondary, source_color};

pub(super) fn draw_list(frame: &mut Frame, area: Rect, st: &mut State) {
    let t = st.theme;
    let title = match st.tab {
        Tab::Installed => " Installed ".to_string(),
        Tab::Search => format!(" Search · {} ", aur_status(&st.aur_state)),
        Tab::Updates => format!(" Updates {} ", updates_status(st)),
        Tab::Clean => format!(" Cleanup {} ", clean_status(st)),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.accent))
        .title(Line::from(title).bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // In Updates the header takes one to three lines; the list starts below it.
    let header_h = if st.tab == Tab::Updates {
        updates_header_lines(st, inner.width as usize).len() as u16
    } else {
        1
    };
    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(header_h), Constraint::Fill(1)]).areas(inner);
    draw_input(frame, input_area, st);

    let rows = list_lines(st, list_area.width as usize);
    if rows.is_empty() {
        let msg = empty_message(st);
        frame.render_widget(
            Paragraph::new(Line::from(msg).style(Style::new().fg(t.muted))),
            list_area,
        );
        return;
    }
    let items: Vec<ListItem> = rows.into_iter().map(ListItem::new).collect();
    let list = List::new(items).highlight_style(Style::new().bg(t.selection).bold());
    let idx = st.tab.idx();
    frame.render_stateful_widget(list, list_area, &mut st.lists[idx]);
}

/// Part of the Updates title: "· error", "· checking…", "· up to date" or
/// "· N available".
fn updates_status(st: &State) -> String {
    if st.updates_error {
        "· error".to_string()
    } else if st.updates_pending {
        "· checking…".to_string()
    } else if st.updates.is_empty() {
        "· up to date".to_string()
    } else {
        format!("· {} available", st.updates.len())
    }
}

fn aur_status(state: &AurState) -> String {
    match state {
        AurState::Idle => "AUR: idle".to_string(),
        AurState::Loading => "AUR: searching…".to_string(),
        AurState::Done { count, local } => {
            let source = if *local { "local" } else { "API" };
            if *count == 1 {
                format!("AUR: 1 result ({source})")
            } else {
                format!("AUR: {count} results ({source})")
            }
        }
        AurState::Error(_) => "AUR: error".to_string(),
    }
}

fn draw_input(frame: &mut Frame, area: Rect, st: &State) {
    let t = st.theme;
    if st.tab == Tab::Updates {
        frame.render_widget(
            Paragraph::new(updates_header_lines(st, area.width as usize)),
            area,
        );
        return;
    }
    if st.tab == Tab::Clean {
        frame.render_widget(
            Paragraph::new(
                Line::from("c/Enter: clean the selected task").style(Style::new().fg(t.muted)),
            ),
            area,
        );
        return;
    }
    let (prefix, text) = match st.tab {
        Tab::Search => ("Search: ", st.query.as_str()),
        _ => ("/ ", st.filter.as_str()),
    };
    let line = if text.is_empty() && !st.input_mode {
        let hint = match st.tab {
            Tab::Search => "press / to search",
            _ => "press / to filter",
        };
        Line::from(Span::styled(hint, Style::new().fg(t.muted)))
    } else {
        Line::from(vec![
            Span::styled(prefix, Style::new().fg(t.cyan).bold()),
            Span::raw(text),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
    if st.input_mode {
        let cursor_x = area.x + (prefix.chars().count() + text.chars().count()) as u16;
        frame.set_cursor_position((cursor_x.min(area.right().saturating_sub(1)), area.y));
    }
}

fn empty_message(st: &State) -> &'static str {
    match st.tab {
        Tab::Installed if st.installed.is_empty() => "No installed packages",
        Tab::Installed => "No packages match the filter",
        Tab::Search if st.query.trim().is_empty() => "Type to search the repos and the AUR",
        Tab::Search if matches!(st.aur_state, AurState::Loading) => "Searching the AUR…",
        Tab::Search => "No results",
        Tab::Clean if st.clean_pending => "Scanning…",
        Tab::Clean => "Nothing to clean",
        Tab::Updates if st.updates_pending => "Checking for updates…",
        Tab::Updates if st.updates_error => "Couldn't check for updates",
        Tab::Updates => "Your system is up to date",
    }
}

/// Widths (name, version): the version takes what its rows ask for, within [8, 20] and up to 2/5.
fn column_widths(rows: &[&Package], width: usize) -> (usize, usize) {
    let longest_version = rows
        .iter()
        .map(|p| p.version.chars().count())
        .max()
        .unwrap_or(0);
    let label_w = rows
        .iter()
        .map(|p| p.source.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(4);
    // mark (2) + indicator (2) + space + 2 spaces before the source + source
    let available = width.saturating_sub(2 + 2 + 1 + 2 + label_w);
    let version_w = longest_version.clamp(8, 20).min((available * 2 / 5).max(6));
    let name_w = available.saturating_sub(version_w).max(1);
    (name_w, version_w)
}

fn list_lines(st: &State, width: usize) -> Vec<Line<'static>> {
    let t = st.theme;
    if st.tab == Tab::Clean {
        return clean_lines(st, width);
    }
    if st.tab == Tab::Updates {
        return update_lines(st, width);
    }
    let rows = st.rows();
    let (name_w, version_w) = column_widths(&rows, width.saturating_sub(2));
    let marked = &st.marked[st.tab.idx()];

    rows.iter()
        .enumerate()
        .map(|(i, p)| {
            let selected = st.sel() == Some(i);
            let mark = if marked.contains(&p.name) {
                Span::styled("● ", Style::new().fg(t.magenta))
            } else {
                Span::raw("  ")
            };
            Line::from(vec![
                mark,
                indicator(t, st.tab, p, st.upgradable.contains(&p.name)),
                warning_mark(st, &p.name),
                Span::raw(format!("{} ", fit(&p.name, name_w))),
                Span::styled(
                    format!("{}  ", fit(&p.version, version_w)),
                    Style::new().fg(row_secondary(t, selected)),
                ),
                Span::styled(
                    p.source.label().to_string(),
                    Style::new().fg(source_color(t, &p.source, selected)),
                ),
            ])
        })
        .collect()
}

/// In Search: ✓ if the installed version is the available one, ↑ if it differs; in Installed, ↑ if there is an update.
fn indicator(t: &Theme, tab: Tab, p: &Package, upgradable: bool) -> Span<'static> {
    match (tab, &p.installed) {
        (Tab::Search, Some(v)) if *v == p.version => Span::styled("✓ ", Style::new().fg(t.green)),
        (Tab::Search, Some(_)) => Span::styled("↑ ", Style::new().fg(t.yellow)),
        (Tab::Installed, _) if upgradable => Span::styled("↑ ", Style::new().fg(t.yellow)),
        _ => Span::raw("  "),
    }
}

fn update_lines(st: &State, width: usize) -> Vec<Line<'static>> {
    let t = st.theme;
    let rows = st.rows();
    let label_w = rows
        .iter()
        .map(|p| p.source.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(4);
    // mark (2) + indicator (2) + warning (2) + space + 2 spaces before the source + source
    let available = width.saturating_sub(2 + 2 + 2 + 1 + 2 + label_w);
    // What the longest row asks for, capped; on a narrow panel at least "→ new" fits.
    let longest = rows.iter().map(|p| change_width(p)).max().unwrap_or(0);
    let longest_arrow = rows
        .iter()
        .map(|p| 2 + p.version.chars().count())
        .max()
        .unwrap_or(0);
    let cap = (available * 3 / 5).max(longest_arrow.min(available)).max(8);
    let change_w = longest.clamp(8, 40).min(cap);
    let name_w = available.saturating_sub(change_w).max(1);
    let marked = &st.marked[st.tab.idx()];

    rows.iter()
        .enumerate()
        .map(|(i, p)| {
            let selected = st.sel() == Some(i);
            let mark = if marked.contains(&p.name) {
                Span::styled("● ", Style::new().fg(t.magenta))
            } else {
                Span::raw("  ")
            };
            let mut spans = vec![
                mark,
                Span::raw("  "),
                warning_mark(st, &p.name),
                Span::raw(format!("{} ", fit(&p.name, name_w))),
            ];
            spans.extend(change_spans(t, p, change_w, selected));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                p.source.label().to_string(),
                Style::new().fg(source_color(t, &p.source, selected)),
            ));
            Line::from(spans)
        })
        .collect()
}

fn change_width(p: &Package) -> usize {
    p.installed.as_deref().unwrap_or("?").chars().count() + 3 + p.version.chars().count()
}

/// Versions in `w` columns: "current → new", then "→ new", then the new version cut.
fn change_spans(t: &Theme, p: &Package, w: usize, selected: bool) -> Vec<Span<'static>> {
    let current = p.installed.as_deref().unwrap_or("?");
    let new = p.version.as_str();
    let secondary = Style::new().fg(row_secondary(t, selected));
    let green = Style::new().fg(t.green);
    let full = change_width(p);
    if full <= w {
        return vec![
            Span::styled(current.to_string(), secondary),
            Span::styled(" → ", secondary),
            Span::styled(new.to_string(), green),
            Span::raw(" ".repeat(w - full)),
        ];
    }
    let arrow = 2 + new.chars().count();
    if arrow <= w {
        return vec![
            Span::styled("→ ", secondary),
            Span::styled(new.to_string(), green),
            Span::raw(" ".repeat(w - arrow)),
        ];
    }
    vec![Span::styled(fit(new, w), green)]
}

/// Warning mark: `⚠ ` in red or yellow, or two spaces; there is no mark in Search.
fn warning_mark(st: &State, name: &str) -> Span<'static> {
    let t = st.theme;
    if st.tab == Tab::Search {
        return Span::raw("  ");
    }
    match st.warnings.get(name).and_then(|ws| warnings::worst(ws)) {
        Some(Severity::Alarm) => Span::styled("⚠ ", Style::new().fg(t.red)),
        Some(Severity::Warn) => Span::styled("⚠ ", Style::new().fg(t.yellow)),
        _ => Span::raw("  "),
    }
}

fn clean_status(st: &State) -> String {
    if st.clean_pending {
        return "· scanning…".to_string();
    }
    let any_other = st
        .clean
        .iter()
        .any(|r| r.task.available && r.task.kind != CleanKind::PkgCacheAll);
    if !any_other {
        return "· nothing to clean".to_string();
    }
    format!(
        "· ~{} reclaimable",
        human_size(reclaimable_bytes(&st.clean))
    )
}

/// Reclaimable bytes: the available tasks except `PkgCacheAll`, which overlaps with the others.
fn reclaimable_bytes(rows: &[CleanRow]) -> u64 {
    rows.iter()
        .filter(|r| r.task.available && r.task.kind != CleanKind::PkgCacheAll)
        .filter_map(|r| r.task.bytes)
        .sum()
}

/// Cleanup rows: the title on the left, count and size on the right.
fn clean_lines(st: &State, width: usize) -> Vec<Line<'static>> {
    const RIGHT_W: usize = 18;
    let title_w = width.saturating_sub(2 + RIGHT_W).max(1);
    st.clean
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let task = &row.task;
            let size = task.bytes.map_or_else(|| "?".to_string(), human_size);
            let style = if task.available {
                Style::new()
            } else {
                Style::new().fg(row_secondary(st.theme, st.sel() == Some(i)))
            };
            Line::from(vec![
                Span::styled(format!("  {}", fit(&task.title, title_w)), style),
                Span::styled(format!(" {:>5}  {:>10}", task.count, size), style),
            ])
        })
        .collect()
}

/// Updates header: the parts are joined with " · " and spread over up to three lines.
fn updates_header_lines(st: &State, width: usize) -> Vec<Line<'static>> {
    let t = st.theme;
    let budget = width.saturating_sub(1).max(1);
    let sep = " · ";
    let sep_w = sep.chars().count();
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0;
    for (text, color) in updates_header_parts(st) {
        let text = if text.chars().count() > budget {
            fit(&text, budget)
        } else {
            text
        };
        let w = text.chars().count();
        let part = Span::styled(text, Style::new().fg(color));
        if cur.is_empty() {
            cur.push(part);
            cur_w = w;
        } else if cur_w + sep_w + w <= budget {
            cur.push(Span::styled(sep, Style::new().fg(t.muted)));
            cur.push(part);
            cur_w += sep_w + w;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push(part);
            cur_w = w;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
        .into_iter()
        .map(|spans| {
            let mut line = vec![Span::raw(" ")];
            line.extend(spans);
            Line::from(line)
        })
        .collect()
}

/// Header parts: news, installed packages outside the AUR, and updates with warnings.
fn updates_header_parts(st: &State) -> Vec<(String, Color)> {
    let t = st.theme;
    let mut parts = vec![news_header_part(st)];
    let missing = st
        .warnings
        .values()
        .filter(|ws| ws.iter().any(|w| w.kind == Kind::Missing))
        .count();
    if missing > 0 {
        parts.push((format!("{missing} not in the AUR"), t.yellow));
    }
    let flagged: Vec<Severity> = st
        .updates
        .iter()
        .filter_map(|p| st.warnings.get(&p.name).and_then(|ws| warnings::worst(ws)))
        .filter(|severity| *severity != Severity::Info)
        .collect();
    if !flagged.is_empty() {
        let color = if flagged.contains(&Severity::Alarm) {
            t.red
        } else {
            t.yellow
        };
        parts.push((format!("{} with warnings", flagged.len()), color));
    }
    parts
}

fn news_header_part(st: &State) -> (String, Color) {
    let t = st.theme;
    match &st.news {
        NewsState::Loading => ("Arch news: loading…".to_string(), t.muted),
        NewsState::Error(_) => ("Arch news: offline".to_string(), t.muted),
        NewsState::Loaded { .. } => {
            let new = st.news.new_items();
            if new.is_empty() {
                ("Arch news: nothing new (n)".to_string(), t.muted)
            } else {
                let color = if new.iter().any(|item| news::is_intervention(&item.title)) {
                    t.red
                } else {
                    t.yellow
                };
                let text = if new.len() == 1 {
                    "Arch news: 1 new (n)".to_string()
                } else {
                    format!("Arch news: {} new (n)", new.len())
                };
                (text, color)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pkg::Source;
    use crate::ui::test_util::{
        NEWS_AFTER, TALL, WIDE, clean_state, news_item, news_loaded, pkg, render_at, test_warning,
        two_packages_state, updates_state,
    };

    #[test]
    fn warning_mark_sits_right_before_the_name() {
        let mut st = two_packages_state();
        st.warnings.insert(
            "git".to_string(),
            vec![test_warning(Kind::Missing, Severity::Warn, "x")],
        );
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(rows.iter().any(|r| r.contains("⚠ git")));
        assert!(
            rows.iter()
                .any(|r| r.contains("firefox") && !r.contains("⚠"))
        );
    }

    #[test]
    fn search_rows_show_installed_check_and_newer_arrow() {
        let mut st = State::new("yay");
        st.tab = Tab::Search;
        st.results = vec![
            pkg(
                "git",
                "2.47.0-1",
                Source::Repo("extra".into()),
                Some("2.47.0-1"),
            ),
            pkg(
                "git-lfs",
                "3.5.0-1",
                Source::Repo("extra".into()),
                Some("3.4.0-1"),
            ),
            pkg("lazygit", "0.40.0-1", Source::Aur, None),
        ];
        st.clamp_lists();
        let rows = render_at(&mut st, WIDE, TALL).join("\n");
        // Between the indicator and the name sits the warning column, empty in Search.
        assert!(rows.contains("✓   git"));
        assert!(rows.contains("↑   git-lfs"));
        assert!(rows.contains("lazygit"));
    }

    #[test]
    fn updates_header_splits_into_two_lines_at_120x32() {
        let mut st = updates_state();
        st.news = news_loaded(vec![
            news_item("One", NEWS_AFTER),
            news_item("Two", NEWS_AFTER),
        ]);
        st.warnings.insert(
            "firefox".to_string(),
            vec![test_warning(Kind::Missing, Severity::Warn, "x")],
        );
        st.warnings.insert(
            "ollama".to_string(),
            vec![test_warning(Kind::Missing, Severity::Warn, "x")],
        );
        st.warnings.insert(
            "yay".to_string(),
            vec![test_warning(Kind::PreRelease, Severity::Warn, "x")],
        );
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(rows[2].contains("Arch news: 2 new (n) · 2 not in the AUR"));
        assert!(rows[3].contains("3 with warnings"));
    }

    #[test]
    fn long_update_version_is_cut_at_80_columns() {
        let mut st = State::new("yay");
        st.tab = Tab::Updates;
        let long = "2026.10.17.r123456.gabcdef0-1-extra-long-suffix";
        st.updates = vec![pkg("pkg", long, Source::Aur, Some("1.0-1"))];
        st.clamp_lists();
        let rows = render_at(&mut st, 80, 24).join("\n");
        assert!(rows.contains("2026.10.17"));
        assert!(rows.contains('…'));
        assert!(!rows.contains(long));
    }

    #[test]
    fn empty_updates_title_says_the_system_is_up_to_date() {
        let mut st = State::new("yay");
        st.tab = Tab::Updates;
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(rows[1].contains("· up to date"));
        assert!(rows.iter().any(|r| r.contains("Your system is up to date")));
    }

    #[test]
    fn clean_title_counts_reclaimable_space_without_the_full_cache() {
        let mut st = clean_state();
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(rows[1].contains("~9.8 GiB reclaimable"));
    }

    #[test]
    fn filter_without_matches_shows_the_empty_message() {
        let mut st = two_packages_state();
        st.filter = "zzz-no-existe".to_string();
        st.refilter();
        let rows = render_at(&mut st, WIDE, TALL).join("\n");
        assert!(rows.contains("No packages match the filter"));
    }
}

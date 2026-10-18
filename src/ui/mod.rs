//! Dibujo con ratatui. `draw` solo lee `State`: no toca la base de datos ni la red.

mod details;
mod help;
mod list;
mod modals;
mod pkgbuild;
#[cfg(test)]
mod test_util;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{Level, Modal, RightTab, State, Tab};
use crate::pkg::Source;
use crate::theme::Theme;

pub fn draw(frame: &mut Frame, st: &mut State) {
    let area = frame.area();
    let t = st.theme;
    frame.render_widget(
        Block::default().style(Style::default().bg(t.bg).fg(t.fg)),
        area,
    );
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);

    draw_top_bar(frame, top, st);
    list::draw_list(frame, left, st);
    details::draw_right(frame, right, st);
    draw_bottom_bar(frame, bottom, st);

    // La ayuda y las noticias ganan sobre cualquier otro modal.
    if matches!(st.modal, Some(Modal::Help)) {
        help::draw_help(frame, area, st);
        return;
    }
    if matches!(st.modal, Some(Modal::News)) {
        help::draw_news(frame, area, st);
        return;
    }
    match &st.modal {
        None | Some(Modal::Help) | Some(Modal::News) => {}
        Some(Modal::Confirm {
            kind,
            pkgs,
            command,
        }) => modals::draw_confirm(frame, area, t, *kind, pkgs, command),
        Some(Modal::Upgrade(plan)) => modals::draw_upgrade(frame, area, t, plan),
        Some(Modal::Clean { task, command }) => {
            modals::draw_clean_modal(frame, area, t, task, command)
        }
        Some(Modal::Theme { selected, .. }) => modals::draw_theme_picker(frame, area, t, *selected),
    }
}

fn draw_top_bar(frame: &mut Frame, area: Rect, st: &State) {
    let t = st.theme;
    let helper = format!(" helper: {} ", st.helper_name);

    let installed_text = format!("1 Instalados ({})", st.installed.len());
    let search_text = if st.query.trim().is_empty() {
        "2 Buscar".to_string()
    } else {
        format!("2 Buscar ({})", st.results.len())
    };
    let updates_text = format!("3 Actualizaciones ({})", updates_count(st));
    let spans = vec![
        Span::styled(" facpm ", Style::new().bold().fg(t.on_accent).bg(t.magenta)),
        Span::raw("  "),
        tab_span(t, installed_text, st.tab == Tab::Installed),
        Span::raw(" "),
        tab_span(t, search_text, st.tab == Tab::Search),
        Span::raw(" "),
        tab_span(t, updates_text, st.tab == Tab::Updates),
        Span::raw(" "),
        tab_span(t, "4 Limpieza", st.tab == Tab::Clean),
    ];
    // En terminales angostas el helper cede su lugar a las pestañas.
    let tabs_w = line_width(&Line::from(spans.clone()));
    let helper_w = if (area.width as usize) >= tabs_w + helper.chars().count() {
        helper.chars().count() as u16
    } else {
        0
    };
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(helper_w)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), left);
    if helper_w > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(helper, Style::new().fg(t.cyan))),
            right,
        );
    }
}

fn updates_count(st: &State) -> String {
    if st.updates_pending {
        "…".to_string()
    } else {
        st.updates.len().to_string()
    }
}

fn tab_span(t: &Theme, text: impl Into<String>, active: bool) -> Span<'static> {
    let style = if active {
        Style::new().bold().fg(t.on_accent).bg(t.accent)
    } else {
        Style::new().fg(t.muted)
    };
    Span::styled(format!(" {} ", text.into()), style)
}

/// Firma del borde inferior del panel derecho: versión y autor si entran; solo el autor si no.
fn signature(t: &Theme, width: u16) -> Option<Line<'static>> {
    let full = format!(" facpm v{} · By CRGUARIN ", env!("CARGO_PKG_VERSION"));
    let short = " By CRGUARIN ";
    let fits = |text: &str| width as usize >= text.chars().count() + 4;
    let text = if fits(&full) {
        full
    } else if fits(short) {
        short.to_string()
    } else {
        return None;
    };
    Some(Line::from(Span::styled(text, Style::new().fg(t.muted))).right_aligned())
}

fn row_secondary(t: &Theme, selected: bool) -> Color {
    if selected { t.fg } else { t.muted }
}

fn source_color(t: &Theme, source: &Source, selected: bool) -> Color {
    match source {
        Source::Repo(r) => match r.as_str() {
            "core" => t.blue,
            "extra" => t.green,
            "multilib" => t.yellow,
            _ => t.cyan,
        },
        Source::Aur => t.magenta,
        Source::Foreign => row_secondary(t, selected),
    }
}

fn draw_bottom_bar(frame: &mut Frame, area: Rect, st: &State) {
    let t = st.theme;
    // Sin modal y sin escribir, la barra cierra con las dos salidas globales.
    let tail: Vec<Span<'static>> = if st.modal.is_none() && !st.input_mode {
        let mut spans = vec![Span::styled(" │ ", Style::new().fg(t.muted))];
        spans.extend(hint_spans(t, &[("?", "ayuda"), ("q", "salir")]));
        spans
    } else {
        Vec::new()
    };
    let tail_w: usize = tail.iter().map(|s| s.content.chars().count()).sum();
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(tail_w as u16)]).areas(area);
    let line = match &st.status {
        Some(s) if s.is_recent() => {
            let color = match s.level {
                Level::Ok => t.green,
                Level::Warn => t.yellow,
                Level::Error => t.red,
            };
            Line::from(Span::styled(
                format!(" {}", s.text),
                Style::new().fg(color).bold(),
            ))
        }
        _ => hint_line(t, &hints(st)),
    };
    frame.render_widget(Paragraph::new(line), left);
    if !tail.is_empty() {
        frame.render_widget(Paragraph::new(Line::from(tail)), right);
    }
}

fn hints(st: &State) -> Vec<(&'static str, &'static str)> {
    match &st.modal {
        Some(Modal::Confirm { .. }) => return vec![("y/Enter", "sí"), ("n/Esc", "no")],
        Some(Modal::Upgrade(_)) => return vec![("y/Enter", "actualizar"), ("n/Esc", "no")],
        Some(Modal::Clean { .. }) => return vec![("y/Enter", "limpiar"), ("n/Esc", "no")],
        Some(Modal::Help) => return vec![("j/k", "desplazar"), ("?/Esc", "cerrar")],
        Some(Modal::News) => return vec![("j/k", "desplazar"), ("n/Esc", "cerrar")],
        Some(Modal::Theme { .. }) => {
            return vec![("j/k", "elegir"), ("Enter", "guardar"), ("Esc", "cancelar")];
        }
        None => {}
    }
    if st.input_mode {
        return vec![("Enter/Esc", "terminar"), ("Ctrl-u", "borrar todo")];
    }
    let other_panel = match st.right_tab {
        RightTab::Details => "PKGBUILD",
        RightTab::Pkgbuild => "detalles",
    };
    match st.tab {
        Tab::Installed => vec![
            ("j/k", "mover"),
            ("espacio", "marcar"),
            ("/", "filtrar"),
            ("d", "desinstalar"),
            ("u", "actualizar"),
            ("p", other_panel),
        ],
        Tab::Search => vec![
            ("j/k", "mover"),
            ("espacio", "marcar"),
            ("/", "buscar"),
            ("i", "instalar"),
            ("u", "actualizar"),
            ("p", other_panel),
        ],
        Tab::Clean => vec![
            ("j/k", "mover"),
            ("c/Enter", "limpiar"),
            ("r", "recalcular"),
            ("p", other_panel),
        ],
        Tab::Updates => vec![
            ("j/k", "mover"),
            ("u", "actualizar sistema"),
            ("n", "noticias"),
            ("r", "comprobar"),
            ("p", other_panel),
        ],
    }
}

fn hint_line(t: &Theme, hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(hint_spans(t, hints));
    Line::from(spans)
}

fn hint_spans(t: &Theme, hints: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (keys, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::new().fg(t.muted)));
        }
        spans.push(Span::styled(
            keys.to_string(),
            Style::new().fg(t.cyan).bold(),
        ));
        spans.push(Span::raw(format!(": {desc}")));
    }
    spans
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        format!("{s}{}", " ".repeat(w - n))
    } else if w == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(w - 1).collect();
        out.push('…');
        out
    }
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Filas que ocupa `lines` al partirse en un ancho de `inner_w`. Una línea vacía ocupa una.
fn rows_needed(lines: &[Line<'_>], inner_w: usize) -> usize {
    lines
        .iter()
        .map(|l| line_width(l).div_ceil(inner_w).max(1))
        .sum()
}

/// Parte `text` en líneas de `width` caracteres; cada línea lógica (por "\n") se parte aparte.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for logical in text.split('\n') {
        wrap_line(logical.trim(), width, &mut out);
    }
    out
}

fn wrap_line(line: &str, width: usize, out: &mut Vec<String>) {
    let mut cur = String::new();
    let mut cur_len = 0;
    for (gap, word) in word_gaps(line) {
        let gap_len = gap.chars().count();
        let word_len = word.chars().count();
        if cur_len > 0 && cur_len + gap_len + word_len <= width {
            cur.push_str(gap);
            cur.push_str(word);
            cur_len += gap_len + word_len;
            continue;
        }
        if cur_len > 0 {
            out.push(std::mem::take(&mut cur));
        }
        let mut rest = word;
        while rest.chars().count() > width {
            let split = rest
                .char_indices()
                .nth(width)
                .map_or(rest.len(), |(i, _)| i);
            out.push(rest[..split].to_string());
            rest = &rest[split..];
        }
        cur.push_str(rest);
        cur_len = rest.chars().count();
    }
    if !cur.is_empty() || line.is_empty() {
        out.push(cur);
    }
}

fn word_gaps(line: &str) -> Vec<(&str, &str)> {
    let mut parts = Vec::new();
    let mut rest = line.trim_start();
    let mut gap = "";
    while !rest.is_empty() {
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, after) = rest.split_at(word_end);
        parts.push((gap, word));
        let gap_end = after
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(after.len());
        let (next_gap, next) = after.split_at(gap_end);
        gap = next_gap;
        rest = next;
    }
    parts
}

/// Etiqueta y valor; las líneas que siguen al valor van con sangría colgante, bajo la etiqueta.
fn labeled_lines(label: &str, value: &str, width: usize) -> Vec<Line<'static>> {
    let label_w = label.chars().count();
    let val_w = width.saturating_sub(label_w).max(1);
    wrap(value, val_w)
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let head = if i == 0 {
                Span::styled(label.to_string(), Style::new().bold())
            } else {
                Span::raw(" ".repeat(label_w))
            };
            Line::from(vec![head, Span::raw(part)])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_util::{TALL, WIDE, all_scenarios, render_at, two_packages_state};

    #[test]
    fn no_panic_in_tiny_terminals_for_every_tab_and_modal() {
        for mut st in all_scenarios() {
            for (w, h) in [(1, 1), (7, 3), (20, 5)] {
                render_at(&mut st, w, h);
            }
        }
    }

    #[test]
    fn top_bar_drops_the_helper_name_when_narrow() {
        let mut st = two_packages_state();
        assert!(render_at(&mut st, WIDE, TALL)[0].contains("helper: yay"));
        assert!(!render_at(&mut st, 40, TALL)[0].contains("helper"));
    }

    #[test]
    fn signature_shows_version_and_author_or_only_author() {
        let mut st = two_packages_state();
        let wide = render_at(&mut st, WIDE, TALL).join("\n");
        assert!(wide.contains(concat!("facpm v", env!("CARGO_PKG_VERSION"))));
        assert!(wide.contains("By CRGUARIN"));

        let narrow = render_at(&mut st, 50, 20).join("\n");
        assert!(narrow.contains("By CRGUARIN"));
        assert!(!narrow.contains("facpm v"));
    }

    #[test]
    fn bottom_bar_hides_the_quit_hint_while_typing() {
        let mut st = two_packages_state();
        let idle = render_at(&mut st, WIDE, TALL);
        assert!(idle[TALL as usize - 1].contains("q: salir"));

        st.input_mode = true;
        let typing = render_at(&mut st, WIDE, TALL);
        assert!(!typing[TALL as usize - 1].contains("q: salir"));
        assert!(typing[TALL as usize - 1].contains("Enter/Esc: terminar"));
    }

    #[test]
    fn fit_cuts_with_an_ellipsis_and_pads_with_spaces() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("abcdef", 0), "");
    }

    #[test]
    fn wrap_keeps_newlines_double_spaces_and_cuts_long_words() {
        assert_eq!(wrap("uno dos\ntres", 5), ["uno", "dos", "tres"]);
        assert_eq!(wrap("a  b", 10), ["a  b"]);
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(rows_needed(&[Line::from("abcdef"), Line::raw("")], 4), 3);
    }
}

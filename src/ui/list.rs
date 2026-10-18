//! Panel izquierdo: la lista de la pestaña activa, con su filtro o búsqueda y el título.

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
        Tab::Installed => " Instalados ".to_string(),
        Tab::Search => format!(" Buscar · {} ", aur_status(&st.aur_state)),
        Tab::Updates => format!(" Actualizaciones {} ", updates_status(st)),
        Tab::Clean => format!(" Limpieza {} ", clean_status(st)),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.accent))
        .title(Line::from(title).bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // En Actualizaciones el encabezado ocupa de una a tres líneas; la lista empieza debajo.
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

/// Parte del título de Actualizaciones: "· error", "· buscando…", "· sistema al día" o
/// "· N disponibles".
fn updates_status(st: &State) -> String {
    if st.updates_error {
        "· error".to_string()
    } else if st.updates_pending {
        "· buscando…".to_string()
    } else if st.updates.is_empty() {
        "· sistema al día".to_string()
    } else {
        format!("· {} disponibles", st.updates.len())
    }
}

fn aur_status(state: &AurState) -> String {
    match state {
        AurState::Idle => "AUR: en espera".to_string(),
        AurState::Loading => "AUR: buscando…".to_string(),
        AurState::Done { count, local } => {
            let source = if *local { "local" } else { "API" };
            if *count == 1 {
                format!("AUR: 1 resultado ({source})")
            } else {
                format!("AUR: {count} resultados ({source})")
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
                Line::from("c/Enter: limpiar la tarea seleccionada")
                    .style(Style::new().fg(t.muted)),
            ),
            area,
        );
        return;
    }
    let (prefix, text) = match st.tab {
        Tab::Search => ("Buscar: ", st.query.as_str()),
        _ => ("/ ", st.filter.as_str()),
    };
    let line = if text.is_empty() && !st.input_mode {
        let hint = match st.tab {
            Tab::Search => "presiona / para buscar",
            _ => "presiona / para filtrar",
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
        Tab::Installed if st.installed.is_empty() => "No hay paquetes instalados",
        Tab::Installed => "Ningún paquete coincide con el filtro",
        Tab::Search if st.query.trim().is_empty() => "Escribe para buscar en repos y AUR",
        Tab::Search if matches!(st.aur_state, AurState::Loading) => "Buscando en el AUR…",
        Tab::Search => "Sin resultados",
        Tab::Clean if st.clean_pending => "Calculando la limpieza…",
        Tab::Clean => "Nada que limpiar",
        Tab::Updates if st.updates_pending => "Comprobando actualizaciones…",
        Tab::Updates if st.updates_error => "No se pudieron comprobar las actualizaciones",
        Tab::Updates => "No hay actualizaciones pendientes",
    }
}

/// Anchos (nombre, versión): la versión toma lo que piden sus filas, en [8, 20] y hasta 2/5.
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
    // marca (2) + indicador (2) + espacio + 2 espacios antes del origen + origen
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

/// En Buscar: ✓ si la versión instalada es la disponible, ↑ si difiere; en Instalados, ↑ si hay actualización.
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
    // marca (2) + indicador (2) + advertencia (2) + espacio + 2 espacios antes del origen + origen
    let available = width.saturating_sub(2 + 2 + 2 + 1 + 2 + label_w);
    // Lo que pide la fila más larga, acotado; si el panel es angosto, al menos cabe "→ nueva".
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

/// Versiones en `w` columnas: "actual → nueva", luego "→ nueva", luego la nueva recortada.
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

/// Marca de advertencia: `⚠ ` en rojo o amarillo, o dos espacios; en Buscar no hay marca.
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
        return "· calculando…".to_string();
    }
    let any_other = st
        .clean
        .iter()
        .any(|r| r.task.available && r.task.kind != CleanKind::PkgCacheAll);
    if !any_other {
        return "· nada que limpiar".to_string();
    }
    format!(
        "· ~{} recuperables",
        human_size(reclaimable_bytes(&st.clean))
    )
}

/// Bytes recuperables: las tareas disponibles sin `PkgCacheAll`, que se superpone con las demás.
fn reclaimable_bytes(rows: &[CleanRow]) -> u64 {
    rows.iter()
        .filter(|r| r.task.available && r.task.kind != CleanKind::PkgCacheAll)
        .filter_map(|r| r.task.bytes)
        .sum()
}

/// Filas de Limpieza: título a la izquierda y cantidad y tamaño a la derecha.
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

/// Encabezado de Actualizaciones: las partes van con " · " y se reparten en hasta tres líneas.
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

/// Partes del encabezado: noticias, instalados fuera del AUR y actualizaciones con advertencias.
fn updates_header_parts(st: &State) -> Vec<(String, Color)> {
    let t = st.theme;
    let mut parts = vec![news_header_part(st)];
    let missing = st
        .warnings
        .values()
        .filter(|ws| ws.iter().any(|w| w.kind == Kind::Missing))
        .count();
    if missing > 0 {
        parts.push((format!("{missing} fuera del AUR"), t.yellow));
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
        parts.push((format!("{} con advertencias", flagged.len()), color));
    }
    parts
}

fn news_header_part(st: &State) -> (String, Color) {
    let t = st.theme;
    match &st.news {
        NewsState::Loading => ("Noticias de Arch: cargando…".to_string(), t.muted),
        NewsState::Error(_) => ("Noticias de Arch: sin conexión".to_string(), t.muted),
        NewsState::Loaded { .. } => {
            let new = st.news.new_items();
            if new.is_empty() {
                ("Noticias de Arch: nada nuevo (n)".to_string(), t.muted)
            } else {
                let color = if new.iter().any(|item| news::is_intervention(&item.title)) {
                    t.red
                } else {
                    t.yellow
                };
                let text = if new.len() == 1 {
                    "Noticias de Arch: 1 nueva (n)".to_string()
                } else {
                    format!("Noticias de Arch: {} nuevas (n)", new.len())
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
        // Entre el indicador y el nombre queda la columna de advertencias, vacía en Buscar.
        assert!(rows.contains("✓   git"));
        assert!(rows.contains("↑   git-lfs"));
        assert!(rows.contains("lazygit"));
    }

    #[test]
    fn updates_header_splits_into_two_lines_at_120x32() {
        let mut st = updates_state();
        st.news = news_loaded(vec![
            news_item("Uno", NEWS_AFTER),
            news_item("Dos", NEWS_AFTER),
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
        assert!(rows[2].contains("Noticias de Arch: 2 nuevas (n) · 2 fuera del AUR"));
        assert!(rows[3].contains("3 con advertencias"));
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
        assert!(rows[1].contains("· sistema al día"));
        assert!(
            rows.iter()
                .any(|r| r.contains("No hay actualizaciones pendientes"))
        );
    }

    #[test]
    fn clean_title_counts_reclaimable_space_without_the_full_cache() {
        let mut st = clean_state();
        let rows = render_at(&mut st, WIDE, TALL);
        assert!(rows[1].contains("~9.8 GiB recuperables"));
    }

    #[test]
    fn filter_without_matches_shows_the_empty_message() {
        let mut st = two_packages_state();
        st.filter = "zzz-no-existe".to_string();
        st.refilter();
        let rows = render_at(&mut st, WIDE, TALL).join("\n");
        assert!(rows.contains("Ningún paquete coincide con el filtro"));
    }
}

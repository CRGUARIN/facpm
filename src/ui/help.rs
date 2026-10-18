//! Popups de ayuda (`?`) y de noticias de Arch (`n`). Ambos tienen el mismo tamaño y scroll.

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
/// Grupos en la columna izquierda cuando hay dos; el resto va a la derecha.
const HELP_LEFT_GROUPS: usize = 3;

struct HelpGroup {
    title: &'static str,
    keys: &'static [(&'static str, &'static str)],
    notes: &'static [&'static str],
}

const HELP_GROUPS: &[HelpGroup] = &[
    HelpGroup {
        title: "Lista (panel izquierdo)",
        keys: &[
            ("j/k ↓/↑", "Mover la selección"),
            ("g/G", "Ir al primero / al último"),
            ("PgUp/PgDn", "Subir / bajar una página"),
            (
                "1 2 3 4",
                "Instalados · Buscar · Actualizaciones · Limpieza",
            ),
            ("h/l ←/→", "Pestaña anterior / siguiente"),
        ],
        notes: &[],
    },
    HelpGroup {
        title: "Panel derecho",
        keys: &[
            ("p", "Alternar entre Detalles y PKGBUILD"),
            ("J/K", "Desplazar una línea (Shift + j/k)"),
            ("Ctrl-d/Ctrl-u", "Desplazar media página"),
        ],
        notes: &[
            "El desplazamiento vuelve al inicio al cambiar de paquete. Las líneas largas del PKGBUILD se cortan.",
        ],
    },
    HelpGroup {
        title: "Escribir (filtrar / buscar)",
        keys: &[
            (
                "/",
                "Empezar a escribir (en Buscar también al entrar con 2)",
            ),
            ("Enter", "Terminar de escribir y conservar el texto"),
            ("Esc", "Salir del modo escritura"),
            ("Ctrl-u", "Borrar todo el texto"),
        ],
        notes: &[
            "Mientras escribes, las teclas van al texto: sal con Enter o Esc para usar los atajos.",
        ],
    },
    HelpGroup {
        title: "Paquetes",
        keys: &[
            ("espacio", "Marcar / desmarcar (para operar sobre varios)"),
            ("i", "Instalar los marcados (o el seleccionado)"),
            ("d/x", "Desinstalar los marcados (o el seleccionado)"),
            ("u", "Actualizar todo el sistema"),
            ("r", "Recargar la base / volver a comprobar"),
        ],
        notes: &[],
    },
    HelpGroup {
        title: "Limpieza (pestaña 4)",
        keys: &[("c/Enter", "Limpiar la tarea seleccionada")],
        notes: &[],
    },
    HelpGroup {
        title: "General",
        keys: &[
            ("t", "Cambiar el tema de colores"),
            ("n", "Noticias de Arch Linux"),
            ("?", "Mostrar / ocultar esta ayuda"),
            ("q", "Salir"),
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
        .title(Line::from(format!(" Ayuda · facpm v{} ", env!("CARGO_PKG_VERSION"))).bold())
        .title_bottom(Line::from(" ?/Esc: cerrar ").right_aligned());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(rows)
            .scroll((st.help_scroll, 0))
            .block(block),
        rect,
    );
}

/// Filas de la ayuda para un ancho interior: dos columnas si caben; si no, una.
fn help_rows(t: &Theme, inner_w: usize) -> Vec<Line<'static>> {
    if inner_w < HELP_TWO_COLUMNS_MIN {
        return help_column(t, HELP_GROUPS, inner_w);
    }
    let col_w = (inner_w - HELP_COL_GAP) / 2;
    let left = help_column(t, &HELP_GROUPS[..HELP_LEFT_GROUPS], col_w);
    let right = help_column(t, &HELP_GROUPS[HELP_LEFT_GROUPS..], col_w);
    zip_columns(&left, &right, col_w)
}

/// Una columna: cada grupo con su título, sus atajos (tecla en chip) y sus notas en cursiva.
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
            for part in wrap(&format!("nota: {note}"), desc_w) {
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

/// Popup de noticias de Arch: mismo tamaño y scroll que la ayuda.
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
        .title(Line::from(" Noticias de Arch ").bold())
        .title_bottom(Line::from(" n/Esc: cerrar ").right_aligned());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(rows)
            .scroll((st.help_scroll, 0))
            .block(block),
        rect,
    );
}

/// Filas de las noticias, de la más nueva a la más vieja, con fecha, título, cuerpo y link.
fn news_rows(t: &Theme, news: &NewsState, width: usize) -> Vec<Line<'static>> {
    let muted = Style::new().fg(t.muted);
    let NewsState::Loaded { items, since } = news else {
        let text = match news {
            NewsState::Error(_) => "no se pudieron descargar",
            _ => "cargando…",
        };
        return vec![Line::from(text).style(muted)];
    };
    if items.is_empty() {
        return vec![Line::from("Sin noticias de Arch.").style(muted)];
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
        // "● " + fecha + "  ": el título va después y sus líneas siguen bajo él.
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
        // Las viñetas `• ` siguen con sangría de 4, bajo su texto.
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
        // El link va en muted; si no entra, se parte en líneas seguidas, sin cortarlo.
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
                .any(|r| r.contains("Lista (panel izquierdo)") && r.contains("Paquetes"))
        );
    }

    #[test]
    fn help_has_one_column_at_80_columns() {
        let mut st = help_state();
        let rows = render_at(&mut st, 80, 24);
        assert!(
            !rows
                .iter()
                .any(|r| r.contains("Lista (panel izquierdo)") && r.contains("Paquetes"))
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
                .contains("cargando…")
        );
        st.news = NewsState::Error("x".to_string());
        assert!(
            render_at(&mut st, WIDE, TALL)
                .join("\n")
                .contains("no se pudieron descargar")
        );
    }

    #[test]
    fn long_news_link_wraps_on_consecutive_lines() {
        let link = "https://archlinux.org/news/un-enlace-largo-que-no-entra-en-una-linea-de-ochenta-columnas-completas/";
        let mut item = news_item("Titulo", NEWS_AFTER);
        item.link = link.to_string();
        let mut st = updates_state();
        st.modal = Some(Modal::News);
        st.news = news_loaded(vec![item]);
        let rows = render_at(&mut st, 80, 24);
        let first = rows
            .iter()
            .position(|r| r.contains(&link[..72]))
            .expect("primera parte del link");
        assert!(rows[first + 1].contains(&link[72..]));
    }

    #[test]
    fn bullet_continuations_indent_under_the_text() {
        let mut item = news_item("Titulo", NEWS_AFTER);
        item.body = "• primer punto con palabras suficientes para que la viñeta se parta en varias líneas zzfin".to_string();
        let mut st = updates_state();
        st.modal = Some(Modal::News);
        st.news = news_loaded(vec![item]);
        let rows = render_at(&mut st, 80, 24);
        let first = rows
            .iter()
            .position(|r| r.contains("• primer"))
            .expect("viñeta");
        let text_col = rows[first].find("primer").expect("texto de la viñeta");
        let last = rows
            .iter()
            .position(|r| r.contains("zzfin"))
            .expect("última parte");
        assert!(last > first);
        assert_eq!(rows[last].find("zzfin").map(|c| c >= text_col), Some(true));
    }
}

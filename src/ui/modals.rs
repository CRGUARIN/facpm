//! Modales centrados: confirmación de instalar o desinstalar, actualización del sistema, limpieza
//! y selector de tema.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::{OpKind, UpgradePlan};
use crate::backend::clean::{CleanKind, CleanTask};
use crate::backend::repo::human_size;
use crate::backend::warnings::Severity;
use crate::theme::{THEMES, Theme};

use super::{centered, fit, labeled_lines, rows_needed, wrap};

/// Líneas de paquetes o tareas que lista un modal antes de resumir con "y N más".
const MAX_CONFIRM_LINES: usize = 10;
/// Advertencias de actualización que lista el modal de `u` antes de resumir.
const UPGRADE_WARNINGS_MAX: usize = 6;
const THEME_PICKER_WIDTH: u16 = 44;

pub(super) fn draw_confirm(
    frame: &mut Frame,
    screen: Rect,
    t: &Theme,
    kind: OpKind,
    pkgs: &[String],
    command: &str,
) {
    let (title, verb) = match kind {
        OpKind::Remove => (" Desinstalar ", "Desinstalar"),
        _ => (" Instalar ", "Instalar"),
    };
    let n = pkgs.len();
    let noun = if n == 1 { "paquete" } else { "paquetes" };
    let width = 72u16.min(screen.width);
    let inner_w = width.saturating_sub(2).max(1) as usize;

    let mut lines = vec![
        Line::from(format!("¿{verb} {n} {noun}?")).bold(),
        Line::raw(""),
    ];
    lines.extend(
        pkgs.iter()
            .take(MAX_CONFIRM_LINES)
            .map(|p| Line::from(format!("  {p}"))),
    );
    if n > MAX_CONFIRM_LINES {
        lines.push(
            Line::from(format!("  y {} más", n - MAX_CONFIRM_LINES))
                .style(Style::new().fg(t.muted)),
        );
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("Comando: ", Style::new().bold()),
        Span::raw(command.to_string()),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from("y/Enter: sí   n/Esc: no").style(Style::new().fg(t.cyan)));

    let height = (rows_needed(&lines, inner_w) + 2) as u16;
    let rect = centered(screen, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.yellow))
        .style(Style::default().bg(t.surface).fg(t.fg))
        .title(Line::from(title).bold());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

pub(super) fn draw_upgrade(frame: &mut Frame, screen: Rect, t: &Theme, plan: &UpgradePlan) {
    let width = 72u16.min(screen.width);
    let inner_w = width.saturating_sub(2).max(1) as usize;

    let mut lines = Vec::new();
    if plan.repos + plan.aur == 0 {
        lines.push(
            Line::from("No se detectaron actualizaciones; igual puedes ejecutar el comando").bold(),
        );
    } else {
        lines.push(Line::from(format!("{} de repos, {} del AUR", plan.repos, plan.aur)).bold());
    }
    lines.push(Line::raw(""));
    lines.extend(plan.lines.iter().map(|l| Line::from(format!("  {l}"))));
    if plan.more > 0 {
        lines.push(Line::from(format!("  y {} más", plan.more)).style(Style::new().fg(t.muted)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("Comando: ", Style::new().bold()),
        Span::raw(plan.command.clone()),
    ]));
    if plan.pacman_warning {
        lines.push(
            Line::from("pacman no actualiza paquetes del AUR").style(Style::new().fg(t.yellow)),
        );
    }
    if !plan.flagged.is_empty() || plan.news_new > 0 {
        lines.push(Line::raw(""));
        lines.push(Line::from("Advertencias").bold());
        for (name, severity, label) in plan.flagged.iter().take(UPGRADE_WARNINGS_MAX) {
            let color = match severity {
                Severity::Alarm => t.red,
                _ => t.yellow,
            };
            lines.push(Line::from(format!("⚠ {name}: {label}")).style(Style::new().fg(color)));
        }
        if plan.flagged.len() > UPGRADE_WARNINGS_MAX {
            let more = plan.flagged.len() - UPGRADE_WARNINGS_MAX;
            lines.push(Line::from(format!("… y {more} más")).style(Style::new().fg(t.muted)));
        }
        if plan.news_new > 0 {
            let color = if plan.news_intervention {
                t.red
            } else {
                t.yellow
            };
            let text = if plan.news_new == 1 {
                "1 noticia de Arch nueva: cancelá con Esc y apretá n para leerla".to_string()
            } else {
                format!(
                    "{} noticias de Arch nuevas: cancelá con Esc y apretá n para leerlas",
                    plan.news_new
                )
            };
            lines.push(Line::from(text).style(Style::new().fg(color)));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from("y/Enter: sí   n/Esc: no").style(Style::new().fg(t.cyan)));

    let height = (rows_needed(&lines, inner_w) + 2) as u16;
    let rect = centered(screen, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.yellow))
        .style(Style::default().bg(t.surface).fg(t.fg))
        .title(Line::from(" Actualizar el sistema ").bold());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

/// Las dos tareas que borran versiones o toda la caché muestran su descripción en amarillo.
pub(super) fn draw_clean_modal(
    frame: &mut Frame,
    screen: Rect,
    t: &Theme,
    task: &CleanTask,
    command: &str,
) {
    let width = 72u16.min(screen.width);
    let inner_w = width.saturating_sub(2).max(1) as usize;

    let mut lines = vec![Line::from(task.title.clone()).bold()];
    let liberates = match task.bytes {
        Some(bytes) => format!("Liberará ~{}", human_size(bytes)),
        None => "Tamaño desconocido".to_string(),
    };
    lines.push(Line::from(liberates));
    if !task.items.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(
            task.items
                .iter()
                .take(MAX_CONFIRM_LINES)
                .map(|item| Line::from(format!("  {item}"))),
        );
        if task.items.len() > MAX_CONFIRM_LINES {
            let more = task.items.len() - MAX_CONFIRM_LINES;
            lines.push(Line::from(format!("  y {more} más")).style(Style::new().fg(t.muted)));
        }
    }
    lines.push(Line::raw(""));
    lines.extend(labeled_lines("Comando: ", command, inner_w));
    if matches!(
        task.kind,
        CleanKind::PkgCacheAll | CleanKind::PkgCacheKeepLatest
    ) {
        lines.push(Line::raw(""));
        lines.extend(
            wrap(&task.description, inner_w)
                .into_iter()
                .map(|l| Line::from(l).style(Style::new().fg(t.yellow))),
        );
    }
    lines.push(Line::raw(""));
    lines.push(Line::from("y/Enter: sí   n/Esc: no").style(Style::new().fg(t.cyan)));

    let height = (rows_needed(&lines, inner_w) + 2) as u16;
    let rect = centered(screen, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.yellow))
        .style(Style::default().bg(t.surface).fg(t.fg))
        .title(Line::from(" Limpiar ").bold());
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

/// Una fila por tema; `●` marca el aplicado y la seleccionada va con fondo `selection`.
pub(super) fn draw_theme_picker(frame: &mut Frame, screen: Rect, t: &Theme, selected: usize) {
    let width = THEME_PICKER_WIDTH.min(screen.width.saturating_sub(4));
    let height = (THEMES.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let inner_h = height.saturating_sub(2) as usize;
    // Si no entra, la selección queda visible.
    let offset = if selected >= inner_h {
        selected + 1 - inner_h
    } else {
        0
    };
    let rows: Vec<Line<'static>> = THEMES
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner_h)
        .map(|(i, theme)| {
            let marker = if theme.id == t.id { "● " } else { "  " };
            let mut spans = vec![
                Span::styled(marker, Style::new().fg(t.accent)),
                Span::raw(fit(theme.name, 18)),
            ];
            let swatch = [
                theme.red,
                theme.green,
                theme.yellow,
                theme.blue,
                theme.magenta,
                theme.cyan,
                theme.accent,
            ];
            for (j, color) in swatch.into_iter().enumerate() {
                if j > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled("██", Style::new().fg(color)));
            }
            let line = Line::from(spans);
            if i == selected {
                line.style(Style::new().bg(t.selection).bold())
            } else {
                line
            }
        })
        .collect();
    let rect = centered(screen, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.accent))
        .style(Style::default().bg(t.surface).fg(t.fg))
        .title(Line::from(" Tema ").bold())
        .title_bottom(Line::from(" Enter: guardar · Esc: cancelar ").right_aligned());
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(rows).block(block), rect);
}

#[cfg(test)]
mod tests {
    use crate::app::{Modal, OpKind, State, UpgradePlan};
    use crate::backend::clean::CleanTask;
    use crate::theme::THEMES;
    use crate::ui::test_util::{WIDE, clean_state, render_at, two_packages_state};

    fn plan(repos: usize, aur: usize, news_new: usize, pacman: bool) -> UpgradePlan {
        UpgradePlan {
            repos,
            aur,
            lines: vec!["firefox 131.0-1 → 131.0-2".to_string()],
            more: 0,
            command: if pacman {
                "sudo pacman -Syu"
            } else {
                "yay -Syu"
            }
            .to_string(),
            pacman_warning: pacman,
            flagged: Vec::new(),
            news_new,
            news_intervention: false,
        }
    }

    fn upgrade_text(news_new: usize) -> String {
        let mut st = State::new("yay");
        st.modal = Some(Modal::Upgrade(plan(1, 0, news_new, false)));
        render_at(&mut st, WIDE, 32).join("\n")
    }

    #[test]
    fn upgrade_modal_says_one_news_or_several() {
        assert!(upgrade_text(1).contains("1 noticia de Arch nueva"));
        assert!(upgrade_text(2).contains("2 noticias de Arch nuevas"));
    }

    #[test]
    fn upgrade_modal_with_nothing_to_update_still_offers_the_command() {
        let mut st = State::new("yay");
        st.modal = Some(Modal::Upgrade(plan(0, 0, 0, false)));
        let text = render_at(&mut st, WIDE, 32).join("\n");
        assert!(text.contains("No se detectaron actualizaciones"));
        assert!(text.contains("y/Enter: sí"));
    }

    #[test]
    fn upgrade_modal_warns_when_pacman_skips_the_aur() {
        let mut st = State::new("yay");
        st.modal = Some(Modal::Upgrade(plan(1, 1, 0, true)));
        let text = render_at(&mut st, WIDE, 32).join("\n");
        assert!(text.contains("pacman no actualiza paquetes del AUR"));
        assert!(text.contains("sudo pacman -Syu"));
    }

    #[test]
    fn confirm_modal_uses_singular_and_plural_and_caps_the_list() {
        let mut one = two_packages_state();
        one.modal = Some(Modal::Confirm {
            kind: OpKind::Install,
            pkgs: vec!["firefox".to_string()],
            command: "yay -S firefox".to_string(),
        });
        assert!(
            render_at(&mut one, WIDE, 32)
                .join("\n")
                .contains("¿Instalar 1 paquete?")
        );

        let mut many = two_packages_state();
        many.modal = Some(Modal::Confirm {
            kind: OpKind::Remove,
            pkgs: (0..12).map(|i| format!("paquete-{i}")).collect(),
            command: "yay -Rns".to_string(),
        });
        let text = render_at(&mut many, WIDE, 32).join("\n");
        assert!(text.contains("¿Desinstalar 12 paquetes?"));
        assert!(text.contains("y 2 más"));
    }

    #[test]
    fn clean_modal_explains_only_the_destructive_tasks() {
        let tasks: Vec<CleanTask> = crate::app::sample_clean_tasks();
        let mut keep = clean_state();
        keep.modal = Some(Modal::Clean {
            task: tasks[1].clone(),
            command: "sudo paccache -rk1".to_string(),
        });
        assert!(
            render_at(&mut keep, WIDE, 32)
                .join("\n")
                .contains("Borra de la caché todas las")
        );

        let mut orphans = clean_state();
        orphans.modal = Some(Modal::Clean {
            task: tasks[0].clone(),
            command: "yay -Rns go".to_string(),
        });
        assert!(
            !render_at(&mut orphans, WIDE, 32)
                .join("\n")
                .contains("Borra de la caché todas las")
        );
    }

    #[test]
    fn theme_picker_marks_the_applied_theme() {
        let mut st = two_packages_state();
        st.modal = Some(Modal::Theme {
            selected: 1,
            original: THEMES[0].id,
        });
        let text = render_at(&mut st, WIDE, 32).join("\n");
        assert!(text.contains(&format!("● {}", THEMES[0].name)));
        assert!(!text.contains(&format!("● {}", THEMES[1].name)));
    }
}

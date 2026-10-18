//! Pestaña PKGBUILD del panel derecho: texto con números de línea y resaltado de bash.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{PkgbuildView, State};
use crate::theme::Theme;

use super::details::draw_message;

pub(super) fn draw_pkgbuild(frame: &mut Frame, area: Rect, st: &mut State) {
    let t = st.theme;
    let text = match &st.pkgbuild {
        PkgbuildView::Ready(text) => text,
        PkgbuildView::Empty => {
            return draw_message(
                frame,
                area,
                Line::from("Sin paquete seleccionado").style(Style::new().fg(t.muted)),
            );
        }
        PkgbuildView::Loading => {
            return draw_message(
                frame,
                area,
                Line::from("Cargando PKGBUILD…").style(Style::new().fg(t.muted)),
            );
        }
        PkgbuildView::Error(e) => {
            let msg = format!("No se pudo obtener el PKGBUILD: {e}");
            return draw_message(frame, area, Line::from(msg).style(Style::new().fg(t.red)));
        }
    };
    let total = text.lines().count();
    let max_scroll = total.saturating_sub(area.height as usize) as u16;
    st.right_scroll = st.right_scroll.min(max_scroll);
    let num_w = total.max(1).to_string().len();
    let lines: Vec<Line> = text
        .lines()
        .enumerate()
        .skip(st.right_scroll as usize)
        .take(area.height as usize)
        .map(|(i, line)| code_line(t, i + 1, num_w, line))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Tabuladores a espacios y demás caracteres de control a espacio, para no romper el dibujo.
fn code_line(t: &Theme, num: usize, num_w: usize, text: &str) -> Line<'static> {
    let clean: String = text
        .replace('\t', "    ")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut spans = vec![Span::styled(
        format!("{num:>num_w$} "),
        Style::new().fg(t.muted),
    )];
    spans.extend(
        bash_tokens(&clean)
            .into_iter()
            .map(|(kind, part)| Span::styled(part.to_string(), hl_style(t, kind))),
    );
    Line::from(spans)
}

fn hl_style(t: &Theme, kind: Hl) -> Style {
    match kind {
        Hl::Plain => Style::new(),
        Hl::Comment => Style::new().fg(t.muted),
        Hl::Assign => Style::new().fg(t.cyan),
        Hl::Expansion => Style::new().fg(t.yellow),
        Hl::Str => Style::new().fg(t.green),
        Hl::Func => Style::new().fg(t.magenta).bold(),
        Hl::Keyword => Style::new().fg(t.blue),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hl {
    Plain,
    Comment,
    /// Nombre de una asignación al inicio de la línea (`pkgname=`, `depends+=(`).
    Assign,
    Expansion,
    Str,
    Func,
    Keyword,
}

const BASH_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "in",
    "function", "local", "return",
];

/// Parte una línea de PKGBUILD en trozos clasificados, que concatenados dan la línea original.
/// No sigue estado entre líneas: un string que no cierra llega hasta el fin de su línea.
fn bash_tokens(line: &str) -> Vec<(Hl, &str)> {
    let b = line.as_bytes();
    let n = b.len();
    let indent = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    let mut lx = Lexer {
        line,
        out: Vec::new(),
        plain: 0,
    };
    let mut i = 0;
    while i < n {
        let c = b[i];
        match c {
            b'#' if i == 0 || b[i - 1].is_ascii_whitespace() => {
                lx.token(i, n, Hl::Comment);
                break;
            }
            b'\'' => {
                let end = b[i + 1..]
                    .iter()
                    .position(|&x| x == b'\'')
                    .map_or(n, |p| i + p + 2);
                lx.token(i, end, Hl::Str);
                i = end;
            }
            b'"' => i = double_quoted(&mut lx, b, i),
            b'$' => {
                let end = expansion_end(b, i);
                if end > i {
                    lx.token(i, end, Hl::Expansion);
                    i = end;
                } else {
                    i += 1;
                }
            }
            c if c.is_ascii_alphanumeric() || c == b'_' => {
                let start = i;
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                // Un número no es identificador: no se resalta ni se buscan palabras dentro.
                if !(c.is_ascii_alphabetic() || c == b'_') {
                    continue;
                }
                let word = &line[start..i];
                let at_start = start == indent;
                let after = &line[i..];
                if at_start && (after.starts_with('=') || after.starts_with("+=")) {
                    lx.token(start, i, Hl::Assign);
                } else if at_start && after.trim_start_matches([' ', '\t']).starts_with("()") {
                    lx.token(start, i, Hl::Func);
                } else if BASH_KEYWORDS.contains(&word) {
                    lx.token(start, i, Hl::Keyword);
                }
            }
            _ => i += 1,
        }
    }
    lx.finish()
}

/// Procesa un string entre comillas dobles que empieza en `start`: el texto va como `Str` y cada
/// `$...` como `Expansion`. Devuelve el índice después del cierre, o el fin de la línea.
fn double_quoted(lx: &mut Lexer<'_>, b: &[u8], start: usize) -> usize {
    let n = b.len();
    let mut seg = start;
    let mut i = start + 1;
    while i < n {
        match b[i] {
            b'\\' => i += 2,
            b'"' => {
                lx.token(seg, i + 1, Hl::Str);
                return i + 1;
            }
            b'$' => {
                let end = expansion_end(b, i);
                if end > i {
                    lx.token(seg, i, Hl::Str);
                    lx.token(i, end, Hl::Expansion);
                    seg = end;
                    i = end;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    lx.token(seg, n, Hl::Str);
    n
}

/// Fin de la expansión que empieza en el `$` de `i` (`$var`, `${...}`, `$1`, `$@`...), o `i` si
/// ese `$` no inicia ninguna (p. ej. `$(`).
fn expansion_end(b: &[u8], i: usize) -> usize {
    let n = b.len();
    match b.get(i + 1).copied() {
        Some(b'{') => b[i + 2..]
            .iter()
            .position(|&c| c == b'}')
            .map_or(n, |p| i + p + 3),
        Some(c) if c.is_ascii_alphanumeric() || c == b'_' => {
            let mut j = i + 1;
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            j
        }
        Some(b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-') => i + 2,
        _ => i,
    }
}

struct Lexer<'a> {
    line: &'a str,
    out: Vec<(Hl, &'a str)>,
    plain: usize,
}

impl<'a> Lexer<'a> {
    /// Agrega el trozo `[start, end)` con la clase `kind`, volcando antes el texto sin clasificar.
    fn token(&mut self, start: usize, end: usize, kind: Hl) {
        if start >= end {
            return;
        }
        let line = self.line;
        if start > self.plain {
            self.out.push((Hl::Plain, &line[self.plain..start]));
        }
        self.out.push((kind, &line[start..end]));
        self.plain = end;
    }

    fn finish(mut self) -> Vec<(Hl, &'a str)> {
        let line = self.line;
        if self.plain < line.len() {
            self.out.push((Hl::Plain, &line[self.plain..]));
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{PkgbuildView, RightTab, State, Tab};
    use crate::ui::test_util::{TALL, WIDE, render_at, two_packages_state};

    fn kinds(line: &str) -> Vec<(Hl, &str)> {
        bash_tokens(line)
    }

    #[test]
    fn tokens_cover_every_line_exactly() {
        let lines = [
            "pkgname=foo",
            "build() {",
            "  cd \"${srcdir}/${pkgname}-${pkgver}\"",
            "    make   # compila $HOME",
            "",
            "url=\"https://example.org/${pkgname}\"",
            "x = 'a''b'",
            "echo \"# no es comentario\"",
        ];
        for line in lines {
            let joined: String = kinds(line).iter().map(|(_, part)| *part).collect();
            assert_eq!(joined, line);
        }
    }

    #[test]
    fn assignments_functions_keywords_and_comments_are_classified() {
        assert_eq!(kinds("pkgname=foo")[0], (Hl::Assign, "pkgname"));
        assert_eq!(kinds("build() {")[0], (Hl::Func, "build"));
        let fi = kinds("  if [ -f x ]; then");
        assert!(fi.contains(&(Hl::Keyword, "if")));
        assert!(fi.contains(&(Hl::Keyword, "then")));
        assert!(kinds("make # compila").contains(&(Hl::Comment, "# compila")));
    }

    #[test]
    fn hash_inside_a_string_is_not_a_comment() {
        let tokens = kinds("echo \"a # b\"");
        assert!(tokens.iter().all(|(kind, _)| *kind != Hl::Comment));
        assert!(tokens.contains(&(Hl::Str, "\"a # b\"")));
    }

    #[test]
    fn expansions_inside_double_quotes_are_split_out() {
        let tokens = kinds("url=\"${pkgname}/x\"");
        assert!(tokens.contains(&(Hl::Expansion, "${pkgname}")));
        assert!(tokens.contains(&(Hl::Str, "/x\"")));
    }

    #[test]
    fn pkgbuild_tab_numbers_its_lines() {
        let mut st = two_packages_state();
        st.tab = Tab::Installed;
        st.right_tab = RightTab::Pkgbuild;
        st.pkgbuild = PkgbuildView::Ready("pkgname=foo\npkgver=1.2.3\n".to_string());
        let rows = render_at(&mut st, WIDE, TALL).join("\n");
        assert!(rows.contains("1 pkgname=foo"));
        assert!(rows.contains("2 pkgver=1.2.3"));
    }

    #[test]
    fn loading_and_error_states_show_their_messages() {
        let mut st = State::new("yay");
        st.right_tab = RightTab::Pkgbuild;
        st.pkgbuild = PkgbuildView::Loading;
        assert!(
            render_at(&mut st, WIDE, TALL)
                .join("\n")
                .contains("Cargando PKGBUILD…")
        );
        st.pkgbuild = PkgbuildView::Error("sin conexión".to_string());
        assert!(
            render_at(&mut st, WIDE, TALL)
                .join("\n")
                .contains("No se pudo obtener el PKGBUILD: sin conexión")
        );
    }
}

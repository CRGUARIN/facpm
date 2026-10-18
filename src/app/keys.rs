//! Teclas de cada modo: ayuda y noticias, modales, selector de tema, edición y modo normal.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{Action, App, Modal, OpKind, Tab};
use crate::backend::clean;
use crate::theme::{self, THEMES};

const PAGE: isize = 10;
/// Aviso cuando el tema necesita colores RGB y la terminal no lo anuncia.
const COLOR_WARNING: &str = "Tu terminal podría no mostrar bien colores RGB (COLORTERM)";

impl App {
    /// Procesa una tecla. Devuelve una acción solo cuando el usuario confirma una operación.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return None;
        }
        if self.state.modal.is_some() {
            return self.handle_modal_key(key);
        }
        if self.state.input_mode {
            self.handle_input_key(key);
        } else {
            self.handle_normal_key(key);
        }
        None
    }

    /// Con la ayuda (`close` = `?`) o las noticias (`close` = `n`) abiertas, esa tecla, Esc y `q`
    /// cierran sin salir de la app. j/k, J/K y Ctrl-d/Ctrl-u desplazan. Nada más se mueve.
    fn handle_overlay_key(&mut self, key: KeyEvent, close: char) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let half = (self.state.help_height / 2).max(1) as i16;
        match key.code {
            KeyCode::Char(c) if c == close => self.state.modal = None,
            KeyCode::Esc | KeyCode::Char('q') => self.state.modal = None,
            KeyCode::Char('d') if ctrl => self.scroll_help(half),
            KeyCode::Char('u') if ctrl => self.scroll_help(-half),
            KeyCode::Char('j' | 'J') | KeyCode::Down => self.scroll_help(1),
            KeyCode::Char('k' | 'K') | KeyCode::Up => self.scroll_help(-1),
            _ => {}
        }
    }

    fn scroll_help(&mut self, delta: i16) {
        self.state.help_scroll = self.state.help_scroll.saturating_add_signed(delta);
    }

    fn open_help(&mut self) {
        self.state.help_scroll = 0;
        self.state.modal = Some(Modal::Help);
    }

    fn open_news(&mut self) {
        self.state.help_scroll = 0;
        self.state.modal = Some(Modal::News);
    }

    /// Aplica el tema `id`; un id desconocido aplica el tema por defecto.
    pub fn set_theme(&mut self, id: &str) {
        self.state.theme = theme::by_id(id).unwrap_or_else(theme::default_theme);
    }

    /// Al arrancar: si el tema guardado necesita RGB y la terminal no lo anuncia, avisa en la
    /// barra de estado.
    pub fn warn_colors_at_startup(&mut self) {
        if self.color_warning_needed() {
            self.set_warning(COLOR_WARNING);
        }
    }

    fn color_warning_needed(&self) -> bool {
        self.state.theme.id != theme::default_theme().id && !self.truecolor
    }

    fn open_theme(&mut self) {
        let selected = theme::index_of(self.state.theme.id).unwrap_or(0);
        self.state.modal = Some(Modal::Theme {
            selected,
            original: self.state.theme.id,
        });
    }

    /// Con el selector abierto, j/k y las flechas eligen y aplican en vivo sin dar la vuelta.
    /// Enter guarda; Esc, q y t vuelven al tema original.
    fn handle_theme_key(&mut self, key: KeyEvent) {
        let (selected, original) = match &self.state.modal {
            Some(Modal::Theme { selected, original }) => (*selected, *original),
            _ => return,
        };
        let last = THEMES.len() - 1;
        let next = match key.code {
            KeyCode::Char('j') | KeyCode::Down => (selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => selected.saturating_sub(1),
            KeyCode::Enter => return self.confirm_theme(),
            KeyCode::Esc | KeyCode::Char('q' | 't') => return self.cancel_theme(original),
            _ => selected,
        };
        self.preview_theme(next);
    }

    fn preview_theme(&mut self, index: usize) {
        self.state.theme = &THEMES[index];
        if let Some(Modal::Theme { selected, .. }) = &mut self.state.modal {
            *selected = index;
        }
    }

    /// Enter: cierra el selector y guarda el tema. Si el guardado falla, el error va primero y
    /// el tema queda aplicado igual.
    fn confirm_theme(&mut self) {
        self.state.modal = None;
        let id = self.state.theme.id;
        let name = self.state.theme.name;
        match (self.theme_saver)(id) {
            Err(e) => self.set_status(format!("No se pudo guardar el tema: {e:#}"), false),
            Ok(()) if self.color_warning_needed() => {
                self.set_warning(format!("Tema: {name} · {COLOR_WARNING}"));
            }
            Ok(()) => self.set_status(format!("Tema: {name}"), true),
        }
    }

    fn cancel_theme(&mut self, original: &'static str) {
        self.state.theme = theme::by_id(original).unwrap_or_else(theme::default_theme);
        self.state.modal = None;
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.state.modal {
            Some(Modal::Help) => self.handle_overlay_key(key, '?'),
            Some(Modal::News) => self.handle_overlay_key(key, 'n'),
            Some(Modal::Theme { .. }) => self.handle_theme_key(key),
            _ => return self.answer_modal(key),
        }
        None
    }

    /// `y` o Enter confirman; `n` o Esc cancelan. Cualquier otra tecla deja el modal abierto.
    fn answer_modal(&mut self, key: KeyEvent) -> Option<Action> {
        let confirmed = matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter);
        let cancelled = matches!(key.code, KeyCode::Char('n' | 'N') | KeyCode::Esc);
        let modal = self.state.modal.take()?;
        if !confirmed {
            if !cancelled {
                self.state.modal = Some(modal);
            }
            return None;
        }
        match modal {
            Modal::Confirm { kind, pkgs, .. } => Some(Action::Run(self.build_command(kind, &pkgs))),
            Modal::Upgrade(_) => {
                self.pending_upgrade = true;
                Some(Action::Run(self.build_command(OpKind::Upgrade, &[])))
            }
            Modal::Clean { task, .. } => {
                let cmd = clean::command(&task, self.helper);
                if cmd.is_none() {
                    self.set_status("Esta limpieza ya no se puede ejecutar", false);
                }
                cmd.map(Action::Run)
            }
            Modal::Help | Modal::News | Modal::Theme { .. } => None,
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => self.state.input_mode = false,
            KeyCode::Enter => {
                self.state.input_mode = false;
                if self.state.tab == Tab::Search {
                    self.search_aur_now();
                }
            }
            KeyCode::Up => self.state.move_sel(-1),
            KeyCode::Down => self.state.move_sel(1),
            KeyCode::Backspace => {
                if self.text_mut().pop().is_some() {
                    self.on_text_changed();
                }
            }
            KeyCode::Char('u') if ctrl => {
                if !self.text_mut().is_empty() {
                    self.text_mut().clear();
                    self.on_text_changed();
                }
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.text_mut().push(c);
                self.on_text_changed();
            }
            _ => {}
        }
        self.refresh_details();
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let half = (self.state.right_height / 2).max(1) as i16;
            match key.code {
                KeyCode::Char('d') => self.scroll_right(half),
                KeyCode::Char('u') => self.scroll_right(-half),
                _ => {}
            }
            return;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('1') => self.switch_tab(Tab::Installed),
            KeyCode::Char('2') => self.switch_tab(Tab::Search),
            KeyCode::Char('3') => self.switch_tab(Tab::Updates),
            KeyCode::Char('4') => self.switch_tab(Tab::Clean),
            KeyCode::Char('h') | KeyCode::Left => self.switch_tab(self.state.tab.prev()),
            KeyCode::Char('l') | KeyCode::Right => self.switch_tab(self.state.tab.next()),
            KeyCode::Char('j') | KeyCode::Down => self.state.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.state.move_sel(-1),
            KeyCode::Char('g') | KeyCode::Home => self.state.go_to(0),
            KeyCode::Char('G') | KeyCode::End => self.state.go_to(usize::MAX),
            KeyCode::PageDown => self.state.move_sel(PAGE),
            KeyCode::PageUp => self.state.move_sel(-PAGE),
            KeyCode::Char('J') => self.scroll_right(1),
            KeyCode::Char('K') => self.scroll_right(-1),
            KeyCode::Char('p') => self.state.right_tab = self.state.right_tab.toggled(),
            KeyCode::Char('/') if matches!(self.state.tab, Tab::Installed | Tab::Search) => {
                self.state.input_mode = true
            }
            KeyCode::Char(' ') => self.toggle_mark(),
            KeyCode::Char('i') => self.request_install(),
            KeyCode::Char('d') | KeyCode::Char('x') => self.request_remove(),
            KeyCode::Char('c') | KeyCode::Enter if self.state.tab == Tab::Clean => {
                self.request_clean()
            }
            KeyCode::Char('u') => self.request_upgrade(),
            KeyCode::Char('r') => {
                self.reload();
                match self.state.tab {
                    Tab::Updates => {
                        self.check_updates();
                        self.spawn_news();
                    }
                    Tab::Clean => self.spawn_clean_scan(),
                    _ => {}
                }
            }
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('n') => self.open_news(),
            KeyCode::Char('t') => self.open_theme(),
            KeyCode::Esc => self.state.status = None,
            _ => {}
        }
        self.refresh_details();
    }

    fn scroll_right(&mut self, delta: i16) {
        self.state.right_scroll = self.state.right_scroll.saturating_add_signed(delta);
    }

    fn switch_tab(&mut self, tab: Tab) {
        if self.state.tab == tab {
            return;
        }
        self.state.tab = tab;
        if tab == Tab::Search && self.state.query.is_empty() {
            self.state.input_mode = true;
        }
        if tab == Tab::Clean && !self.clean_opened {
            self.spawn_clean_scan();
        }
        self.state.clamp_lists();
    }

    fn text_mut(&mut self) -> &mut String {
        match self.state.tab {
            Tab::Installed | Tab::Updates | Tab::Clean => &mut self.state.filter,
            Tab::Search => &mut self.state.query,
        }
    }

    fn on_text_changed(&mut self) {
        match self.state.tab {
            Tab::Installed => {
                self.state.refilter();
                self.state.select_first_row();
            }
            Tab::Search => self.on_query_changed(),
            Tab::Updates | Tab::Clean => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use crate::app::{Level, RightTab};

    fn only_kanagawa(id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(id == "kanagawa", "se guardó {id}");
        Ok(())
    }

    #[test]
    fn help_closes_with_esc_and_q_but_q_does_not_quit() {
        let mut app = empty_app();
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Esc);
        assert!(app.state.modal.is_none());

        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('q'));
        assert!(app.state.modal.is_none(), "q cierra la ayuda");
        assert!(!app.should_quit(), "q con la ayuda abierta no sale");
    }

    #[test]
    fn ctrl_c_quits_even_with_help_open() {
        let mut app = empty_app();
        press(&mut app, KeyCode::Char('?'));
        app.handle_key(ctrl('c'));
        assert!(app.should_quit());
    }

    #[test]
    fn help_scrolls_with_jk_and_half_pages_without_moving_the_list() {
        let mut app = empty_app();
        app.state
            .set_installed(vec![repo_pkg("zzfacpm-a"), repo_pkg("zzfacpm-b")]);
        app.refresh_details();
        let selected = app.state.sel();
        app.state.help_height = 10;
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.state.help_scroll, 1);
        assert_eq!(app.state.sel(), selected, "la lista no se mueve");

        app.handle_key(ctrl('d'));
        assert_eq!(app.state.help_scroll, 6);
        app.handle_key(ctrl('u'));
        app.handle_key(ctrl('u'));
        app.handle_key(ctrl('u'));
        assert_eq!(app.state.help_scroll, 0, "no baja de cero");
    }

    #[test]
    fn news_opens_with_n_and_closes_without_quitting() {
        let mut app = empty_app();
        press(&mut app, KeyCode::Char('n'));
        assert!(matches!(app.state.modal, Some(Modal::News)));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.state.help_scroll, 1, "j desplaza");
        assert_eq!(app.state.sel(), None, "la lista no se mueve");
        press(&mut app, KeyCode::Char('n'));
        assert!(app.state.modal.is_none(), "n cierra");
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Esc);
        assert!(app.state.modal.is_none(), "Esc cierra");
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char('q'));
        assert!(app.state.modal.is_none(), "q cierra");
        assert!(!app.should_quit(), "q no sale");
    }

    #[test]
    fn theme_selector_previews_and_esc_q_t_restore_the_original() {
        for key in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('t')] {
            let mut app = empty_app();
            app.set_theme("kanagawa");
            press(&mut app, KeyCode::Char('t'));
            assert!(matches!(
                app.state.modal,
                Some(Modal::Theme {
                    selected: 1,
                    original: "kanagawa"
                })
            ));
            press(&mut app, KeyCode::Char('j'));
            assert_eq!(app.state.theme.id, "monokai", "vista previa en vivo");
            press(&mut app, key);
            assert!(app.state.modal.is_none(), "{key:?} cierra");
            assert_eq!(app.state.theme.id, "kanagawa", "{key:?} vuelve al original");
            assert!(!app.should_quit(), "{key:?} no sale");
        }
    }

    #[test]
    fn enter_saves_the_chosen_theme_and_reports_it() {
        let mut app = empty_app();
        app.theme_saver = only_kanagawa;
        app.truecolor = true;
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(app.state.modal.is_none());
        assert_eq!(status_text(&app), "Tema: Kanagawa");
        assert_eq!(level(&app), Some(Level::Ok));
    }

    #[test]
    fn failing_save_reports_error_and_keeps_the_theme() {
        let mut app = empty_app();
        app.theme_saver = failing_theme_save;
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(status_text(&app).contains("No se pudo guardar el tema"));
        assert_eq!(level(&app), Some(Level::Error));
        assert_eq!(app.state.theme.id, "kanagawa", "queda aplicado igual");
    }

    #[test]
    fn rgb_theme_without_truecolor_warns_and_terminal_never_does() {
        let mut app = empty_app();
        app.truecolor = false;
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(status_text(&app).starts_with("Tema: Kanagawa · "));
        assert!(status_text(&app).contains("COLORTERM"));
        assert_eq!(level(&app), Some(Level::Warn));

        let mut app = empty_app();
        app.truecolor = false;
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(status_text(&app), "Tema: Terminal");
        assert_eq!(level(&app), Some(Level::Ok));
    }

    #[test]
    fn startup_warns_only_for_rgb_theme_without_truecolor() {
        let mut app = empty_app();
        app.truecolor = false;
        app.set_theme("kanagawa");
        app.warn_colors_at_startup();
        assert!(status_text(&app).contains("COLORTERM"));
        assert_eq!(level(&app), Some(Level::Warn));

        let mut app = empty_app();
        app.truecolor = true;
        app.set_theme("kanagawa");
        app.warn_colors_at_startup();
        assert!(app.state.status.is_none());
    }

    #[test]
    fn esc_leaves_edit_mode_and_keeps_the_filter() {
        let mut app = empty_app();
        app.state.tab = Tab::Installed;
        app.state.input_mode = true;
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Esc);
        assert!(!app.state.input_mode);
        assert_eq!(app.state.filter, "g");
    }

    #[test]
    fn tabs_rotate_with_h_l_and_jump_with_1_to_4() {
        let mut app = empty_app();
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.state.tab, Tab::Updates);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.state.tab, Tab::Clean);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.state.tab, Tab::Installed, "l da la vuelta");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.state.tab, Tab::Clean, "h da la vuelta");
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.state.tab, Tab::Search);
        assert!(app.state.input_mode, "Buscar entra en modo edición");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.state.tab, Tab::Installed);
    }

    #[test]
    fn p_toggles_the_right_tab_and_jk_scroll_the_panel_until_the_selection_changes() {
        let mut app = empty_app();
        app.state.right_scroll = 4;
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.state.right_tab, RightTab::Pkgbuild);
        assert_eq!(app.state.right_scroll, 0);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.state.right_tab, RightTab::Details);

        app.state
            .set_installed(vec![repo_pkg("zzfacpm-a"), repo_pkg("zzfacpm-b")]);
        app.refresh_details();
        press(&mut app, KeyCode::Char('J'));
        press(&mut app, KeyCode::Char('J'));
        press(&mut app, KeyCode::Char('K'));
        assert_eq!(app.state.right_scroll, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.state.right_scroll, 0,
            "cambiar de paquete resetea el scroll"
        );
    }
}

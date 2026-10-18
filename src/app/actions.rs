//! Acciones sobre paquetes: marcar, instalar, desinstalar, armar el comando y recargar la base.

use std::process::Command;

use super::{App, Modal, OpKind, Tab};
use crate::backend::helper::command_line;
use crate::pkg::{Package, Source};

const PARTIAL_UPDATE_MSG: &str =
    "Las actualizaciones parciales no están soportadas: usa u para actualizar todo el sistema";

impl App {
    pub(super) fn toggle_mark(&mut self) {
        if self.state.tab == Tab::Clean {
            return;
        }
        if self.partial_update_blocked() {
            return;
        }
        let Some(name) = self.state.selected().map(|p| p.name.clone()) else {
            return;
        };
        let idx = self.state.tab.idx();
        let set = &mut self.state.marked[idx];
        if !set.remove(&name) {
            set.insert(name);
        }
        self.state.move_sel(1);
    }

    /// En Actualizaciones, espacio, `i` y `d` no hacen nada: solo avisan. Devuelve true si la
    /// tecla se bloqueó.
    fn partial_update_blocked(&mut self) -> bool {
        if self.state.tab != Tab::Updates {
            return false;
        }
        self.set_status(PARTIAL_UPDATE_MSG, false);
        true
    }

    /// Paquetes sobre los que actúan `i` y `d`: los marcados visibles o, si no hay, el actual.
    fn targets(&self) -> Vec<Package> {
        let marked = &self.state.marked[self.state.tab.idx()];
        let picked: Vec<Package> = self
            .state
            .rows()
            .into_iter()
            .filter(|p| marked.contains(&p.name))
            .cloned()
            .collect();
        if !picked.is_empty() {
            return picked;
        }
        self.state.selected().cloned().into_iter().collect()
    }

    pub(super) fn request_install(&mut self) {
        if self.state.tab == Tab::Clean {
            return;
        }
        if self.partial_update_blocked() {
            return;
        }
        if self.state.tab != Tab::Search {
            return;
        }
        let targets = self.targets();
        if targets.is_empty() {
            self.set_status("No hay ningún paquete seleccionado", false);
            return;
        }
        if !self.helper.supports_aur() && targets.iter().any(|p| p.source == Source::Aur) {
            let msg = format!("El helper actual ({}) no soporta AUR", self.helper.name());
            self.set_status(msg, false);
            return;
        }
        let pkgs = targets.into_iter().map(|p| p.name).collect();
        self.open_confirm(OpKind::Install, pkgs);
    }

    pub(super) fn request_remove(&mut self) {
        if self.state.tab == Tab::Clean {
            return;
        }
        if self.partial_update_blocked() {
            return;
        }
        let targets = self.targets();
        if targets.is_empty() {
            self.set_status("No hay ningún paquete seleccionado", false);
            return;
        }
        let pkgs: Vec<String> = targets
            .into_iter()
            .filter(|p| p.installed.is_some())
            .map(|p| p.name)
            .collect();
        if pkgs.is_empty() {
            self.set_status("Ningún paquete seleccionado está instalado", false);
            return;
        }
        self.open_confirm(OpKind::Remove, pkgs);
    }

    fn open_confirm(&mut self, kind: OpKind, pkgs: Vec<String>) {
        let command = command_line(&self.build_command(kind, &pkgs));
        self.state.modal = Some(Modal::Confirm {
            kind,
            pkgs,
            command,
        });
    }

    pub(super) fn build_command(&self, kind: OpKind, pkgs: &[String]) -> Command {
        match kind {
            OpKind::Install => self.helper.install_command(pkgs),
            OpKind::Remove => self.helper.remove_command(pkgs),
            OpKind::Upgrade => self.helper.upgrade_command(),
        }
    }

    /// Recarga la base de pacman. `r` también rehace la búsqueda del AUR si hay una consulta.
    pub(super) fn reload(&mut self) {
        if let Err(e) = self.repo.reload() {
            self.set_status(format!("No se pudo recargar: {e:#}"), false);
            return;
        }
        self.refresh_db();
        if self.state.tab == Tab::Search && !self.state.query.trim().is_empty() {
            self.aur_last_query = None;
            self.search_aur_now();
        }
        self.set_status("Base de datos recargada", true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Level;
    use crate::app::test_util::*;
    use crate::backend::helper::Helper;
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn partial_update_keys_only_warn_in_updates_tab() {
        let mut app = empty_app();
        press(&mut app, KeyCode::Char('3'));
        for c in [' ', 'i', 'd'] {
            press(&mut app, KeyCode::Char(c));
            assert!(status_text(&app).contains("parciales"), "tecla {c:?}");
        }
        assert!(app.state.marked[Tab::Updates.idx()].is_empty());
        assert!(app.state.modal.is_none());
    }

    #[test]
    fn remove_refuses_packages_that_are_not_installed() {
        let mut app = empty_app();
        app.state.tab = Tab::Search;
        app.state.results = vec![Package {
            name: FAKE.to_string(),
            version: "1.0-1".to_string(),
            description: String::new(),
            source: Source::Repo("extra".to_string()),
            installed: None,
        }];
        app.state.clamp_lists();
        press(&mut app, KeyCode::Char('d'));
        assert!(app.state.modal.is_none());
        assert_eq!(
            status_text(&app),
            "Ningún paquete seleccionado está instalado"
        );
    }

    #[test]
    fn install_from_aur_needs_a_helper_that_supports_it() {
        let mut app = app_with(Helper::Pacman);
        app.state.tab = Tab::Search;
        app.state.results = vec![Package {
            name: FAKE.to_string(),
            version: "1.0-1".to_string(),
            description: String::new(),
            source: Source::Aur,
            installed: None,
        }];
        app.state.clamp_lists();
        press(&mut app, KeyCode::Char('i'));
        assert!(app.state.modal.is_none());
        assert_eq!(
            status_text(&app),
            "El helper actual (pacman) no soporta AUR"
        );
        assert_eq!(level(&app), Some(Level::Error));
    }
}

//! Package actions: mark, install, uninstall, build the command and reload the database.

use std::process::Command;

use super::{App, Modal, OpKind, Tab};
use crate::backend::helper::command_line;
use crate::pkg::{Package, Source};

const PARTIAL_UPDATE_MSG: &str =
    "Partial upgrades aren't supported: press U to upgrade the whole system";

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

    /// In Updates, space, `i` and `u` do nothing but warn. Returns true if the key
    /// was blocked.
    fn partial_update_blocked(&mut self) -> bool {
        if self.state.tab != Tab::Updates {
            return false;
        }
        self.set_status(PARTIAL_UPDATE_MSG, false);
        true
    }

    /// Packages that `i` and `u` act on: the visible marked ones or, if none, the current one.
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
            self.set_status("No package selected", false);
            return;
        }
        if !self.helper.supports_aur() && targets.iter().any(|p| p.source == Source::Aur) {
            let msg = format!(
                "The current helper ({}) doesn't support the AUR",
                self.helper.name()
            );
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
            self.set_status("No package selected", false);
            return;
        }
        let pkgs: Vec<String> = targets
            .into_iter()
            .filter(|p| p.installed.is_some())
            .map(|p| p.name)
            .collect();
        if pkgs.is_empty() {
            self.set_status("None of the selected packages is installed", false);
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

    /// Reloads the pacman database. `r` also redoes the AUR search if there's a query.
    pub(super) fn reload(&mut self) {
        if let Err(e) = self.repo.reload() {
            self.set_status(format!("Couldn't reload: {e:#}"), false);
            return;
        }
        self.refresh_db();
        if self.state.tab == Tab::Search && !self.state.query.trim().is_empty() {
            self.aur_last_query = None;
            self.search_aur_now();
        }
        self.set_status("Database reloaded", true);
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
        for c in [' ', 'i', 'u'] {
            press(&mut app, KeyCode::Char(c));
            assert!(status_text(&app).contains("Partial upgrades"), "key {c:?}");
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
        press(&mut app, KeyCode::Char('u'));
        assert!(app.state.modal.is_none());
        assert_eq!(
            status_text(&app),
            "None of the selected packages is installed"
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
            "The current helper (pacman) doesn't support the AUR"
        );
        assert_eq!(level(&app), Some(Level::Error));
    }
}

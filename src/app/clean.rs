//! Cleanup: each task's modal and the background scan, which keeps the selection by
//! task kind.

use super::{App, Modal, Tab, clean_row};
use crate::backend::clean::CleanTask;

impl App {
    pub(super) fn request_clean(&mut self) {
        let Some(row) = self.state.selected_clean().cloned() else {
            return;
        };
        match row.command {
            Some(command) if row.task.available => {
                self.state.modal = Some(Modal::Clean {
                    task: row.task,
                    command,
                });
            }
            _ => {
                let msg = row
                    .task
                    .note
                    .clone()
                    .unwrap_or_else(|| "Nothing to clean".to_string());
                self.set_status(msg, false);
            }
        }
    }

    /// Starts the Cleanup scan. The orphans are computed here (alpm is fast); the scan itself,
    /// which can take a while, runs in a thread with the current generation.
    pub(super) fn spawn_clean_scan(&mut self) {
        self.clean_opened = true;
        // Remember the selected task, whether or not this tab is active.
        let current = self.state.lists[Tab::Clean.idx()]
            .selected()
            .and_then(|i| self.state.clean.get(i))
            .map(|row| row.task.kind);
        if current.is_some() {
            self.clean_keep = current;
        }
        self.clean_gen += 1;
        let generation = self.clean_gen;
        let orphans = self.repo.orphans();
        self.state.orphans = orphans.clone();
        self.state.clean.clear();
        self.state.clean_pending = true;
        self.state.clamp_lists();
        let scanner = self.clean_scanner;
        let helper = self.helper;
        let tx = self.clean_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((generation, scanner(orphans, helper)));
        });
        self.refresh_details();
    }

    pub(super) fn on_clean_scan(&mut self, generation: u64, tasks: Vec<CleanTask>) {
        if generation != self.clean_gen {
            return;
        }
        self.state.clean = tasks
            .into_iter()
            .map(|task| clean_row(task, self.helper))
            .collect();
        self.state.clean_pending = false;
        self.state.clamp_lists();
        if let Some(kind) = self.clean_keep
            && let Some(i) = self
                .state
                .clean
                .iter()
                .position(|row| row.task.kind == kind)
        {
            self.state.lists[Tab::Clean.idx()].select(Some(i));
        }
        self.refresh_details();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Action;
    use crate::app::state::sample_clean_tasks;
    use crate::app::test_util::*;
    use crate::backend::clean::CleanKind;
    use crate::backend::helper::Helper;
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn clean_scan_fills_rows_with_their_commands() {
        let mut app = app_with(Helper::Yay);
        press(&mut app, KeyCode::Char('4'));
        let generation = app.clean_gen;
        app.on_clean_scan(generation, sample_clean_tasks());
        assert!(!app.state.clean_pending);
        assert_eq!(app.state.clean.len(), 6);
        assert_eq!(app.state.clean[0].task.kind, CleanKind::Orphans);
        assert_eq!(
            app.state.clean[0].command.as_deref(),
            Some("yay -Rns go libxml2-legacy")
        );
        assert_eq!(
            app.state.clean[2].command, None,
            "unavailable tasks have no command"
        );
    }

    #[test]
    fn stale_clean_scan_is_discarded() {
        let mut app = empty_app();
        press(&mut app, KeyCode::Char('4'));
        let stale = app.clean_gen + 7;
        app.on_clean_scan(stale, sample_clean_tasks());
        assert!(app.state.clean.is_empty());
        assert!(app.state.clean_pending, "still scanning");
    }

    #[test]
    fn enter_on_keep_latest_opens_the_modal_and_y_runs_paccache() {
        let mut app = app_with(Helper::Yay);
        press(&mut app, KeyCode::Char('4'));
        let generation = app.clean_gen;
        app.on_clean_scan(generation, sample_clean_tasks());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        match &app.state.modal {
            Some(Modal::Clean { task, command }) => {
                assert_eq!(task.kind, CleanKind::PkgCacheKeepLatest);
                assert_eq!(command, "sudo paccache -rk1");
            }
            other => panic!("{other:?}"),
        }
        match press(&mut app, KeyCode::Char('y')) {
            Some(Action::Run(cmd)) => {
                assert_eq!(
                    crate::backend::helper::command_line(&cmd),
                    "sudo paccache -rk1"
                )
            }
            None => panic!("confirming must return the action"),
        }
        assert!(app.state.modal.is_none());
    }

    #[test]
    fn c_on_an_unavailable_task_sets_its_note_without_a_modal() {
        let mut app = app_with(Helper::Yay);
        press(&mut app, KeyCode::Char('4'));
        let generation = app.clean_gen;
        app.on_clean_scan(generation, sample_clean_tasks());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('c'));
        assert!(app.state.modal.is_none());
        let note = sample_clean_tasks()[2]
            .note
            .clone()
            .expect("the sample task has a note");
        assert!(status_text(&app).contains(&note));
    }

    #[test]
    fn rescan_keeps_the_selected_task_by_kind_not_by_position() {
        let mut app = app_with(Helper::Yay);
        press(&mut app, KeyCode::Char('4'));
        let generation = app.clean_gen;
        app.on_clean_scan(generation, sample_clean_tasks());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Char('r'));
        assert!(app.state.clean.is_empty() && app.state.clean_pending);

        let generation = app.clean_gen;
        let mut reversed = sample_clean_tasks();
        reversed.reverse();
        app.on_clean_scan(generation, reversed);
        let selected = app.state.selected_clean().expect("there is a selection");
        assert_eq!(selected.task.kind, CleanKind::PkgCacheKeepLatest);
    }
}

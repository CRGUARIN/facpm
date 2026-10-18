use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use facpm::app::{Action, App};
use facpm::backend::helper::Helper;
use facpm::backend::repo::RepoDb;
use facpm::{config, tui, ui};
use ratatui::crossterm::event::{self, Event, KeyEventKind};

fn main() -> ExitCode {
    let repo = match RepoDb::new() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("facpm: no se pudo abrir la base de datos de paquetes: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let helper = Helper::detect();
    let mut terminal = tui::init();
    let mut app = App::new(repo, helper);
    let config = config::Config::load();
    app.set_theme(&config.theme);
    app.warn_colors_at_startup();
    app.start_background();
    let result = event_loop(&mut terminal, &mut app);
    tui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("facpm: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn event_loop(terminal: &mut tui::Term, app: &mut App) -> Result<()> {
    while !app.should_quit() {
        terminal.draw(|frame| ui::draw(frame, app.state_mut()))?;

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(Action::Run(cmd)) = app.handle_key(key)
        {
            let ok = tui::run_external(terminal, cmd)?;
            app.after_operation(ok);
        }

        app.tick();
    }
    Ok(())
}

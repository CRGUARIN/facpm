use std::io::{self, Write};
use std::process::Command;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

pub type Term = ratatui::DefaultTerminal;

pub fn init() -> Term {
    ratatui::init()
}

pub fn restore() {
    ratatui::restore()
}

/// Corre `cmd` fuera de la TUI y espera Enter. Devuelve si terminó con éxito.
pub fn run_external(terminal: &mut Term, mut cmd: Command) -> anyhow::Result<bool> {
    let line = crate::backend::helper::command_line(&cmd);
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, Show)?;

    println!();
    println!("==> {line}");
    let ok = match cmd.status() {
        Ok(status) if status.success() => {
            println!("\n==> Terminó con éxito.");
            true
        }
        Ok(status) => {
            println!("\n==> Terminó con error ({status}).");
            false
        }
        Err(e) => {
            println!("\nNo se pudo ejecutar el comando: {e}");
            false
        }
    };
    println!("Presiona Enter para volver a facpm");
    io::stdout().flush()?;
    let mut enter = String::new();
    io::stdin().read_line(&mut enter)?;

    execute!(io::stdout(), EnterAlternateScreen)?;
    enable_raw_mode()?;
    while event::poll(Duration::ZERO)? {
        event::read()?;
    }
    // `terminal.clear()` consulta el cursor, y tras un comando externo esa consulta puede fallar.
    execute!(io::stdout(), Clear(ClearType::All))?;
    *terminal = ratatui::Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(ok)
}

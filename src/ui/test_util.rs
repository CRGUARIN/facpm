//! Estados de ejemplo y renderizado a texto o a buffer para los tests de `ui`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::app::{
    Modal, NewsState, OpKind, State, Tab, UpgradePlan, clean_row, sample_clean_tasks,
};
use crate::backend::helper::Helper;
use crate::backend::news::NewsItem;
use crate::backend::warnings::{self, Kind, Severity, Warning};
use crate::pkg::{Package, Source};

pub(super) const WIDE: u16 = 120;
pub(super) const TALL: u16 = 32;
pub(super) const NEWS_SINCE: i64 = 1_700_000_000;
pub(super) const NEWS_AFTER: i64 = 1_700_100_000;

pub(super) fn pkg(name: &str, version: &str, source: Source, installed: Option<&str>) -> Package {
    Package {
        name: name.to_string(),
        version: version.to_string(),
        description: format!("Descripción de {name}"),
        source,
        installed: installed.map(str::to_string),
    }
}

pub(super) fn update_pkg(name: &str, current: &str, new: &str, source: Source) -> Package {
    pkg(name, new, source, Some(current))
}

pub(super) fn updates_state() -> State {
    let mut st = State::new("yay");
    st.tab = Tab::Updates;
    st.updates = vec![
        update_pkg(
            "firefox",
            "131.0-1",
            "131.0-2",
            Source::Repo("extra".into()),
        ),
        update_pkg(
            "ollama",
            "0.40.5-1",
            "0.40.6-1",
            Source::Repo("extra".into()),
        ),
        update_pkg("yay", "12.4.2-1", "12.5.0-1", Source::Aur),
    ];
    st.upgradable = st.updates.iter().map(|p| p.name.clone()).collect();
    st.clamp_lists();
    st
}

pub(super) fn two_packages_state() -> State {
    let mut st = State::new("yay");
    st.set_installed(vec![
        pkg(
            "firefox",
            "131.0-1",
            Source::Repo("extra".into()),
            Some("131.0-1"),
        ),
        pkg(
            "git",
            "2.47.0-1",
            Source::Repo("extra".into()),
            Some("2.47.0-1"),
        ),
    ]);
    st
}

pub(super) fn clean_state() -> State {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    let mut st = State::new("yay");
    st.tab = Tab::Clean;
    st.clean = sample_clean_tasks()
        .into_iter()
        .map(|task| clean_row(task, Helper::Yay))
        .collect();
    st.orphans = vec![
        ("go".to_string(), 226 * MIB + 614 * KIB),
        ("libxml2-legacy".to_string(), MIB),
    ];
    st.clamp_lists();
    st
}

pub(super) fn test_warning(kind: Kind, severity: Severity, label: &str) -> Warning {
    Warning {
        kind,
        severity,
        label: label.to_string(),
        text: format!("{label}: explicación de prueba."),
    }
}

pub(super) fn news_item(title: &str, published: i64) -> NewsItem {
    NewsItem {
        title: title.to_string(),
        link: "https://archlinux.org/news/prueba/".to_string(),
        published,
        date: warnings::date(published),
        body: "Cuerpo de la noticia de prueba.".to_string(),
    }
}

pub(super) fn news_loaded(items: Vec<NewsItem>) -> NewsState {
    NewsState::Loaded {
        items,
        since: NEWS_SINCE,
    }
}

pub(super) fn all_scenarios() -> Vec<State> {
    let mut out = vec![two_packages_state(), updates_state(), clean_state()];
    let mut search = two_packages_state();
    search.tab = Tab::Search;
    out.push(search);

    let modals = [
        Modal::Help,
        Modal::News,
        Modal::Theme {
            selected: 0,
            original: "terminal",
        },
        Modal::Confirm {
            kind: OpKind::Install,
            pkgs: vec!["firefox".to_string()],
            command: "yay -S firefox".to_string(),
        },
        Modal::Upgrade(UpgradePlan {
            repos: 2,
            aur: 1,
            lines: vec!["firefox 131.0-1 → 131.0-2".to_string()],
            more: 0,
            command: "yay -Syu".to_string(),
            pacman_warning: false,
            flagged: vec![(
                "yay".to_string(),
                Severity::Alarm,
                "cambió el mantenedor".to_string(),
            )],
            news_new: 1,
            news_intervention: true,
        }),
    ];
    for modal in modals {
        let mut st = two_packages_state();
        st.news = news_loaded(vec![news_item("Título de prueba", NEWS_AFTER)]);
        st.modal = Some(modal);
        out.push(st);
    }
    out
}

/// Renderiza `st` en una terminal de prueba de `w` x `h` y devuelve sus filas como texto.
pub(super) fn render_at(st: &mut State, w: u16, h: u16) -> Vec<String> {
    let buf = render_buffer(st, w, h);
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

/// Renderiza `st` y devuelve el buffer completo, para revisar estilos celda por celda.
pub(super) fn render_buffer(st: &mut State, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| super::draw(f, st)).unwrap();
    term.backend().buffer().clone()
}

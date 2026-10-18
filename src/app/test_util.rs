//! Helpers for the `app` tests: an app with fake savers and made-up packages.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::backend::news::NewsItem;
use crate::backend::warnings::Kind;
use crate::pkg::Source;

pub(super) const FAKE: &str = "zzfacpm-test";
pub(super) const BEFORE: i64 = 1_600_000_000;
pub(super) const SINCE: i64 = 1_700_000_000;
pub(super) const AFTER: i64 = 1_700_100_000;

/// App with fake savers and scanner: it writes no config and reads no cleanup data. The pacman
/// database is opened as it is on this machine, read-only.
pub(super) fn app_with(helper: Helper) -> App {
    let mut app = App::new(RepoDb::with_repo_list(no_repos).unwrap(), helper);
    app.clean_scanner = fake_scan;
    app.repo_checker = fake_checker;
    app.theme_saver = ok_theme_save;
    app.maintainers_saver = ok_maintainers_save;
    app
}

pub(super) fn empty_app() -> App {
    let mut app = app_with(Helper::Yay);
    app.state.set_installed(Vec::new());
    app.refresh_details();
    app
}

pub(super) fn press(app: &mut App, code: KeyCode) -> Option<Action> {
    app.handle_key(KeyEvent::from(code))
}

pub(super) fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub(super) fn fake_scan(_orphans: Vec<(String, u64)>, _helper: Helper) -> Vec<CleanTask> {
    Vec::new()
}

/// No synced repos: only the local database is read and `pacman-conf` doesn't run.
pub(super) fn no_repos() -> anyhow::Result<Vec<String>> {
    Ok(Vec::new())
}

pub(super) fn fake_checker() -> anyhow::Result<Option<Vec<RawUpdate>>> {
    Ok(Some(Vec::new()))
}

pub(super) fn ok_theme_save(_id: &str) -> anyhow::Result<()> {
    Ok(())
}

pub(super) fn failing_theme_save(_id: &str) -> anyhow::Result<()> {
    anyhow::bail!("disco lleno")
}

pub(super) fn ok_maintainers_save(_m: &Maintainers) -> anyhow::Result<()> {
    Ok(())
}

pub(super) fn failing_maintainers_save(_m: &Maintainers) -> anyhow::Result<()> {
    anyhow::bail!("disco lleno")
}

pub(super) fn aur_pkg_version(name: &str, base: &str, version: &str, depends: &[&str]) -> AurPkg {
    aur_json(name, base, version, 1, Some("nobody"), depends)
}

pub(super) fn aur_pkg(name: &str, depends: &[&str]) -> AurPkg {
    aur_pkg_version(name, name, "1.0-1", depends)
}

pub(super) fn aur_pkg_maintained(name: &str, version: &str, maintainer: Option<&str>) -> AurPkg {
    aur_json(name, name, version, 500, maintainer, &[])
}

fn aur_json(
    name: &str,
    base: &str,
    version: &str,
    votes: u64,
    maintainer: Option<&str>,
    depends: &[&str],
) -> AurPkg {
    serde_json::from_value(serde_json::json!({
        "Name": name,
        "Version": version,
        "Description": "test package",
        "URL": null,
        "NumVotes": votes,
        "Popularity": 0.5,
        "Maintainer": maintainer,
        "OutOfDate": null,
        "FirstSubmitted": 0,
        "LastModified": 0,
        "PackageBase": base,
        "Depends": depends,
    }))
    .unwrap()
}

pub(super) fn repo_pkg(name: &str) -> Package {
    Package {
        name: name.to_string(),
        version: "1.0-1".to_string(),
        description: String::new(),
        source: Source::Repo("extra".to_string()),
        installed: Some("1.0-1".to_string()),
    }
}

pub(super) fn foreign_pkg(name: &str, version: &str) -> Package {
    Package {
        name: name.to_string(),
        version: version.to_string(),
        description: String::new(),
        source: Source::Foreign,
        installed: Some(version.to_string()),
    }
}

pub(super) fn aur_update(name: &str, current: &str, new: &str) -> Update {
    Update {
        name: name.to_string(),
        current: current.to_string(),
        new: new.to_string(),
        source: Source::Aur,
    }
}

pub(super) fn news_item(title: &str, published: i64) -> NewsItem {
    NewsItem {
        title: title.to_string(),
        link: "https://archlinux.org/news/test/".to_string(),
        published,
        date: warnings::date(published),
        body: "Body of a test news item.".to_string(),
    }
}

pub(super) fn aur_selected_app(base: &str) -> App {
    let mut app = empty_app();
    app.state.tab = Tab::Search;
    app.aur_pkgs = vec![aur_pkg_version(FAKE, base, "1.0-1", &[])];
    app.state.query = FAKE.to_string();
    app.aur_last_query = Some(FAKE.to_string());
    app.state.aur_state = AurState::Done {
        count: 1,
        local: false,
    };
    app.rebuild_results();
    app.state.select_first_row();
    app.refresh_details();
    app
}

pub(super) fn status_text(app: &App) -> String {
    app.state
        .status
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default()
}

pub(super) fn level(app: &App) -> Option<Level> {
    app.state.status.as_ref().map(|s| s.level)
}

pub(super) fn kinds_of(app: &App, name: &str) -> Vec<Kind> {
    app.state
        .warnings
        .get(name)
        .map(|ws| ws.iter().map(|w| w.kind).collect())
        .unwrap_or_default()
}

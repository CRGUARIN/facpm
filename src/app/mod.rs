//! The app: the state the UI reads, plus the backend. Keys, actions, searches,
//! update checks and cleanup live in submodules that share `App`.

mod actions;
mod clean;
mod keys;
mod search;
mod state;
#[cfg(test)]
mod test_util;
mod updates;

pub use state::*;

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use crate::backend::aur::AurPkg;
use crate::backend::aur_index::AurIndex;
use crate::backend::clean::{CleanKind, CleanTask};
use crate::backend::helper::Helper;
use crate::backend::news::NewsItem;
use crate::backend::pkgbuild::PkgbuildSource;
use crate::backend::repo::RepoDb;
use crate::backend::updates::{RawUpdate, Update};
use crate::backend::warnings::{self, Maintainers};
use crate::pkg::Package;

// Thread replies. A reply with an old generation is dropped.
type AurMsg = (u64, anyhow::Result<Vec<AurPkg>>);
type InfoMsg = (String, anyhow::Result<Vec<AurPkg>>);
type PkgKey = (PkgbuildSource, String);
type PkgbuildMsg = (PkgKey, Result<String, String>);
type RepoCheckMsg = (u64, anyhow::Result<Option<Vec<RawUpdate>>>);
type AurCheckMsg = (u64, anyhow::Result<Vec<Update>>);
type CleanScanMsg = (u64, Vec<CleanTask>);
type NewsMsg = (u64, anyhow::Result<Vec<NewsItem>>, Option<i64>);
type ViewKey = (Tab, RightTab, Option<String>);
type UpdCheck = Option<Result<Vec<Update>, String>>;
type ScanFn = fn(Vec<(String, u64)>, Helper) -> Vec<CleanTask>;
/// Checks the repos: `checkupdates` in production, a fixed reply in tests.
type RepoChecker = fn() -> anyhow::Result<Option<Vec<RawUpdate>>>;
type MaintainersSaver = fn(&Maintainers) -> anyhow::Result<()>;
type ThemeSaver = fn(&str) -> anyhow::Result<()>;

pub enum Action {
    Run(Command),
}

pub enum IndexMsg {
    Loaded(AurIndex),
    Error(String),
}

pub struct App {
    repo: RepoDb,
    helper: Helper,
    state: State,
    repo_results: Vec<Package>,
    aur_pkgs: Vec<AurPkg>,
    aur_last_query: Option<String>,
    aur_tx: Sender<AurMsg>,
    aur_rx: Receiver<AurMsg>,
    aur_generation: u64,
    aur_deadline: Option<Instant>,
    aur_index: Option<AurIndex>,
    index_tx: Sender<IndexMsg>,
    index_rx: Receiver<IndexMsg>,
    aur_info: HashMap<String, AurPkg>,
    info_inflight: HashSet<String>,
    info_deadline: Option<(Instant, String)>,
    info_tx: Sender<InfoMsg>,
    info_rx: Receiver<InfoMsg>,
    view_key: Option<ViewKey>,
    pkgbuild_target: Option<Result<PkgKey, String>>,
    pkgbuild_cache: HashMap<PkgKey, Result<String, String>>,
    pkgbuild_inflight: HashSet<PkgKey>,
    pkgbuild_deadline: Option<(Instant, PkgKey)>,
    pkgbuild_tx: Sender<PkgbuildMsg>,
    pkgbuild_rx: Receiver<PkgbuildMsg>,
    /// Generation of the last repo check; older replies are dropped.
    repo_check_gen: u64,
    repo_checker: RepoChecker,
    repo_check: UpdCheck,
    repo_upd_tx: Sender<RepoCheckMsg>,
    repo_upd_rx: Receiver<RepoCheckMsg>,
    aur_check_gen: u64,
    aur_check: UpdCheck,
    aur_check_started: bool,
    aur_upd_tx: Sender<AurCheckMsg>,
    aur_upd_rx: Receiver<AurCheckMsg>,
    clean_gen: u64,
    /// Once the Cleanup tab has been opened, every operation rescans it.
    clean_opened: bool,
    clean_scanner: ScanFn,
    clean_tx: Sender<CleanScanMsg>,
    clean_rx: Receiver<CleanScanMsg>,
    clean_keep: Option<CleanKind>,
    /// Last maintainer seen for each foreign AUR package: the baseline.
    maintainers: Maintainers,
    maintainers_saver: MaintainersSaver,
    news_gen: u64,
    news_tx: Sender<NewsMsg>,
    news_rx: Receiver<NewsMsg>,
    /// Last command started with `U`: if it succeeds, the baseline is updated.
    pending_upgrade: bool,
    theme_saver: ThemeSaver,
    /// `COLORTERM` announces 24-bit color; it is read once, when the app is created.
    truecolor: bool,
    should_quit: bool,
}

impl App {
    pub fn new(repo: RepoDb, helper: Helper) -> Self {
        let (aur_tx, aur_rx) = mpsc::channel();
        let (index_tx, index_rx) = mpsc::channel();
        let (info_tx, info_rx) = mpsc::channel();
        let (pkgbuild_tx, pkgbuild_rx) = mpsc::channel();
        let (repo_upd_tx, repo_upd_rx) = mpsc::channel();
        let (aur_upd_tx, aur_upd_rx) = mpsc::channel();
        let (clean_tx, clean_rx) = mpsc::channel();
        let (news_tx, news_rx) = mpsc::channel();
        let mut state = State::new(helper.name());
        state.set_installed(repo.installed());
        let mut app = App {
            repo,
            helper,
            state,
            repo_results: Vec::new(),
            aur_pkgs: Vec::new(),
            aur_last_query: None,
            aur_tx,
            aur_rx,
            aur_generation: 0,
            aur_deadline: None,
            aur_index: None,
            index_tx,
            index_rx,
            aur_info: HashMap::new(),
            info_inflight: HashSet::new(),
            info_deadline: None,
            info_tx,
            info_rx,
            view_key: None,
            pkgbuild_target: None,
            pkgbuild_cache: HashMap::new(),
            pkgbuild_inflight: HashSet::new(),
            pkgbuild_deadline: None,
            pkgbuild_tx,
            pkgbuild_rx,
            repo_check_gen: 0,
            repo_checker: crate::backend::updates::checkupdates,
            repo_check: None,
            repo_upd_tx,
            repo_upd_rx,
            aur_check_gen: 0,
            aur_check: None,
            aur_check_started: false,
            aur_upd_tx,
            aur_upd_rx,
            clean_gen: 0,
            clean_opened: false,
            clean_scanner: crate::backend::clean::scan,
            clean_tx,
            clean_rx,
            clean_keep: None,
            maintainers: HashMap::new(),
            maintainers_saver: warnings::save_maintainers,
            news_gen: 0,
            news_tx,
            news_rx,
            pending_upgrade: false,
            theme_saver: save_theme,
            truecolor: truecolor_from_env(),
            should_quit: false,
        };
        app.refresh_details();
        app.rebuild_updates();
        app
    }

    pub fn start_background(&mut self) {
        let tx = self.index_tx.clone();
        std::thread::spawn(move || search::load_index(&tx));
        self.spawn_repo_check();
        self.maintainers = warnings::load_maintainers();
        self.spawn_news();
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// What the main loop does after an external command. A `U` that succeeded
    /// updates the maintainer baseline.
    pub fn after_operation(&mut self, ok: bool) {
        for marked in &mut self.state.marked {
            marked.clear();
        }
        let upgraded = std::mem::take(&mut self.pending_upgrade);
        let reload_result = self.repo.reload();
        self.refresh_db();
        if upgraded && ok {
            self.snapshot_after_upgrade();
        }
        if ok {
            self.set_status("✓ Operation completed", true);
        } else {
            self.set_status("✗ The operation failed", false);
        }
        if let Err(e) = reload_result {
            self.set_status(format!("Couldn't reload the database: {e:#}"), false);
        }
        self.check_updates();
        if self.clean_opened {
            self.spawn_clean_scan();
        }
    }

    pub fn tick(&mut self) {
        while let Ok((generation, res)) = self.aur_rx.try_recv() {
            if generation != self.aur_generation {
                continue;
            }
            match res {
                Ok(pkgs) => {
                    self.state.aur_state = AurState::Done {
                        count: pkgs.len(),
                        local: false,
                    };
                    self.aur_pkgs = pkgs;
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    self.aur_pkgs.clear();
                    self.state.aur_state = AurState::Error(msg.clone());
                    self.set_background_status(format!("AUR: {msg}"), Level::Error);
                }
            }
            self.rebuild_results();
            self.refresh_details();
        }
        while let Ok((name, res)) = self.info_rx.try_recv() {
            self.info_inflight.remove(&name);
            match res {
                Ok(pkgs) => self
                    .aur_info
                    .extend(pkgs.into_iter().map(|p| (p.name.clone(), p))),
                Err(e) => self.set_background_status(
                    format!("AUR: couldn't read the information for {name}: {e:#}"),
                    Level::Error,
                ),
            }
            self.refresh_details();
        }
        while let Ok((key, res)) = self.pkgbuild_rx.try_recv() {
            self.pkgbuild_inflight.remove(&key);
            let shown = matches!(&self.pkgbuild_target, Some(Ok(k)) if *k == key);
            self.pkgbuild_cache.insert(key, res);
            if shown {
                self.sync_pkgbuild_view();
            }
        }
        while let Ok(msg) = self.index_rx.try_recv() {
            match msg {
                IndexMsg::Loaded(index) => self.on_aur_index(index),
                IndexMsg::Error(e) => self.on_index_error(e),
            }
        }
        while let Ok((generation, res)) = self.repo_upd_rx.try_recv() {
            self.on_repo_updates(generation, res);
        }
        while let Ok((generation, res)) = self.aur_upd_rx.try_recv() {
            self.on_aur_updates(generation, res);
        }
        while let Ok((generation, result, last_upgrade)) = self.news_rx.try_recv() {
            self.on_news(generation, result, last_upgrade);
        }
        while let Ok((generation, tasks)) = self.clean_rx.try_recv() {
            self.on_clean_scan(generation, tasks);
        }
        if self.aur_deadline.is_some_and(|d| Instant::now() >= d) {
            self.search_aur_now();
        }
        if let Some((_, name)) = self.info_deadline.take_if(|(at, _)| Instant::now() >= *at) {
            self.spawn_info(name);
        }
        if let Some((_, key)) = self
            .pkgbuild_deadline
            .take_if(|(at, _)| Instant::now() >= *at)
        {
            self.spawn_pkgbuild(key);
        }
        if self.state.status.as_ref().is_some_and(|s| !s.is_recent()) {
            self.state.status = None;
        }
    }

    fn set_status(&mut self, text: impl Into<String>, ok: bool) {
        let level = if ok { Level::Ok } else { Level::Error };
        self.set_level(text, level);
    }

    fn set_warning(&mut self, text: impl Into<String>) {
        self.set_level(text, Level::Warn);
    }

    /// Message from a background thread: an Ok doesn't replace a warning or error still shown,
    /// so the startup warning isn't hidden by the index. User actions use `set_status`.
    fn set_background_status(&mut self, text: impl Into<String>, level: Level) {
        let protected = self
            .state
            .status
            .as_ref()
            .is_some_and(|s| s.level != Level::Ok && s.is_recent());
        if level == Level::Ok && protected {
            return;
        }
        self.set_level(text, level);
    }

    fn set_level(&mut self, text: impl Into<String>, level: Level) {
        self.state.status = Some(Status {
            text: text.into(),
            level,
            at: Instant::now(),
        });
    }
}

fn truecolor_from_env() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"))
}

fn save_theme(id: &str) -> anyhow::Result<()> {
    crate::config::Config {
        theme: id.to_string(),
    }
    .save()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use std::time::Duration;

    #[test]
    fn after_upgrade_that_works_reports_ok_and_clears_the_pending_flag() {
        let mut app = empty_app();
        app.aur_index = Some(AurIndex::from_pkgs(Vec::new()));
        app.pending_upgrade = true;
        app.after_operation(true);
        assert!(!app.pending_upgrade);
        assert_eq!(status_text(&app), "✓ Operation completed");
        assert_eq!(level(&app), Some(Level::Ok));
    }

    #[test]
    fn after_upgrade_that_fails_keeps_the_baseline_and_reports_error() {
        let mut app = empty_app();
        app.aur_index = Some(AurIndex::from_pkgs(Vec::new()));
        app.maintainers
            .insert("zzfacpm-f".to_string(), Some("old".to_string()));
        app.pending_upgrade = true;
        app.after_operation(false);
        assert!(!app.pending_upgrade);
        assert_eq!(status_text(&app), "✗ The operation failed");
        assert_eq!(level(&app), Some(Level::Error));
        assert_eq!(
            app.maintainers.get("zzfacpm-f"),
            Some(&Some("old".to_string())),
            "a failed U doesn't touch the baseline"
        );
    }

    #[test]
    fn after_operation_without_upgrade_reports_the_result_only() {
        let mut app = empty_app();
        app.aur_index = Some(AurIndex::from_pkgs(Vec::new()));
        app.maintainers
            .insert("zzfacpm-f".to_string(), Some("old".to_string()));
        app.after_operation(true);
        assert_eq!(status_text(&app), "✓ Operation completed");
        assert_eq!(level(&app), Some(Level::Ok));
        assert!(!app.pending_upgrade);
        assert_eq!(
            app.maintainers.get("zzfacpm-f"),
            Some(&Some("old".to_string())),
            "without U there is no new baseline"
        );
    }

    #[test]
    fn tick_clears_a_status_older_than_its_ttl() {
        let mut app = empty_app();
        app.set_status("reciente", true);
        app.state.status.as_mut().unwrap().at = Instant::now() - Duration::from_secs(3);
        app.tick();
        assert!(app.state.status.is_some(), "3 s is still recent");

        app.state.status.as_mut().unwrap().at = Instant::now() - Duration::from_secs(5);
        app.tick();
        assert!(app.state.status.is_none());
    }
}

//! Actualizaciones de repos y del AUR, el modal de `u`, noticias, advertencias y la línea base de
//! mantenedores.

use std::collections::{HashMap, HashSet};

use chrono::Utc;

use super::{App, Level, Modal, NewsState, UpdCheck, UpgradePlan};
use crate::backend::aur::AurPkg;
use crate::backend::helper::{Helper, command_line};
use crate::backend::news::{self, NewsItem};
use crate::backend::updates::{self, RawUpdate, Update};
use crate::backend::warnings::{self, Severity, Warning};
use crate::pkg::{Package, Source};

const AUR_INFO_BATCH: usize = 100;
const UPGRADE_LIST_MAX: usize = 10;
const NO_CHECKUPDATES: &str = "checkupdates no está instalado (pacman-contrib): se usa la base local, puede estar desactualizada";
/// Ventana de "noticia nueva" cuando no hay log de pacman: 30 días.
const NEWS_WINDOW: i64 = 30 * 24 * 60 * 60;

impl App {
    pub(super) fn request_upgrade(&mut self) {
        let rows = &self.state.updates;
        let repos = rows
            .iter()
            .filter(|p| matches!(p.source, Source::Repo(_)))
            .count();
        let aur = rows.iter().filter(|p| p.source == Source::Aur).count();
        let lines: Vec<String> = rows
            .iter()
            .take(UPGRADE_LIST_MAX)
            .map(|p| {
                format!(
                    "{} {} → {}",
                    p.name,
                    p.installed.as_deref().unwrap_or("?"),
                    p.version
                )
            })
            .collect();
        let more = rows.len() - lines.len();
        // La peor advertencia de cada paquete; la lista de warnings va ordenada.
        let mut flagged: Vec<(String, Severity, String)> = rows
            .iter()
            .filter_map(|row| {
                let first = self.state.warnings.get(&row.name)?.first()?;
                (first.severity != Severity::Info)
                    .then(|| (row.name.clone(), first.severity, first.label.clone()))
            })
            .collect();
        flagged.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let news_new = self.state.news.new_items();
        let plan = UpgradePlan {
            repos,
            aur,
            lines,
            more,
            command: command_line(&self.helper.upgrade_command()),
            pacman_warning: self.helper == Helper::Pacman && aur > 0,
            flagged,
            news_new: news_new.len(),
            news_intervention: news_new
                .iter()
                .any(|item| news::is_intervention(&item.title)),
        };
        self.state.modal = Some(Modal::Upgrade(plan));
    }

    pub(super) fn check_updates(&mut self) {
        self.spawn_repo_check();
        self.recompute_aur_updates();
    }

    pub(super) fn spawn_repo_check(&mut self) {
        self.repo_check_gen += 1;
        let generation = self.repo_check_gen;
        self.repo_check = None;
        let tx = self.repo_upd_tx.clone();
        let checker = self.repo_checker;
        std::thread::spawn(move || {
            let _ = tx.send((generation, checker()));
        });
        self.rebuild_updates();
    }

    /// La parte del AUR de las actualizaciones. Con índice es síncrona; sin índice, `aur::info`
    /// en un hilo.
    pub(super) fn recompute_aur_updates(&mut self) {
        let local = self.aur_index.as_ref().map(|index| {
            let foreign = foreign_packages(&self.state.installed);
            let aur: Vec<AurPkg> = foreign.iter().filter_map(|p| index.get(&p.name)).collect();
            updates::aur_updates(&foreign, &aur)
        });
        match local {
            Some(list) => {
                // Descarta una comprobación por `aur::info` que siguiera en vuelo.
                self.aur_check_gen += 1;
                self.aur_check = Some(Ok(list));
                self.rebuild_updates();
            }
            None => self.spawn_aur_info_check(),
        }
    }

    pub(super) fn spawn_aur_info_check(&mut self) {
        self.aur_check_gen += 1;
        self.aur_check_started = true;
        let generation = self.aur_check_gen;
        let foreign = foreign_packages(&self.state.installed);
        if foreign.is_empty() {
            self.aur_check = Some(Ok(Vec::new()));
            self.rebuild_updates();
            return;
        }
        self.aur_check = None;
        let tx = self.aur_upd_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((generation, aur_info_updates(&foreign)));
        });
        self.rebuild_updates();
    }

    /// Resultado de `checkupdates`. Los `Update` de repos se arman aquí porque necesitan la base.
    /// Ok(None): sin `checkupdates`, se usa la base local.
    pub(super) fn on_repo_updates(
        &mut self,
        generation: u64,
        result: anyhow::Result<Option<Vec<RawUpdate>>>,
    ) {
        if generation != self.repo_check_gen {
            return;
        }
        let check: Result<Vec<Update>, String> = match result {
            Ok(Some(raw)) => Ok(raw
                .into_iter()
                .map(|(name, current, new)| {
                    let repo = self.repo.repo_of(&name).unwrap_or_else(|| "?".to_string());
                    Update {
                        name,
                        current,
                        new,
                        source: Source::Repo(repo),
                    }
                })
                .collect()),
            Ok(None) => {
                self.set_background_status(NO_CHECKUPDATES, Level::Warn);
                Ok(self.repo.local_updates())
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.set_background_status(format!("checkupdates: {msg}"), Level::Error);
                Err(msg)
            }
        };
        self.repo_check = Some(check);
        self.rebuild_updates();
        self.refresh_details();
    }

    pub(super) fn on_aur_updates(&mut self, generation: u64, result: anyhow::Result<Vec<Update>>) {
        if generation != self.aur_check_gen {
            return;
        }
        self.aur_check = Some(match result {
            Ok(list) => Ok(list),
            Err(e) => {
                let msg = format!("{e:#}");
                self.set_background_status(
                    format!("AUR: no se pudieron comprobar las actualizaciones: {msg}"),
                    Level::Error,
                );
                Err(msg)
            }
        });
        self.rebuild_updates();
        self.refresh_details();
    }

    pub(super) fn rebuild_updates(&mut self) {
        let pending = self.repo_check.is_none() || self.aur_check.is_none();
        let error =
            matches!(self.repo_check, Some(Err(_))) || matches!(self.aur_check, Some(Err(_)));
        let mut repo = ok_updates(&self.repo_check);
        let mut aur = ok_updates(&self.aur_check);
        repo.sort_by(|a, b| a.name.cmp(&b.name));
        aur.sort_by(|a, b| a.name.cmp(&b.name));
        self.state.upgradable = repo.iter().chain(&aur).map(|u| u.name.clone()).collect();
        self.state.updates = repo.into_iter().chain(aur).map(update_row).collect();
        self.state.updates_pending = pending;
        self.state.updates_error = error;
        self.state.clamp_lists();
        self.recompute_warnings();
    }

    pub(super) fn spawn_news(&mut self) {
        self.news_gen += 1;
        let generation = self.news_gen;
        self.state.news = NewsState::Loading;
        let tx = self.news_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((generation, news::fetch(), news::last_upgrade()));
        });
    }

    /// Resultado de las noticias. `since` es la última actualización del log, o hace 30 días si
    /// no hay log. Un error no pone status: el encabezado ya dice que no se pudieron descargar.
    pub(super) fn on_news(
        &mut self,
        generation: u64,
        result: anyhow::Result<Vec<NewsItem>>,
        last_upgrade: Option<i64>,
    ) {
        if generation != self.news_gen {
            return;
        }
        self.state.news = match result {
            Ok(mut items) => {
                items.sort_by_key(|item| std::cmp::Reverse(item.published));
                let since = last_upgrade.unwrap_or_else(|| Utc::now().timestamp() - NEWS_WINDOW);
                NewsState::Loaded { items, since }
            }
            Err(e) => NewsState::Error(format!("{e:#}")),
        };
        self.recompute_warnings();
    }

    /// Recalcula las advertencias de cada paquete: señales de la actualización, del AUR (con el
    /// índice), noticias nuevas que lo mencionan o que son avisos del AUR, y los foráneos
    /// instalados sin actualización.
    pub(super) fn recompute_warnings(&mut self) {
        let now = Utc::now().timestamp();
        let new_news: Vec<NewsItem> = self.state.news.new_items().into_iter().cloned().collect();
        let mut map: HashMap<String, Vec<Warning>> = HashMap::new();
        for row in &self.state.updates {
            let update = Update {
                name: row.name.clone(),
                current: row.installed.clone().unwrap_or_default(),
                new: row.version.clone(),
                source: row.source.clone(),
            };
            let mut ws = warnings::update_signals(&update);
            if row.source == Source::Aur {
                let aur = self
                    .aur_index
                    .as_ref()
                    .and_then(|index| index.get(&row.name));
                if let Some(aur) = aur {
                    ws.extend(warnings::aur_signals(
                        &aur,
                        self.maintainers.get(&row.name),
                        now,
                    ));
                }
            }
            for item in &new_news {
                if news::mentions_package(&item.title, &row.name) {
                    let intervention = news::is_intervention(&item.title);
                    ws.push(warnings::news(&item.title, &item.date, intervention));
                }
                if row.source == Source::Aur && news::is_aur_notice(&item.title) {
                    ws.push(warnings::aur_notice(&item.title, &item.date));
                }
            }
            store_warnings(&mut map, &row.name, ws);
        }
        if let Some(index) = &self.aur_index {
            let upgrading: HashSet<&str> =
                self.state.updates.iter().map(|p| p.name.as_str()).collect();
            for pkg in self
                .state
                .installed
                .iter()
                .filter(|p| p.source == Source::Foreign)
            {
                if upgrading.contains(pkg.name.as_str()) {
                    continue;
                }
                let mut ws = Vec::new();
                match index.get(&pkg.name) {
                    Some(aur) => ws.extend(warnings::aur_signals(
                        &aur,
                        self.maintainers.get(&pkg.name),
                        now,
                    )),
                    None => {
                        let pkgbase = self.repo.pkgbase(&pkg.name);
                        let missing = warnings::is_missing(
                            &pkg.name,
                            pkgbase.as_deref(),
                            |name| index.get(name).is_some(),
                            |base| index.has_base(base),
                        );
                        if missing {
                            ws.push(warnings::missing());
                        }
                    }
                }
                store_warnings(&mut map, &pkg.name, ws);
            }
        }
        self.state.warnings = map;
    }

    /// Línea base de la primera carga del índice: cada foráneo del índice que no tiene entrada
    /// se guarda con su mantenedor actual. Los que ya estaban no se tocan: ahí está la alarma.
    pub(super) fn seed_maintainers(&mut self) {
        let Some(index) = &self.aur_index else {
            return;
        };
        let mut added = false;
        for pkg in self
            .state
            .installed
            .iter()
            .filter(|p| p.source == Source::Foreign)
        {
            if self.maintainers.contains_key(&pkg.name) {
                continue;
            }
            if let Some(aur) = index.get(&pkg.name) {
                self.maintainers.insert(pkg.name.clone(), aur.maintainer);
                added = true;
            }
        }
        if added {
            self.save_maintainers_snapshot();
        }
    }

    /// Tras un `u` que terminó bien: los foráneos del índice pasan a tener su mantenedor actual,
    /// así que la alarma de cambio de mantenedor desaparece.
    pub(super) fn snapshot_after_upgrade(&mut self) {
        let Some(index) = &self.aur_index else {
            return;
        };
        for pkg in self
            .state
            .installed
            .iter()
            .filter(|p| p.source == Source::Foreign)
        {
            if let Some(aur) = index.get(&pkg.name) {
                self.maintainers.insert(pkg.name.clone(), aur.maintainer);
            }
        }
        self.save_maintainers_snapshot();
        self.recompute_warnings();
    }

    fn save_maintainers_snapshot(&mut self) {
        if let Err(e) = (self.maintainers_saver)(&self.maintainers) {
            self.set_background_status(
                format!("No se pudo guardar el registro de mantenedores: {e:#}"),
                Level::Warn,
            );
        }
    }
}

fn aur_info_updates(foreign: &[Package]) -> anyhow::Result<Vec<Update>> {
    let names: Vec<String> = foreign.iter().map(|p| p.name.clone()).collect();
    let mut pkgs = Vec::new();
    for batch in names.chunks(AUR_INFO_BATCH) {
        pkgs.extend(crate::backend::aur::info(batch)?);
    }
    Ok(updates::aur_updates(foreign, &pkgs))
}

fn foreign_packages(installed: &[Package]) -> Vec<Package> {
    installed
        .iter()
        .filter(|p| p.source == Source::Foreign)
        .cloned()
        .collect()
}

fn ok_updates(check: &UpdCheck) -> Vec<Update> {
    match check {
        Some(Ok(list)) => list.clone(),
        _ => Vec::new(),
    }
}

fn update_row(u: Update) -> Package {
    Package {
        name: u.name,
        version: u.new,
        description: String::new(),
        source: u.source,
        installed: Some(u.current),
    }
}

fn store_warnings(map: &mut HashMap<String, Vec<Warning>>, name: &str, mut ws: Vec<Warning>) {
    if !ws.is_empty() {
        warnings::sort(&mut ws);
        map.insert(name.to_string(), ws);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use crate::app::{Action, Level};
    use crate::backend::aur_index::AurIndex;
    use crate::backend::warnings::{Kind, Maintainers};
    use ratatui::crossterm::event::KeyCode;

    fn only_mantenedor(m: &Maintainers) -> anyhow::Result<()> {
        anyhow::ensure!(
            m.get("zzfacpm-g") == Some(&Some("mantenedor".to_string())),
            "línea base incompleta"
        );
        Ok(())
    }

    fn only_nuevo(m: &Maintainers) -> anyhow::Result<()> {
        anyhow::ensure!(
            m.get("zzfacpm-f") == Some(&Some("nuevo".to_string())),
            "línea base incompleta"
        );
        Ok(())
    }

    fn with_updates(app: &mut App, repo: Vec<RawUpdate>, aur: Vec<Update>) {
        let repo_gen = app.repo_check_gen;
        app.on_repo_updates(repo_gen, Ok(Some(repo)));
        let aur_gen = app.aur_check_gen;
        app.on_aur_updates(aur_gen, Ok(aur));
    }

    fn raw(name: &str, current: &str, new: &str) -> RawUpdate {
        (name.into(), current.into(), new.into())
    }

    #[test]
    fn aur_updates_come_from_the_index_for_foreign_packages() {
        let mut app = empty_app();
        app.state
            .set_installed(vec![foreign_pkg("zzfacpm-f", "1.0-1")]);
        app.refresh_details();
        app.on_aur_index(AurIndex::from_pkgs(vec![aur_pkg_version(
            "zzfacpm-f",
            "zzfacpm-f",
            "1.1-1",
            &[],
        )]));
        let list = match &app.aur_check {
            Some(Ok(list)) => list.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].new, "1.1-1");
        assert!(app.state.upgradable.contains("zzfacpm-f"));
        assert_eq!(app.state.updates.len(), 1);
    }

    #[test]
    fn repo_updates_are_sorted_and_marked() {
        let mut app = empty_app();
        with_updates(
            &mut app,
            vec![
                raw("zzfacpm-b", "1.0-1", "1.1-1"),
                raw("zzfacpm-a", "2.0-1", "2.0-2"),
            ],
            Vec::new(),
        );
        let names: Vec<&str> = app.state.updates.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["zzfacpm-a", "zzfacpm-b"]);
        assert!(app.state.upgradable.contains("zzfacpm-a"));
        assert!(!app.state.updates_pending && !app.state.updates_error);
        assert_eq!(app.state.updates[0].installed.as_deref(), Some("2.0-1"));
        assert_eq!(app.state.updates[0].version, "2.0-2");
    }

    #[test]
    fn u_opens_the_upgrade_modal_and_y_runs_the_helper_command() {
        let mut app = app_with(Helper::Yay);
        app.state.set_installed(Vec::new());
        app.refresh_details();
        with_updates(
            &mut app,
            vec![raw("zzfacpm-a", "1.0-1", "1.1-1")],
            vec![aur_update("zzfacpm-b", "1.0-1", "1.0-2")],
        );
        press(&mut app, KeyCode::Char('u'));
        match &app.state.modal {
            Some(Modal::Upgrade(plan)) => {
                assert_eq!((plan.repos, plan.aur), (1, 1));
                assert_eq!(plan.command, "yay -Syu");
                assert!(!plan.pacman_warning);
            }
            other => panic!("{other:?}"),
        }
        match press(&mut app, KeyCode::Char('y')) {
            Some(Action::Run(cmd)) => assert_eq!(command_line(&cmd), "yay -Syu"),
            None => panic!("confirmar debe devolver la acción"),
        }
        assert!(app.state.modal.is_none());
        assert!(app.pending_upgrade);
    }

    #[test]
    fn upgrade_modal_warns_when_pacman_would_skip_aur_packages() {
        let mut app = app_with(Helper::Pacman);
        let agen = app.aur_check_gen;
        app.on_aur_updates(agen, Ok(vec![aur_update("zzfacpm-b", "1.0-1", "1.0-2")]));
        press(&mut app, KeyCode::Char('u'));
        match &app.state.modal {
            Some(Modal::Upgrade(plan)) => {
                assert!(plan.pacman_warning);
                assert_eq!(plan.command, "sudo pacman -Syu");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn upgrade_modal_counts_new_news_in_singular_and_plural() {
        let mut app = empty_app();
        with_updates(
            &mut app,
            vec![raw("zzfacpm-a", "1.0-1", "1.0-2")],
            Vec::new(),
        );
        let news_gen = app.news_gen;
        app.on_news(
            news_gen,
            Ok(vec![news_item(
                "zzfacpm-a needs manual intervention",
                AFTER,
            )]),
            Some(SINCE),
        );
        press(&mut app, KeyCode::Char('u'));
        match &app.state.modal {
            Some(Modal::Upgrade(plan)) => {
                assert_eq!(plan.news_new, 1);
                assert!(plan.news_intervention);
                assert_eq!(plan.flagged.len(), 1);
                assert_eq!(plan.flagged[0].0, "zzfacpm-a");
                assert_eq!(plan.flagged[0].1, Severity::Alarm);
            }
            other => panic!("{other:?}"),
        }
        press(&mut app, KeyCode::Esc);

        let news_gen = app.news_gen;
        app.on_news(
            news_gen,
            Ok(vec![
                news_item("zzfacpm-b released", AFTER),
                news_item("zzfacpm-c released", AFTER),
            ]),
            Some(SINCE),
        );
        press(&mut app, KeyCode::Char('u'));
        match &app.state.modal {
            Some(Modal::Upgrade(plan)) => {
                assert_eq!(plan.news_new, 2);
                assert!(!plan.news_intervention);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn on_news_keeps_the_newest_first_and_the_log_date() {
        let mut app = empty_app();
        let generation = app.news_gen;
        let items = vec![news_item("Vieja", BEFORE), news_item("Nueva", AFTER)];
        app.on_news(generation, Ok(items), Some(SINCE));
        match &app.state.news {
            NewsState::Loaded { items, since } => {
                assert_eq!(*since, SINCE);
                assert_eq!(items[0].title, "Nueva");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn on_news_without_log_uses_the_30_day_window() {
        let mut app = empty_app();
        let before = Utc::now().timestamp();
        let generation = app.news_gen;
        app.on_news(generation, Ok(vec![news_item("Vieja", BEFORE)]), None);
        match &app.state.news {
            NewsState::Loaded { since, .. } => {
                assert!(*since >= before - NEWS_WINDOW);
                assert!(*since <= Utc::now().timestamp() - NEWS_WINDOW);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn news_warnings_reach_only_the_packages_they_name() {
        let mut app = empty_app();
        with_updates(
            &mut app,
            vec![raw("zzfacpm-r", "1.0-1", "1.0-2")],
            vec![aur_update("zzfacpm-b", "1.0-1", "1.0-2")],
        );
        let news_gen = app.news_gen;
        app.on_news(
            news_gen,
            Ok(vec![
                news_item("Active AUR malicious packages incident", AFTER),
                news_item("zzfacpm-b released", AFTER),
            ]),
            Some(SINCE),
        );
        let b = kinds_of(&app, "zzfacpm-b");
        assert!(b.contains(&Kind::AurNotice));
        assert!(b.contains(&Kind::News));
        assert!(!kinds_of(&app, "zzfacpm-r").contains(&Kind::AurNotice));
    }

    #[test]
    fn maintainer_change_is_an_alarm_and_the_baseline_is_kept() {
        let mut app = empty_app();
        app.state
            .set_installed(vec![foreign_pkg("zzfacpm-f", "1.0-1")]);
        app.refresh_details();
        app.maintainers
            .insert("zzfacpm-f".to_string(), Some("viejo".to_string()));
        app.on_aur_index(AurIndex::from_pkgs(vec![aur_pkg_maintained(
            "zzfacpm-f",
            "1.0-1",
            Some("nuevo"),
        )]));
        let ws = app.state.warnings.get("zzfacpm-f").expect("advertencias");
        assert!(
            ws.iter()
                .any(|w| w.kind == Kind::MaintainerChanged && w.severity == Severity::Alarm)
        );
        assert_eq!(
            app.maintainers.get("zzfacpm-f"),
            Some(&Some("viejo".to_string())),
            "el cambio no se pisa"
        );
    }

    #[test]
    fn index_seeds_unknown_foreign_packages_and_saves_the_baseline() {
        let mut app = empty_app();
        app.maintainers_saver = only_mantenedor;
        app.state
            .set_installed(vec![foreign_pkg("zzfacpm-g", "1.0-1")]);
        app.refresh_details();
        app.on_aur_index(AurIndex::from_pkgs(vec![aur_pkg_maintained(
            "zzfacpm-g",
            "1.0-1",
            Some("mantenedor"),
        )]));
        assert_eq!(
            app.maintainers.get("zzfacpm-g"),
            Some(&Some("mantenedor".to_string()))
        );
        assert_eq!(
            level(&app),
            Some(Level::Ok),
            "el guardado recibió la línea base completa"
        );
        assert!(!kinds_of(&app, "zzfacpm-g").contains(&Kind::MaintainerChanged));
    }

    #[test]
    fn failing_baseline_save_is_a_warning() {
        let mut app = empty_app();
        app.maintainers_saver = failing_maintainers_save;
        app.state
            .set_installed(vec![foreign_pkg("zzfacpm-g", "1.0-1")]);
        app.refresh_details();
        app.on_aur_index(AurIndex::from_pkgs(vec![aur_pkg_maintained(
            "zzfacpm-g",
            "1.0-1",
            Some("mantenedor"),
        )]));
        assert!(status_text(&app).contains("No se pudo guardar el registro de mantenedores"));
        assert_eq!(level(&app), Some(Level::Warn));
    }

    #[test]
    fn snapshot_after_upgrade_clears_the_maintainer_alarm() {
        let mut app = empty_app();
        app.state
            .set_installed(vec![foreign_pkg("zzfacpm-f", "1.0-1")]);
        app.refresh_details();
        app.maintainers
            .insert("zzfacpm-f".to_string(), Some("viejo".to_string()));
        app.on_aur_index(AurIndex::from_pkgs(vec![aur_pkg_maintained(
            "zzfacpm-f",
            "1.0-1",
            Some("nuevo"),
        )]));
        assert!(kinds_of(&app, "zzfacpm-f").contains(&Kind::MaintainerChanged));

        app.maintainers_saver = only_nuevo;
        app.snapshot_after_upgrade();
        assert!(!kinds_of(&app, "zzfacpm-f").contains(&Kind::MaintainerChanged));
        assert_eq!(
            app.maintainers.get("zzfacpm-f"),
            Some(&Some("nuevo".to_string()))
        );
        assert_eq!(
            level(&app),
            Some(Level::Ok),
            "el guardado recibió la línea base nueva"
        );
    }

    #[test]
    fn missing_foreign_package_is_warned_but_its_debug_twin_is_not() {
        let mut app = empty_app();
        app.state.set_installed(vec![
            foreign_pkg("zzfacpm-m", "1.0-1"),
            foreign_pkg("zzfacpm-n-debug", "1.0-1"),
        ]);
        app.refresh_details();
        app.on_aur_index(AurIndex::from_pkgs(vec![
            aur_pkg("zzfacpm-otro", &[]),
            aur_pkg("zzfacpm-n", &[]),
        ]));
        assert!(kinds_of(&app, "zzfacpm-m").contains(&Kind::Missing));
        let ws = app.state.warnings.get("zzfacpm-m").expect("advertencias");
        assert_eq!(ws[0].severity, Severity::Warn);
        assert!(!kinds_of(&app, "zzfacpm-n-debug").contains(&Kind::Missing));
    }
}

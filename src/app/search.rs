//! Búsqueda en repos y en el AUR, índice local, `aur::info`, PKGBUILD y el contenido del panel
//! derecho.

use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use super::{App, AurState, IndexMsg, Level, PkgKey, PkgbuildView, RightTab, Tab, ViewKey};
use crate::backend::aur;
use crate::backend::aur_index::AurIndex;
use crate::backend::pkgbuild::{self, PkgbuildSource};
use crate::pkg::{Details, Package, Source};

/// Espera tras la última tecla antes de consultar el AUR por RPC.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Espera tras el último cambio de selección antes de pedir `aur::info` de un paquete del AUR.
const INFO_DEBOUNCE: Duration = Duration::from_millis(150);
/// Espera tras el último cambio de selección antes de descargar un PKGBUILD.
const PKGBUILD_DEBOUNCE: Duration = Duration::from_millis(300);
const INDEX_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// Filas de los detalles que el RPC `search` no trae (solo `info` las tiene).
const AUR_INFO_ROWS: [&str; 5] = [
    "Licencias",
    "Palabras clave",
    "Depende de",
    "Dependencias de compilación",
    "Dependencias opcionales",
];
const LOADING_TEXT: &str = "cargando…";

impl App {
    pub(super) fn on_query_changed(&mut self) {
        self.repo_results = self.repo.search(&self.state.query);
        self.aur_pkgs.clear();
        // Invalida cualquier respuesta del AUR que aún esté en vuelo.
        self.aur_last_query = None;
        self.aur_generation += 1;
        self.aur_deadline = None;
        if self.state.query.trim().is_empty() {
            self.state.aur_state = AurState::Idle;
        } else if self.aur_index.is_some() {
            self.search_local();
        } else {
            self.state.aur_state = AurState::Loading;
            self.aur_deadline = Some(Instant::now() + DEBOUNCE);
        }
        self.rebuild_results();
        self.state.select_first_row();
    }

    fn search_local(&mut self) {
        let Some(index) = &self.aur_index else {
            return;
        };
        self.aur_pkgs = index.search(&self.state.query);
        self.aur_last_query = Some(self.state.query.clone());
        self.aur_deadline = None;
        self.state.aur_state = AurState::Done {
            count: self.aur_pkgs.len(),
            local: true,
        };
    }

    /// Consulta el AUR sin esperar el debounce. Con índice, busca en él. Si no, solo pide a la
    /// red si la consulta cambió desde la última o si la anterior falló.
    pub(super) fn search_aur_now(&mut self) {
        self.aur_deadline = None;
        if self.state.query.trim().is_empty() {
            self.state.aur_state = AurState::Idle;
            return;
        }
        if self.aur_index.is_some() {
            self.search_local();
            self.rebuild_results();
            return;
        }
        let same_query = self.aur_last_query.as_deref() == Some(self.state.query.as_str());
        let failed = matches!(self.state.aur_state, AurState::Error(_));
        if same_query && !failed {
            return;
        }
        self.spawn_aur();
    }

    fn spawn_aur(&mut self) {
        self.aur_generation += 1;
        let generation = self.aur_generation;
        let query = self.state.query.clone();
        let tx = self.aur_tx.clone();
        self.aur_last_query = Some(query.clone());
        self.state.aur_state = AurState::Loading;
        std::thread::spawn(move || {
            let res = aur::search(&query);
            let _ = tx.send((generation, res));
        });
    }

    pub(super) fn on_aur_index(&mut self, index: AurIndex) {
        if self.aur_index.is_none() {
            self.set_background_status(
                format!("Índice del AUR: {} paquetes", index.len()),
                Level::Ok,
            );
        }
        self.aur_index = Some(index);
        self.seed_maintainers();
        self.recompute_aur_updates();
        self.recompute_warnings();
        if !self.state.query.trim().is_empty() {
            // Una búsqueda RPC que siga en vuelo ya no vale: su respuesta se descarta.
            self.aur_generation += 1;
            self.search_local();
            self.rebuild_results();
        }
        self.refresh_details();
    }

    pub(super) fn on_index_error(&mut self, msg: String) {
        self.set_background_status(format!("Índice del AUR: {msg}"), Level::Error);
        if self.aur_index.is_none() && !self.aur_check_started {
            self.spawn_aur_info_check();
        }
    }

    pub(super) fn spawn_info(&mut self, name: String) {
        if self.aur_info.contains_key(&name) || self.info_inflight.contains(&name) {
            return;
        }
        self.info_inflight.insert(name.clone());
        let tx = self.info_tx.clone();
        std::thread::spawn(move || {
            let res = aur::info(std::slice::from_ref(&name));
            let _ = tx.send((name, res));
        });
    }

    pub(super) fn spawn_pkgbuild(&mut self, key: PkgKey) {
        if self.pkgbuild_cache.contains_key(&key) || self.pkgbuild_inflight.contains(&key) {
            return;
        }
        self.pkgbuild_inflight.insert(key.clone());
        let tx = self.pkgbuild_tx.clone();
        std::thread::spawn(move || {
            let res = pkgbuild::fetch(key.0, &key.1).map_err(|e| format!("{e:#}"));
            let _ = tx.send((key, res));
        });
    }

    fn schedule_info(&mut self, pkg: Option<&Package>) {
        self.info_deadline = match pkg {
            Some(p)
                if self.state.right_tab == RightTab::Details
                    && p.source == Source::Aur
                    && !self.aur_info.contains_key(&p.name) =>
            {
                Some((Instant::now() + INFO_DEBOUNCE, p.name.clone()))
            }
            _ => None,
        };
    }

    fn select_pkgbuild(&mut self, pkg: Option<&Package>) {
        self.pkgbuild_deadline = None;
        self.pkgbuild_target = match (self.state.right_tab, pkg) {
            (RightTab::Pkgbuild, Some(p)) => Some(self.pkgbuild_key_of(p)),
            _ => None,
        };
        if let Some(Ok(key)) = &self.pkgbuild_target
            && !self.pkgbuild_cache.contains_key(key)
            && !self.pkgbuild_inflight.contains(key)
        {
            self.pkgbuild_deadline = Some((Instant::now() + PKGBUILD_DEBOUNCE, key.clone()));
        }
        self.sync_pkgbuild_view();
    }

    fn pkgbuild_key_of(&self, p: &Package) -> Result<PkgKey, String> {
        let (source, base) = match &p.source {
            Source::Aur => {
                let base = self
                    .aur_info
                    .get(&p.name)
                    .or_else(|| self.aur_pkgs.iter().find(|a| a.name == p.name))
                    .map(|a| a.package_base.clone())
                    .or_else(|| self.repo.pkgbase(&p.name));
                (PkgbuildSource::Aur, base)
            }
            Source::Foreign => (PkgbuildSource::Aur, self.repo.pkgbase(&p.name)),
            Source::Repo(_) => (PkgbuildSource::Repo, self.repo.pkgbase(&p.name)),
        };
        base.map(|b| (source, b))
            .ok_or_else(|| format!("no se encontró el paquete base de {}", p.name))
    }

    pub(super) fn sync_pkgbuild_view(&mut self) {
        self.state.pkgbuild = match &self.pkgbuild_target {
            None => PkgbuildView::Empty,
            Some(Err(motivo)) => PkgbuildView::Error(motivo.clone()),
            Some(Ok(key)) => match self.pkgbuild_cache.get(key) {
                Some(Ok(text)) => PkgbuildView::Ready(text.clone()),
                Some(Err(e)) => PkgbuildView::Error(e.clone()),
                None => PkgbuildView::Loading,
            },
        };
    }

    pub(super) fn rebuild_results(&mut self) {
        let repo_names: HashSet<&str> = self.repo_results.iter().map(|p| p.name.as_str()).collect();
        let mut results = self.repo_results.clone();
        results.extend(
            self.aur_pkgs
                .iter()
                .filter(|a| !repo_names.contains(a.name.as_str()))
                .map(|a| a.to_package(self.repo.installed_version(&a.name))),
        );
        self.state.results = results;
        self.state.clamp_lists();
    }

    pub(super) fn refresh_db(&mut self) {
        self.state.set_installed(self.repo.installed());
        self.repo_results = self.repo.search(&self.state.query);
        self.rebuild_results();
        self.refresh_details();
        self.recompute_warnings();
    }

    /// Recalcula los detalles del paquete seleccionado. El scroll vuelve a 0 si cambió.
    pub(super) fn refresh_details(&mut self) {
        let pkg = self.state.selected().cloned();
        let key: ViewKey = (
            self.state.tab,
            self.state.right_tab,
            match self.state.tab {
                Tab::Clean => self.state.selected_clean().map(|r| r.task.title.clone()),
                _ => pkg.as_ref().map(|p| p.name.clone()),
            },
        );
        if self.view_key.as_ref() != Some(&key) {
            self.state.right_scroll = 0;
            self.schedule_info(pkg.as_ref());
            self.select_pkgbuild(pkg.as_ref());
            self.view_key = Some(key);
        }
        if self.state.right_tab == RightTab::Details {
            self.state.details = match &pkg {
                Some(p) => self.details_of(p),
                None => Vec::new(),
            };
        }
    }

    fn details_of(&self, p: &Package) -> Details {
        if p.source == Source::Aur {
            if let Some(info) = self.aur_info.get(&p.name) {
                return info.details();
            }
            if let Some(a) = self.aur_pkgs.iter().find(|a| a.name == p.name) {
                let mut details = a.details();
                if self.info_pending(&p.name) {
                    mark_loading(&mut details);
                }
                return details;
            }
            // Un paquete del AUR fuera de los resultados (una actualización): filas básicas y,
            // mientras llega `info`, las de dependencias en "cargando…".
            let mut details = basic_details(p);
            if self.info_pending(&p.name) {
                details.extend(
                    AUR_INFO_ROWS
                        .iter()
                        .map(|key| (key.to_string(), LOADING_TEXT.to_string())),
                );
            }
            return details;
        }
        self.repo
            .details(&p.name)
            .unwrap_or_else(|| basic_details(p))
    }

    fn info_pending(&self, name: &str) -> bool {
        self.info_inflight.contains(name)
            || self.info_deadline.as_ref().is_some_and(|(_, n)| n == name)
    }
}

/// Hilo del índice: carga la caché; si tiene más de un día, la refresca y vuelve a cargarla.
/// Los errores llegan como mensaje, no son fatales.
pub(super) fn load_index(tx: &Sender<IndexMsg>) {
    let send_error = |e: anyhow::Error| {
        let _ = tx.send(IndexMsg::Error(format!("{e:#}")));
    };
    match AurIndex::load_cached() {
        Ok(Some(index)) => {
            let _ = tx.send(IndexMsg::Loaded(index));
        }
        Ok(None) => {}
        Err(e) => send_error(e),
    }
    match AurIndex::refresh_if_stale(INDEX_MAX_AGE) {
        Ok(true) => match AurIndex::load_cached() {
            Ok(Some(index)) => {
                let _ = tx.send(IndexMsg::Loaded(index));
            }
            Ok(None) => {}
            Err(e) => send_error(e),
        },
        Ok(false) => {}
        Err(e) => send_error(e),
    }
}

fn mark_loading(details: &mut Details) {
    for (key, value) in details.iter_mut() {
        if AUR_INFO_ROWS.contains(&key.as_str()) {
            *value = LOADING_TEXT.to_string();
        }
    }
}

fn basic_details(p: &Package) -> Details {
    vec![
        ("Nombre".into(), p.name.clone()),
        ("Versión".into(), p.version.clone()),
        ("Descripción".into(), p.description.clone()),
        ("Repositorio".into(), p.source.label().to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn enter_with_the_same_query_keeps_the_finished_aur_search() {
        let mut app = empty_app();
        app.state.tab = Tab::Search;
        app.state.query = "git".to_string();
        app.state.input_mode = true;
        app.aur_last_query = Some("git".to_string());
        let done = AurState::Done {
            count: 3,
            local: false,
        };
        app.state.aur_state = done.clone();
        let generation = app.aur_generation;

        press(&mut app, KeyCode::Enter);

        assert!(!app.state.input_mode);
        assert_eq!(app.state.aur_state, done);
        assert_eq!(app.aur_generation, generation, "no debe relanzar el AUR");
    }

    #[test]
    fn debounces_are_300_150_and_300_ms() {
        let mut app = empty_app();
        app.state.tab = Tab::Search;
        app.state.input_mode = true;
        let before = Instant::now();
        press(&mut app, KeyCode::Char('g'));
        let rpc = app.aur_deadline.expect("búsqueda por RPC con debounce");
        assert!(rpc >= before + DEBOUNCE && rpc <= Instant::now() + DEBOUNCE);

        let mut app = aur_selected_app("zzfacpm-base");
        app.info_deadline = None;
        app.view_key = None;
        let before = Instant::now();
        app.refresh_details();
        let (info, name) = app
            .info_deadline
            .clone()
            .expect("pedido de info con debounce");
        assert_eq!(name, FAKE);
        assert!(info >= before + INFO_DEBOUNCE && info <= Instant::now() + INFO_DEBOUNCE);

        app.state.right_tab = RightTab::Pkgbuild;
        app.view_key = None;
        let before = Instant::now();
        app.refresh_details();
        let (at, key) = app
            .pkgbuild_deadline
            .clone()
            .expect("descarga con debounce");
        assert_eq!(key, (PkgbuildSource::Aur, "zzfacpm-base".to_string()));
        assert!(at >= before + PKGBUILD_DEBOUNCE && at <= Instant::now() + PKGBUILD_DEBOUNCE);
    }

    #[test]
    fn pkgbuild_is_loading_until_the_download_arrives() {
        let mut app = aur_selected_app("zzfacpm-base");
        press(&mut app, KeyCode::Char('p'));
        let key = (PkgbuildSource::Aur, "zzfacpm-base".to_string());
        assert_eq!(app.pkgbuild_target, Some(Ok(key.clone())));
        assert_eq!(app.state.pkgbuild, PkgbuildView::Loading);
        assert!(
            app.pkgbuild_deadline.is_some(),
            "la descarga debe programarse"
        );

        app.pkgbuild_deadline = None;
        app.info_deadline = None;
        app.pkgbuild_inflight.insert(key.clone());
        app.pkgbuild_tx
            .send((key.clone(), Ok("pkgname=zzfacpm-prueba\n".to_string())))
            .unwrap();
        app.tick();
        assert_eq!(
            app.state.pkgbuild,
            PkgbuildView::Ready("pkgname=zzfacpm-prueba\n".to_string())
        );
        assert!(app.pkgbuild_inflight.is_empty());
    }

    #[test]
    fn pkgbuild_from_cache_is_ready_at_once() {
        let mut app = aur_selected_app("zzfacpm-base");
        let key = (PkgbuildSource::Aur, "zzfacpm-base".to_string());
        app.pkgbuild_cache.insert(key, Ok("pkgver=1\n".to_string()));
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(
            app.state.pkgbuild,
            PkgbuildView::Ready("pkgver=1\n".to_string())
        );
        assert!(
            app.pkgbuild_deadline.is_none(),
            "con caché no hace falta descargar"
        );
    }

    #[test]
    fn pkgbuild_is_not_fetched_twice() {
        let mut app = empty_app();
        let key = (PkgbuildSource::Aur, "zzfacpm-base".to_string());
        app.pkgbuild_inflight.insert(key.clone());
        app.spawn_pkgbuild(key.clone());
        assert_eq!(
            app.pkgbuild_inflight.len(),
            1,
            "no debe haber un segundo pedido"
        );

        app.pkgbuild_inflight.clear();
        app.pkgbuild_cache.insert(key.clone(), Ok(String::new()));
        app.spawn_pkgbuild(key);
        assert!(app.pkgbuild_inflight.is_empty(), "con caché no se descarga");
    }

    #[test]
    fn pkgbuild_reports_a_package_without_base() {
        let mut app = empty_app();
        app.state.tab = Tab::Search;
        app.state.results = vec![Package {
            name: FAKE.to_string(),
            version: "1.0-1".to_string(),
            description: String::new(),
            source: Source::Aur,
            installed: None,
        }];
        app.state.clamp_lists();
        press(&mut app, KeyCode::Char('p'));
        assert!(
            matches!(&app.state.pkgbuild, PkgbuildView::Error(e) if e.contains(FAKE)),
            "{:?}",
            app.state.pkgbuild
        );
    }

    #[test]
    fn index_arrival_recomputes_the_active_query_locally() {
        let mut app = empty_app();
        app.state.tab = Tab::Search;
        app.state.query = FAKE.to_string();
        app.on_aur_index(AurIndex::from_pkgs(vec![aur_pkg(FAKE, &[])]));
        assert_eq!(
            app.state.aur_state,
            AurState::Done {
                count: 1,
                local: true
            }
        );
        assert_eq!(app.state.results.len(), 1);
        assert!(status_text(&app).contains("Índice del AUR: 1 paquetes"));
    }

    #[test]
    fn search_with_the_index_is_synchronous_and_has_no_debounce() {
        let mut app = empty_app();
        app.aur_index = Some(AurIndex::from_pkgs(vec![aur_pkg(FAKE, &[])]));
        app.state.tab = Tab::Search;
        app.state.input_mode = true;
        for c in FAKE.chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(
            app.state.aur_state,
            AurState::Done {
                count: 1,
                local: true
            }
        );
        assert!(app.aur_deadline.is_none(), "sin debounce");
    }
}

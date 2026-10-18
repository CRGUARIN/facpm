//! Lo que lee la interfaz, sin base de datos ni red.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;

use crate::backend::clean::{self, CleanTask};
use crate::backend::helper::{Helper, command_line};
use crate::backend::news::NewsItem;
use crate::backend::warnings::{Severity, Warning};
use crate::pkg::{Details, Package};
use crate::theme::{self, Theme};

const STATUS_TTL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Installed,
    Search,
    Updates,
    Clean,
}

impl Tab {
    pub fn idx(self) -> usize {
        match self {
            Tab::Installed => 0,
            Tab::Search => 1,
            Tab::Updates => 2,
            Tab::Clean => 3,
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Installed => Tab::Search,
            Tab::Search => Tab::Updates,
            Tab::Updates => Tab::Clean,
            Tab::Clean => Tab::Installed,
        }
    }

    pub fn prev(self) -> Tab {
        match self {
            Tab::Installed => Tab::Clean,
            Tab::Search => Tab::Installed,
            Tab::Updates => Tab::Search,
            Tab::Clean => Tab::Updates,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Details,
    Pkgbuild,
}

impl RightTab {
    pub fn toggled(self) -> Self {
        match self {
            RightTab::Details => RightTab::Pkgbuild,
            RightTab::Pkgbuild => RightTab::Details,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Install,
    Remove,
    Upgrade,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AurState {
    Idle,
    Loading,
    /// `local`: los resultados salieron del índice local; si no, del RPC.
    Done {
        count: usize,
        local: bool,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PkgbuildView {
    Empty,
    Loading,
    Error(String),
    Ready(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CleanRow {
    pub task: CleanTask,
    /// `None` si la tarea no se puede ejecutar.
    pub command: Option<String>,
}

pub fn clean_row(task: CleanTask, helper: Helper) -> CleanRow {
    let command = clean::command(&task, helper).map(|cmd| command_line(&cmd));
    CleanRow { task, command }
}

#[derive(Debug, Clone)]
pub struct UpgradePlan {
    pub repos: usize,
    pub aur: usize,
    pub lines: Vec<String>,
    pub more: usize,
    pub command: String,
    /// El helper es pacman y hay paquetes del AUR que no va a actualizar.
    pub pacman_warning: bool,
    /// Paquetes con advertencias Warn o Alarm: nombre, peor severidad y su etiqueta.
    pub flagged: Vec<(String, Severity, String)>,
    pub news_new: usize,
    pub news_intervention: bool,
}

#[derive(Debug, Clone)]
pub enum Modal {
    Confirm {
        kind: OpKind,
        pkgs: Vec<String>,
        command: String,
    },
    Upgrade(UpgradePlan),
    Clean {
        task: CleanTask,
        command: String,
    },
    /// Mientras está abierta, las teclas son suyas.
    Help,
    News,
    /// `selected` es la fila resaltada, ya aplicada en vivo; `original` el id con el que se abrió,
    /// al que vuelven Esc, q y t.
    Theme {
        selected: usize,
        original: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Error,
}

/// `since`: las noticias publicadas después son "nuevas" (última actualización del log, o 30 días).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsState {
    Loading,
    Loaded { items: Vec<NewsItem>, since: i64 },
    Error(String),
}

impl NewsState {
    pub fn new_items(&self) -> Vec<&NewsItem> {
        match self {
            NewsState::Loaded { items, since } => items
                .iter()
                .filter(|item| item.published > *since)
                .collect(),
            _ => Vec::new(),
        }
    }
}

pub struct Status {
    pub text: String,
    pub level: Level,
    pub at: Instant,
}

impl Status {
    pub fn is_recent(&self) -> bool {
        self.at.elapsed() < STATUS_TTL
    }
}

pub struct State {
    pub tab: Tab,
    /// Modo edición: las teclas escriben en el filtro o en la búsqueda.
    pub input_mode: bool,
    pub helper_name: &'static str,
    pub theme: &'static Theme,
    pub installed: Vec<Package>,
    pub filter: String,
    pub filtered: Vec<usize>,
    pub query: String,
    pub results: Vec<Package>,
    pub aur_state: AurState,
    /// Repos primero, después AUR. La versión es la nueva y `installed` la actual.
    pub updates: Vec<Package>,
    pub updates_pending: bool,
    pub updates_error: bool,
    /// Nombres con actualización, para el `↑` de Instalados.
    pub upgradable: HashSet<String>,
    pub warnings: HashMap<String, Vec<Warning>>,
    pub news: NewsState,
    pub clean: Vec<CleanRow>,
    pub clean_pending: bool,
    /// El panel derecho muestra su tamaño.
    pub orphans: Vec<(String, u64)>,
    /// Selección y scroll de cada lista. Índice = `Tab::idx`.
    pub lists: [ListState; 4],
    pub marked: [HashSet<String>; 4],
    pub details: Details,
    pub right_tab: RightTab,
    pub right_scroll: u16,
    /// Lo escribe la UI al dibujar, para medir media página.
    pub right_height: u16,
    pub pkgbuild: PkgbuildView,
    pub help_scroll: u16,
    /// Lo escribe la UI al dibujar, para medir media página.
    pub help_height: u16,
    pub modal: Option<Modal>,
    pub status: Option<Status>,
}

impl State {
    pub fn new(helper_name: &'static str) -> Self {
        Self {
            tab: Tab::Installed,
            input_mode: false,
            helper_name,
            theme: theme::default_theme(),
            installed: Vec::new(),
            filter: String::new(),
            filtered: Vec::new(),
            query: String::new(),
            results: Vec::new(),
            aur_state: AurState::Idle,
            updates: Vec::new(),
            updates_pending: false,
            updates_error: false,
            upgradable: HashSet::new(),
            warnings: HashMap::new(),
            news: NewsState::Loading,
            clean: Vec::new(),
            clean_pending: false,
            orphans: Vec::new(),
            lists: [
                ListState::default(),
                ListState::default(),
                ListState::default(),
                ListState::default(),
            ],
            marked: [
                HashSet::new(),
                HashSet::new(),
                HashSet::new(),
                HashSet::new(),
            ],
            details: Vec::new(),
            right_tab: RightTab::Details,
            right_scroll: 0,
            right_height: 0,
            pkgbuild: PkgbuildView::Empty,
            help_scroll: 0,
            help_height: 0,
            modal: None,
            status: None,
        }
    }

    pub fn set_installed(&mut self, pkgs: Vec<Package>) {
        self.installed = pkgs;
        self.refilter();
    }

    /// Recalcula `filtered`: cada término del filtro debe aparecer en nombre o descripción.
    pub fn refilter(&mut self) {
        let terms: Vec<String> = self
            .filter
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        self.filtered = self
            .installed
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let name = p.name.to_lowercase();
                let desc = p.description.to_lowercase();
                terms
                    .iter()
                    .all(|t| name.contains(t.as_str()) || desc.contains(t.as_str()))
            })
            .map(|(i, _)| i)
            .collect();
        self.clamp_lists();
    }

    pub fn rows(&self) -> Vec<&Package> {
        match self.tab {
            Tab::Installed => self.filtered.iter().map(|&i| &self.installed[i]).collect(),
            Tab::Search => self.results.iter().collect(),
            Tab::Updates => self.updates.iter().collect(),
            Tab::Clean => Vec::new(),
        }
    }

    pub fn selected(&self) -> Option<&Package> {
        let i = self.sel()?;
        self.rows().get(i).copied()
    }

    pub fn sel(&self) -> Option<usize> {
        self.lists[self.tab.idx()].selected()
    }

    pub fn selected_clean(&self) -> Option<&CleanRow> {
        if self.tab != Tab::Clean {
            return None;
        }
        self.clean.get(self.sel()?)
    }

    pub fn len_of(&self, tab: Tab) -> usize {
        match tab {
            Tab::Installed => self.filtered.len(),
            Tab::Search => self.results.len(),
            Tab::Updates => self.updates.len(),
            Tab::Clean => self.clean.len(),
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        let n = self.len_of(self.tab);
        let idx = self.tab.idx();
        if n == 0 {
            self.lists[idx].select(None);
            return;
        }
        let cur = self.lists[idx].selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, n as isize - 1) as usize;
        self.lists[idx].select(Some(next));
    }

    /// Selecciona la fila `i`, acotada a la lista. `usize::MAX` es la última.
    pub fn go_to(&mut self, i: usize) {
        let n = self.len_of(self.tab);
        let idx = self.tab.idx();
        self.lists[idx].select(if n == 0 { None } else { Some(i.min(n - 1)) });
    }

    pub fn select_first_row(&mut self) {
        self.go_to(0);
    }

    /// Deja la selección dentro de rango: 0 si hay filas, None si la lista está vacía.
    pub fn clamp_lists(&mut self) {
        for tab in [Tab::Installed, Tab::Search, Tab::Updates, Tab::Clean] {
            let n = self.len_of(tab);
            let list = &mut self.lists[tab.idx()];
            let next = match list.selected() {
                _ if n == 0 => None,
                None => Some(0),
                Some(i) => Some(i.min(n - 1)),
            };
            list.select(next);
        }
    }
}

/// Tareas de limpieza falsas para los tests: no leen el sistema ni ejecutan nada.
#[cfg(test)]
pub(crate) fn sample_clean_tasks() -> Vec<CleanTask> {
    use crate::backend::clean::CleanKind;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    vec![
        CleanTask {
            kind: CleanKind::Orphans,
            title: "Paquetes huérfanos".to_string(),
            description: "Paquetes instalados como dependencia que ya ningún paquete necesita."
                .to_string(),
            items: vec!["go".to_string(), "libxml2-legacy".to_string()],
            count: 2,
            bytes: Some(226 * MIB + 614 * KIB),
            note: None,
            available: true,
        },
        CleanTask {
            kind: CleanKind::PkgCacheKeepLatest,
            title: "Caché pacman: versiones viejas".to_string(),
            description: "Borra de la caché todas las versiones excepto la última de cada paquete. Sin versiones anteriores no podrás volver atrás sin conexión.".to_string(),
            items: Vec::new(),
            count: 51,
            bytes: Some(4 * GIB + 600 * MIB),
            note: None,
            available: true,
        },
        CleanTask {
            kind: CleanKind::PkgCacheUninstalled,
            title: "Caché pacman: desinstalados".to_string(),
            description: "Borra de la caché los paquetes que ya no están instalados.".to_string(),
            items: Vec::new(),
            count: 0,
            bytes: None,
            note: Some("requiere pacman-contrib (paccache)".to_string()),
            available: false,
        },
        CleanTask {
            kind: CleanKind::AurBuildCache,
            title: "Caché de compilación AUR (yay)".to_string(),
            description: "Borra los directorios de compilación del AUR en /home/usuario/.cache/yay."
                .to_string(),
            items: vec![
                "/home/usuario/.cache/yay/go-tools".to_string(),
                "/home/usuario/.cache/yay/zen-browser-bin".to_string(),
            ],
            count: 2,
            bytes: Some(5 * GIB),
            note: None,
            available: true,
        },
        CleanTask {
            kind: CleanKind::FacpmCache,
            title: "Caché de facpm (índice AUR)".to_string(),
            description: "Borra el índice local del AUR. Se vuelve a descargar la próxima vez que inicies facpm.".to_string(),
            items: vec!["/home/usuario/.cache/facpm/packages-meta-v1.json.gz".to_string()],
            count: 1,
            bytes: Some(10 * MIB),
            note: None,
            available: true,
        },
        CleanTask {
            kind: CleanKind::PkgCacheAll,
            title: "Caché pacman completa".to_string(),
            description: "Vacía por completo la caché de pacman (pacman -Scc). No podrás reinstalar ni volver a versiones anteriores sin conexión.".to_string(),
            items: Vec::new(),
            count: 1200,
            bytes: Some(12 * GIB),
            note: None,
            available: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::clean::CleanKind;
    use crate::pkg::Source;

    fn pkg(name: &str, description: &str) -> Package {
        Package {
            name: name.to_string(),
            version: "1.0-1".to_string(),
            description: description.to_string(),
            source: Source::Repo("extra".to_string()),
            installed: None,
        }
    }

    fn news_item(published: i64) -> NewsItem {
        NewsItem {
            title: String::new(),
            link: String::new(),
            published,
            date: String::new(),
            body: String::new(),
        }
    }

    #[test]
    fn filter_needs_every_term_in_name_or_description() {
        let mut st = State::new("pacman");
        st.set_installed(vec![
            pkg("git", "control de versiones"),
            pkg("lazygit", "interfaz de terminal para git"),
            pkg("vim", "editor de texto"),
        ]);
        st.filter = "git terminal".to_string();
        st.refilter();
        assert_eq!(st.filtered, vec![1]);

        st.filter = "GIT".to_string();
        st.refilter();
        assert_eq!(st.filtered, vec![0, 1]);

        st.filter.clear();
        st.refilter();
        assert_eq!(st.filtered, vec![0, 1, 2]);
    }

    #[test]
    fn move_sel_stops_at_both_ends() {
        let mut st = State::new("pacman");
        st.set_installed(vec![pkg("a", ""), pkg("b", ""), pkg("c", "")]);
        assert_eq!(st.sel(), Some(0));

        st.move_sel(-1);
        assert_eq!(st.sel(), Some(0));
        st.move_sel(10);
        assert_eq!(st.sel(), Some(2));
        st.move_sel(1);
        assert_eq!(st.sel(), Some(2));
    }

    #[test]
    fn move_sel_on_empty_list_has_no_selection() {
        let mut st = State::new("pacman");
        st.move_sel(1);
        assert_eq!(st.sel(), None);
    }

    #[test]
    fn go_to_clamps_to_the_last_row() {
        let mut st = State::new("pacman");
        st.go_to(5);
        assert_eq!(st.sel(), None);

        st.set_installed(vec![pkg("a", ""), pkg("b", "")]);
        st.go_to(usize::MAX);
        assert_eq!(st.sel(), Some(1));
        st.go_to(0);
        assert_eq!(st.sel(), Some(0));
    }

    #[test]
    fn clamp_lists_handles_empty_and_out_of_range_lists() {
        let mut st = State::new("pacman");
        st.clamp_lists();
        assert!(st.lists.iter().all(|list| list.selected().is_none()));

        st.results = vec![pkg("a", ""), pkg("b", "")];
        st.lists[Tab::Search.idx()].select(Some(7));
        st.clamp_lists();
        assert_eq!(st.lists[Tab::Search.idx()].selected(), Some(1));

        st.lists[Tab::Search.idx()].select(None);
        st.clamp_lists();
        assert_eq!(st.lists[Tab::Search.idx()].selected(), Some(0));
    }

    #[test]
    fn selected_clean_only_on_the_clean_tab() {
        let mut st = State::new("yay");
        st.clean = sample_clean_tasks()
            .into_iter()
            .map(|task| clean_row(task, Helper::Yay))
            .collect();
        st.clamp_lists();

        assert!(st.selected_clean().is_none());
        st.tab = Tab::Clean;
        let row = st.selected_clean().expect("hay filas de limpieza");
        assert_eq!(row.task.kind, CleanKind::Orphans);
        assert_eq!(row.command.as_deref(), Some("yay -Rns go libxml2-legacy"));
    }

    #[test]
    fn new_items_are_published_after_since() {
        let news = NewsState::Loaded {
            items: vec![news_item(100), news_item(200), news_item(300)],
            since: 200,
        };
        let fresh: Vec<i64> = news.new_items().iter().map(|item| item.published).collect();
        assert_eq!(fresh, vec![300]);

        assert!(NewsState::Loading.new_items().is_empty());
        assert!(
            NewsState::Error("sin red".to_string())
                .new_items()
                .is_empty()
        );
    }

    #[test]
    fn tab_next_and_prev_wrap_around() {
        assert_eq!(Tab::Installed.next(), Tab::Search);
        assert_eq!(Tab::Clean.next(), Tab::Installed);
        assert_eq!(Tab::Installed.prev(), Tab::Clean);
        for tab in [Tab::Installed, Tab::Search, Tab::Updates, Tab::Clean] {
            assert_eq!(tab.next().prev(), tab);
        }
        assert_eq!(Tab::Clean.idx(), 3);
    }
}

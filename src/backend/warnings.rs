use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use chrono::{Local, TimeZone};

use crate::backend::aur::AurPkg;
use crate::backend::updates::Update;
use crate::pkg::Source;

/// De menor a mayor: `Info < Warn < Alarm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warn,
    Alarm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    MaintainerChanged,
    Missing,
    Orphan,
    NewAndUnknown,
    OutOfDate,
    Vcs,
    PreRelease,
    TestingRepo,
    MajorBump,
    News,
    AurNotice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub kind: Kind,
    pub severity: Severity,
    /// Etiqueta corta (hasta 26 columnas) para las listas y el modal.
    pub label: String,
    /// Explicación completa para el panel de Detalles.
    pub text: String,
}

/// Último mantenedor visto de cada paquete del AUR instalado; None = huérfano.
pub type Maintainers = HashMap<String, Option<String>>;

const DAY: i64 = 24 * 60 * 60;
const NEW_DAYS: i64 = 30;
const FEW_VOTES: u64 = 10;
const NOBODY: &str = "(nadie)";
const PRE_WORDS: [&str; 8] = [
    "alpha", "beta", "rc", "pre", "preview", "dev", "nightly", "snapshot",
];
const VCS_SUFFIXES: [&str; 7] = ["-git", "-svn", "-hg", "-bzr", "-darcs", "-fossil", "-cvs"];

fn warning(kind: Kind, severity: Severity, label: &str, text: impl Into<String>) -> Warning {
    Warning {
        kind,
        severity,
        label: label.to_string(),
        text: text.into(),
    }
}

/// Señales de un paquete foráneo instalado que está en el índice. `previous` es el mantenedor
/// del último registro: None no tiene línea base, así que no hay `MaintainerChanged`.
pub fn aur_signals(pkg: &AurPkg, previous: Option<&Option<String>>, now: i64) -> Vec<Warning> {
    let mut out = Vec::new();

    if let Some(prev) = previous {
        let before = prev.as_deref();
        let after = pkg.maintainer.as_deref();
        if before != after {
            out.push(warning(
                Kind::MaintainerChanged,
                Severity::Alarm,
                "cambió el mantenedor",
                format!(
                    "El mantenedor cambió de {} a {} desde tu última actualización con facpm. \
                     Así se colaron los paquetes maliciosos del AUR: revisá el PKGBUILD (p) \
                     antes de actualizar.",
                    before.unwrap_or(NOBODY),
                    after.unwrap_or(NOBODY),
                ),
            ));
        }
    }

    if pkg.maintainer.is_none() {
        out.push(warning(
            Kind::Orphan,
            Severity::Warn,
            "huérfano",
            "No tiene mantenedor: nadie revisa sus cambios y cualquiera puede adoptarlo.",
        ));
    }

    let age = now.saturating_sub(pkg.first_submitted);
    if age < NEW_DAYS * DAY && pkg.num_votes < FEW_VOTES {
        out.push(warning(
            Kind::NewAndUnknown,
            Severity::Warn,
            "nuevo y poco votado",
            format!(
                "Se publicó hace {} días y tiene {} votos: los paquetes maliciosos suelen ser \
                 nuevos y poco conocidos.",
                age / DAY,
                pkg.num_votes,
            ),
        ));
    }

    if let Some(ts) = pkg.out_of_date {
        out.push(warning(
            Kind::OutOfDate,
            Severity::Info,
            "marcado desactualizado",
            format!("Marcado como desactualizado el {}.", date(ts)),
        ));
    }

    out
}

/// Señales de inestabilidad de una actualización, sea de un repo o del AUR.
pub fn update_signals(update: &Update) -> Vec<Warning> {
    let mut out = Vec::new();

    if is_vcs(&update.name) {
        out.push(warning(
            Kind::Vcs,
            Severity::Info,
            "VCS (rama de desarrollo)",
            "Se compila desde la rama de desarrollo del proyecto: puede romperse entre versiones.",
        ));
    }

    if is_prerelease(&update.new) {
        out.push(warning(
            Kind::PreRelease,
            Severity::Warn,
            "versión preliminar",
            format!(
                "La versión {} es preliminar (alpha/beta/rc): puede ser inestable.",
                update.new
            ),
        ));
    }

    if let Source::Repo(repo) = &update.source
        && is_testing_repo(repo)
    {
        out.push(warning(
            Kind::TestingRepo,
            Severity::Warn,
            "repo de pruebas",
            format!("Viene de {repo}, un repositorio de pruebas."),
        ));
    }

    if let Some(text) = major_bump_text(&update.current, &update.new) {
        out.push(warning(
            Kind::MajorBump,
            Severity::Info,
            "versión mayor",
            text,
        ));
    }

    out
}

/// Paquete foráneo que ya no está en el AUR.
pub fn missing() -> Warning {
    warning(
        Kind::Missing,
        Severity::Warn,
        "ya no está en el AUR",
        "No está en el AUR: lo borraron, lo renombraron o lo instalaste a mano. Arch borra los \
         paquetes maliciosos: si no sabés de dónde salió, revisalo.",
    )
}

/// Noticia de Arch que menciona un paquete instalado. `date` es "YYYY-MM-DD".
pub fn news(title: &str, date: &str, intervention: bool) -> Warning {
    let severity = if intervention {
        Severity::Alarm
    } else {
        Severity::Warn
    };
    warning(
        Kind::News,
        severity,
        "noticia de Arch",
        format!("Noticia de Arch del {date}: \"{title}\". Leela con n antes de actualizar."),
    )
}

/// Aviso de Arch sobre el AUR.
pub fn aur_notice(title: &str, date: &str) -> Warning {
    warning(
        Kind::AurNotice,
        Severity::Warn,
        "aviso de Arch sobre el AUR",
        format!("Arch publicó un aviso sobre el AUR el {date}: \"{title}\". Leelo con n."),
    )
}

/// Foráneo que no está en el AUR, salvo que sí esté: por su nombre; o, si termina en `-debug`,
/// su tronco (nombre sin el sufijo) por nombre o por base; o su pkgbase por base.
pub fn is_missing(
    name: &str,
    pkgbase: Option<&str>,
    has_name: impl Fn(&str) -> bool,
    has_base: impl Fn(&str) -> bool,
) -> bool {
    let present = has_name(name)
        || name
            .strip_suffix("-debug")
            .is_some_and(|stem| has_name(stem) || has_base(stem))
        || pkgbase.is_some_and(has_base);
    !present
}

/// De mayor a menor severidad. Es estable: a igual severidad se conserva el orden.
pub fn sort(warnings: &mut [Warning]) {
    warnings.sort_by_key(|w| std::cmp::Reverse(w.severity));
}

pub fn worst(warnings: &[Warning]) -> Option<Severity> {
    warnings.iter().map(|w| w.severity).max()
}

/// "YYYY-MM-DD" en hora local; un timestamp inválido da "?".
pub fn date(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or_else(|| "?".to_string(), |t| t.format("%Y-%m-%d").to_string())
}

/// Texto del salto de epoch o de versión mayor, si lo hay. Un epoch nuevo mayor gana aunque
/// alguna versión no empiece con dígitos. Sin salto de epoch, un número mayor que el viejo
/// (ambos menores de 1000) da el de versión mayor, y hacen falta dígitos iniciales en las dos.
fn major_bump_text(current: &str, new: &str) -> Option<String> {
    let (old_epoch, old_rest) = split_epoch(current);
    let (new_epoch, new_rest) = split_epoch(new);
    if new_epoch > old_epoch {
        return Some(format!(
            "Cambio de epoch ({current} → {new}): la numeración de versiones cambió y puede \
             traer cambios incompatibles."
        ));
    }
    let old_major = leading_number(old_rest)?;
    let new_major = leading_number(new_rest)?;
    if new_major > old_major && old_major < 1000 && new_major < 1000 {
        return Some(format!(
            "Cambio de versión mayor ({current} → {new}): puede traer cambios incompatibles."
        ));
    }
    None
}

/// Epoch (0 si no hay) y el resto de la versión.
fn split_epoch(version: &str) -> (u64, &str) {
    match version.split_once(':') {
        Some((epoch, rest)) => (epoch.parse().unwrap_or(0), rest),
        None => (0, version),
    }
}

/// Dígitos iniciales como número. Un número que no cabe en u64 cuenta como enorme.
fn leading_number(version: &str) -> Option<u64> {
    let end = version
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(version.len());
    (end > 0).then(|| version[..end].parse().unwrap_or(u64::MAX))
}

/// Alguna palabra de letras ASCII del upstream (sin epoch ni pkgrel) es de preliminar:
/// "6.10rc3" sí, "r123.gabcdef" no.
fn is_prerelease(version: &str) -> bool {
    let (_, without_epoch) = split_epoch(version);
    let upstream = without_epoch
        .rsplit_once('-')
        .map_or(without_epoch, |(upstream, _pkgrel)| upstream);
    upstream
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| PRE_WORDS.iter().any(|pre| word.eq_ignore_ascii_case(pre)))
}

fn is_vcs(name: &str) -> bool {
    VCS_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

fn is_testing_repo(repo: &str) -> bool {
    matches!(repo, "testing" | "staging")
        || repo.ends_with("-testing")
        || repo.ends_with("-staging")
}

const MAINTAINERS_FILE: &str = "aur-maintainers.tsv";
/// Ruta que no existe: marca que no hay HOME ni XDG_STATE_HOME absolutos.
const NO_HOME_PATH: &str = "/nonexistent/facpm/aur-maintainers.tsv";

/// `$XDG_STATE_HOME/facpm/aur-maintainers.tsv` o `~/.local/state/facpm/aur-maintainers.tsv`.
pub fn maintainers_path() -> PathBuf {
    maintainers_path_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// Si el registro falta o no se puede leer, queda vacío.
pub fn load_maintainers() -> Maintainers {
    load_from(&maintainers_path())
}

/// Escribe el registro en la ruta por defecto. Falla si no hay HOME ni XDG_STATE_HOME absolutos.
pub fn save_maintainers(m: &Maintainers) -> anyhow::Result<()> {
    save_maintainers_to(&maintainers_path(), m)
}

fn maintainers_path_from(xdg: Option<OsString>, home: Option<OsString>) -> PathBuf {
    absolute_dir(xdg)
        .or_else(|| absolute_dir(home).map(|home| home.join(".local").join("state")))
        .map_or_else(
            || PathBuf::from(NO_HOME_PATH),
            |base| base.join("facpm").join(MAINTAINERS_FILE),
        )
}

fn absolute_dir(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

fn load_from(path: &Path) -> Maintainers {
    fs::read_to_string(path)
        .map(|text| parse_maintainers(&text))
        .unwrap_or_default()
}

fn save_maintainers_to(path: &Path, m: &Maintainers) -> anyhow::Result<()> {
    if path == Path::new(NO_HOME_PATH) {
        bail!(
            "no hay HOME ni XDG_STATE_HOME absolutos: no se puede guardar el registro de mantenedores"
        );
    }
    let dir = path
        .parent()
        .context("la ruta del registro de mantenedores no tiene carpeta")?;
    fs::create_dir_all(dir).with_context(|| format!("no se pudo crear {}", dir.display()))?;
    let tmp = tmp_path(path);
    let result = write_then_rename(&tmp, path, &render_maintainers(m));
    if result.is_err() {
        fs::remove_file(&tmp).ok();
    }
    result.with_context(|| format!("no se pudo guardar {}", path.display()))
}

/// Líneas "nombre\tmantenedor". Las que no tienen tabulador o nombre se ignoran; un mantenedor
/// vacío es None.
pub fn parse_maintainers(text: &str) -> Maintainers {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, maintainer)| {
            let maintainer = (!maintainer.is_empty()).then(|| maintainer.to_string());
            (name.to_string(), maintainer)
        })
        .collect()
}

/// Una línea "nombre\tmantenedor\n" por paquete, ordenadas por nombre.
pub fn render_maintainers(m: &Maintainers) -> String {
    let mut names: Vec<&String> = m.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| format!("{name}\t{}\n", m[name].as_deref().unwrap_or("")))
        .collect()
}

/// Archivo temporal junto al destino, con el pid para no pisar a otra instancia.
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(name)
}

fn write_then_rename(tmp: &Path, path: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directorio temporal de la prueba, borrado al salir aunque la prueba falle.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("facpm-warnings-{}-{test}", std::process::id()));
            fs::remove_dir_all(&dir).ok();
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    const NOW: i64 = 1_800_000_000;

    /// Paquete del AUR con lo que usan las señales; el resto queda con valores fijos.
    fn aur(
        maintainer: Option<&str>,
        votes: u64,
        first_submitted: i64,
        out_of_date: Option<i64>,
    ) -> AurPkg {
        serde_json::from_value(serde_json::json!({
            "Name": "demo",
            "Version": "1.0-1",
            "NumVotes": votes,
            "Popularity": 0.0,
            "FirstSubmitted": first_submitted,
            "LastModified": 0,
            "PackageBase": "demo",
            "Maintainer": maintainer,
            "OutOfDate": out_of_date,
        }))
        .expect("paquete de prueba válido")
    }

    fn update(name: &str, current: &str, new: &str, source: Source) -> Update {
        Update {
            name: name.to_string(),
            current: current.to_string(),
            new: new.to_string(),
            source,
        }
    }

    fn repo_update(name: &str, current: &str, new: &str) -> Update {
        update(name, current, new, Source::Repo("extra".to_string()))
    }

    fn has(warnings: &[Warning], kind: Kind) -> bool {
        warnings.iter().any(|w| w.kind == kind)
    }

    fn find(warnings: &[Warning], kind: Kind) -> Option<&Warning> {
        warnings.iter().find(|w| w.kind == kind)
    }

    fn sample(severity: Severity, label: &str) -> Warning {
        warning(Kind::Vcs, severity, label, "")
    }

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    #[test]
    fn maintainer_changes_include_nobody_transitions() {
        let bob = aur(Some("bob"), 100, 0, None);
        let from_alice = aur_signals(&bob, Some(&Some("alice".to_string())), NOW);
        let changed = find(&from_alice, Kind::MaintainerChanged).expect("alice → bob dispara");
        assert_eq!(changed.severity, Severity::Alarm);
        assert!(changed.text.contains("alice") && changed.text.contains("bob"));

        let from_nobody = aur_signals(&bob, Some(&None), NOW);
        let adopted = find(&from_nobody, Kind::MaintainerChanged).expect("huérfano → bob dispara");
        assert!(adopted.text.contains("(nadie)") && adopted.text.contains("bob"));

        let abandoned = aur(None, 100, 0, None);
        let to_nobody = aur_signals(&abandoned, Some(&Some("alice".to_string())), NOW);
        let text = &find(&to_nobody, Kind::MaintainerChanged)
            .expect("alice → huérfano dispara")
            .text;
        assert!(text.contains("alice") && text.contains("(nadie)"));
    }

    #[test]
    fn unchanged_or_missing_baseline_is_quiet() {
        let alice = aur(Some("alice"), 100, 0, None);
        assert!(!has(
            &aur_signals(&alice, Some(&Some("alice".to_string())), NOW),
            Kind::MaintainerChanged
        ));
        assert!(!has(
            &aur_signals(&alice, None, NOW),
            Kind::MaintainerChanged
        ));
    }

    #[test]
    fn orphan_when_no_maintainer() {
        let orphan = aur(None, 100, 0, None);
        let orphan_ws = aur_signals(&orphan, None, NOW);
        let warning = find(&orphan_ws, Kind::Orphan).expect("sin mantenedor");
        assert_eq!(warning.severity, Severity::Warn);
        assert_eq!(warning.label, "huérfano");

        let owned = aur(Some("alice"), 100, 0, None);
        assert!(!has(&aur_signals(&owned, None, NOW), Kind::Orphan));
    }

    #[test]
    fn new_and_unknown_needs_young_and_few_votes() {
        let young = aur(Some("alice"), 9, NOW - 29 * DAY, None);
        let young_ws = aur_signals(&young, None, NOW);
        let warning = find(&young_ws, Kind::NewAndUnknown).expect("29 días y 9 votos");
        assert_eq!(warning.severity, Severity::Warn);
        assert!(warning.text.contains("29 días"));
        assert!(warning.text.contains("9 votos"));

        let old = aur(Some("alice"), 9, NOW - 31 * DAY, None);
        assert!(!has(&aur_signals(&old, None, NOW), Kind::NewAndUnknown));

        let popular = aur(Some("alice"), 10, NOW - 29 * DAY, None);
        assert!(!has(&aur_signals(&popular, None, NOW), Kind::NewAndUnknown));
    }

    #[test]
    fn out_of_date_is_info_with_date() {
        let ts = NOW - 3 * DAY;
        let pkg = aur(Some("alice"), 100, 0, Some(ts));
        let pkg_ws = aur_signals(&pkg, None, NOW);
        let warning = find(&pkg_ws, Kind::OutOfDate).expect("marcado desactualizado");
        assert_eq!(warning.severity, Severity::Info);
        assert_eq!(warning.label, "marcado desactualizado");
        assert!(warning.text.contains(&date(ts)));
    }

    #[test]
    fn aur_signals_keep_fixed_order() {
        let pkg = aur(None, 1, NOW - DAY, Some(NOW - 2 * DAY));
        let kinds: Vec<Kind> = aur_signals(&pkg, Some(&Some("alice".to_string())), NOW)
            .iter()
            .map(|w| w.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                Kind::MaintainerChanged,
                Kind::Orphan,
                Kind::NewAndUnknown,
                Kind::OutOfDate
            ]
        );
    }

    #[test]
    fn vcs_by_name_suffix() {
        for name in [
            "foo-git", "bar-svn", "baz-hg", "qux-bzr", "a-darcs", "b-fossil", "c-cvs",
        ] {
            let w = update_signals(&repo_update(name, "1.0-1", "1.1-1"));
            let warning = find(&w, Kind::Vcs).unwrap_or_else(|| panic!("{name} debe disparar"));
            assert_eq!(warning.severity, Severity::Info);
        }
        assert!(!has(
            &update_signals(&repo_update("gitg", "1.0-1", "1.1-1")),
            Kind::Vcs
        ));
    }

    #[test]
    fn prerelease_versions_are_flagged_and_stable_ones_are_not() {
        for new in [
            "6.10rc3-1",
            "2.0.0.beta1-1",
            "1:2.0rc1-1",
            "3.0-preview2-1",
            "2.0-RC1-1",
            "1.0-alpha-1",
        ] {
            let w = update_signals(&repo_update("p", "1.0-1", new));
            let warning =
                find(&w, Kind::PreRelease).unwrap_or_else(|| panic!("{new} debe disparar"));
            assert_eq!(warning.severity, Severity::Warn);
            assert!(warning.text.contains(new));
        }
        for new in ["1.0-1", "r123.gabcdef-1", "20260817.0-2", "1.0-premium-1"] {
            let w = update_signals(&repo_update("p", "1.0-1", new));
            assert!(!has(&w, Kind::PreRelease), "{new} no debe disparar");
        }
    }

    #[test]
    fn testing_repos_by_name() {
        for repo in ["testing", "staging", "core-testing", "extra-staging"] {
            let u = update("p", "1.0-1", "1.1-1", Source::Repo(repo.to_string()));
            let w = update_signals(&u);
            let warning =
                find(&w, Kind::TestingRepo).unwrap_or_else(|| panic!("{repo} debe disparar"));
            assert_eq!(warning.severity, Severity::Warn);
            assert!(warning.text.contains(repo));
        }
        let extra = update("p", "1.0-1", "1.1-1", Source::Repo("extra".to_string()));
        assert!(!has(&update_signals(&extra), Kind::TestingRepo));
        let aur = update("p", "1.0-1", "1.1-1", Source::Aur);
        assert!(!has(&update_signals(&aur), Kind::TestingRepo));
    }

    #[test]
    fn major_bump_by_major_number() {
        let w = update_signals(&repo_update("p", "1.9-1", "2.0-1"));
        let warning = find(&w, Kind::MajorBump).expect("1.9 → 2.0");
        assert_eq!(warning.severity, Severity::Info);
        assert_eq!(warning.label, "versión mayor");
        assert!(
            warning
                .text
                .starts_with("Cambio de versión mayor (1.9-1 → 2.0-1)")
        );

        assert!(!has(
            &update_signals(&repo_update("p", "1.0-1", "1.5-1")),
            Kind::MajorBump
        ));
        assert!(!has(
            &update_signals(&repo_update("p", "20260817-1", "20260901-1")),
            Kind::MajorBump
        ));
        assert!(!has(
            &update_signals(&repo_update("p", "999-1", "1000-1")),
            Kind::MajorBump
        ));
        assert!(has(
            &update_signals(&repo_update("p", "998-1", "999-1")),
            Kind::MajorBump
        ));
    }

    #[test]
    fn major_bump_by_epoch_uses_epoch_text_even_without_digits() {
        let w = update_signals(&repo_update("p", "1:1.0-1", "2:0.5-1"));
        let warning = find(&w, Kind::MajorBump).expect("epoch 1 → 2");
        assert_eq!(warning.severity, Severity::Info);
        assert_eq!(
            warning.text,
            "Cambio de epoch (1:1.0-1 → 2:0.5-1): la numeración de versiones cambió y puede \
             traer cambios incompatibles."
        );

        let w = update_signals(&repo_update("p", "1:abc-1", "2:def-1"));
        let warning = find(&w, Kind::MajorBump).expect("epoch 1 → 2 sin dígitos");
        assert!(
            warning
                .text
                .starts_with("Cambio de epoch (1:abc-1 → 2:def-1)")
        );
    }

    #[test]
    fn major_bump_needs_leading_digits() {
        assert!(!has(
            &update_signals(&repo_update("p", "abc", "def")),
            Kind::MajorBump
        ));
    }

    #[test]
    fn is_missing_is_false_when_known_by_name_debug_stem_or_base() {
        assert!(!is_missing("foo", None, |n| n == "foo", |_| false));
        assert!(!is_missing("foo-debug", None, |n| n == "foo", |_| false));
        assert!(!is_missing(
            "foo-bin",
            Some("foo-base"),
            |_| false,
            |b| b == "foo-base"
        ));
        assert!(!is_missing("foo-debug", None, |_| false, |b| b == "foo"));
    }

    #[test]
    fn is_missing_is_true_when_unknown_everywhere() {
        assert!(is_missing("ausente", None, |_| false, |_| false));
        assert!(is_missing(
            "ausente-debug",
            Some("ausente"),
            |_| false,
            |_| false
        ));
    }

    #[test]
    fn sort_orders_by_severity_desc_and_is_stable() {
        let mut ws = vec![
            sample(Severity::Info, "a"),
            sample(Severity::Alarm, "b"),
            sample(Severity::Warn, "c"),
            sample(Severity::Alarm, "d"),
        ];
        sort(&mut ws);
        let labels: Vec<&str> = ws.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["b", "d", "c", "a"]);
    }

    #[test]
    fn worst_is_the_highest_or_none() {
        assert_eq!(worst(&[]), None);
        assert_eq!(
            worst(&[sample(Severity::Info, "a"), sample(Severity::Warn, "b")]),
            Some(Severity::Warn)
        );
        assert_eq!(
            worst(&[
                sample(Severity::Warn, "a"),
                sample(Severity::Alarm, "b"),
                sample(Severity::Info, "c"),
            ]),
            Some(Severity::Alarm)
        );
    }

    #[test]
    fn constructors_have_fixed_labels_and_texts() {
        let m = missing();
        assert_eq!((m.kind, m.severity), (Kind::Missing, Severity::Warn));
        assert_eq!(m.label, "ya no está en el AUR");

        let alarm = news("Título", "2026-10-17", true);
        assert_eq!((alarm.kind, alarm.severity), (Kind::News, Severity::Alarm));
        assert_eq!(alarm.label, "noticia de Arch");
        assert_eq!(
            alarm.text,
            "Noticia de Arch del 2026-10-17: \"Título\". Leela con n antes de actualizar."
        );
        assert_eq!(news("Título", "2026-10-17", false).severity, Severity::Warn);

        let notice = aur_notice("Aviso", "2026-06-01");
        assert_eq!(
            (notice.kind, notice.severity),
            (Kind::AurNotice, Severity::Warn)
        );
        assert_eq!(notice.label, "aviso de Arch sobre el AUR");
        assert_eq!(
            notice.text,
            "Arch publicó un aviso sobre el AUR el 2026-06-01: \"Aviso\". Leelo con n."
        );
    }

    #[test]
    fn date_is_ymd_or_question_mark_when_invalid() {
        let s = date(NOW);
        assert_eq!(s.len(), 10, "obtenido {s:?}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(date(i64::MAX), "?");
    }

    #[test]
    fn maintainers_path_uses_absolute_xdg_then_home() {
        assert_eq!(
            maintainers_path_from(os("/x/state"), os("/home/u")),
            PathBuf::from("/x/state/facpm/aur-maintainers.tsv")
        );
        assert_eq!(
            maintainers_path_from(os("relativo"), os("/home/u")),
            PathBuf::from("/home/u/.local/state/facpm/aur-maintainers.tsv")
        );
        assert_eq!(
            maintainers_path_from(os(""), os("/home/u")),
            PathBuf::from("/home/u/.local/state/facpm/aur-maintainers.tsv")
        );
    }

    #[test]
    fn maintainers_path_without_absolute_bases_is_the_sentinel() {
        assert_eq!(
            maintainers_path_from(None, None),
            PathBuf::from(NO_HOME_PATH)
        );
        assert_eq!(
            maintainers_path_from(None, os("")),
            PathBuf::from(NO_HOME_PATH)
        );
        assert_eq!(
            maintainers_path_from(os("relativo"), os("relativo")),
            PathBuf::from(NO_HOME_PATH)
        );
    }

    #[test]
    fn render_and_parse_are_inverse_with_orphans_sorted() {
        let mut m = Maintainers::new();
        m.insert("yay".to_string(), Some("alguien".to_string()));
        m.insert("abandonado".to_string(), None);
        let text = render_maintainers(&m);
        assert_eq!(text, "abandonado\t\nyay\talguien\n");
        assert_eq!(parse_maintainers(&text), m);
    }

    #[test]
    fn parse_ignores_lines_without_tab_or_name() {
        let m = parse_maintainers("sin tabulador\n\talguien\n\t\n   \nyay\talguien\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m["yay"], Some("alguien".to_string()));
    }

    #[test]
    fn save_writes_a_file_that_loads_back_and_leaves_no_tmp() {
        let tmp = TempDir::new("save");
        let path = tmp.path().join("anidada").join(MAINTAINERS_FILE);
        let mut m = Maintainers::new();
        m.insert("yay".to_string(), Some("alguien".to_string()));
        m.insert("abandonado".to_string(), None);

        save_maintainers_to(&path, &m).unwrap();
        assert_eq!(load_from(&path), m);
        assert!(!tmp_path(&path).exists());

        m.insert("yay".to_string(), Some("otra".to_string()));
        save_maintainers_to(&path, &m).unwrap();
        assert_eq!(load_from(&path), m);
    }

    #[test]
    fn save_refuses_the_sentinel_path_without_writing() {
        let m = Maintainers::new();
        assert!(save_maintainers_to(Path::new(NO_HOME_PATH), &m).is_err());
    }

    #[test]
    fn load_from_missing_file_is_empty() {
        let tmp = TempDir::new("load");
        assert!(load_from(&tmp.path().join("no-existe.tsv")).is_empty());
    }
}

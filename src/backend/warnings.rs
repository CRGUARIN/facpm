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

/// Lowest to highest: `Info < Warn < Alarm`.
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
    /// Short label (up to 26 columns) for the lists and the modal.
    pub label: String,
    /// Full explanation for the Details panel.
    pub text: String,
}

/// Last maintainer seen for each installed AUR package; None = orphaned.
pub type Maintainers = HashMap<String, Option<String>>;

const DAY: i64 = 24 * 60 * 60;
const NEW_DAYS: i64 = 30;
const FEW_VOTES: u64 = 10;
const NOBODY: &str = "(nobody)";
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

/// Signals for an installed foreign package that is in the index. `previous` is the maintainer
/// from the last record: None has no baseline, so there is no `MaintainerChanged`.
pub fn aur_signals(pkg: &AurPkg, previous: Option<&Option<String>>, now: i64) -> Vec<Warning> {
    let mut out = Vec::new();

    if let Some(prev) = previous {
        let before = prev.as_deref();
        let after = pkg.maintainer.as_deref();
        if before != after {
            out.push(warning(
                Kind::MaintainerChanged,
                Severity::Alarm,
                "maintainer changed",
                format!(
                    "The maintainer changed from {} to {} since your last upgrade with facpm. \
                     That's how the malicious AUR packages got in: review the PKGBUILD (p) \
                     before upgrading.",
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
            "orphaned",
            "It has no maintainer: nobody reviews its changes, and anyone can adopt it.",
        ));
    }

    let age = now.saturating_sub(pkg.first_submitted);
    if age < NEW_DAYS * DAY && pkg.num_votes < FEW_VOTES {
        out.push(warning(
            Kind::NewAndUnknown,
            Severity::Warn,
            "new, few votes",
            format!(
                "Published {} days ago with {} votes: malicious packages tend to be \
                 new and little known.",
                age / DAY,
                pkg.num_votes,
            ),
        ));
    }

    if let Some(ts) = pkg.out_of_date {
        out.push(warning(
            Kind::OutOfDate,
            Severity::Info,
            "flagged out of date",
            format!("Flagged as out of date on {}.", date(ts)),
        ));
    }

    out
}

/// Instability signals for an update, from a repo or from the AUR.
pub fn update_signals(update: &Update) -> Vec<Warning> {
    let mut out = Vec::new();

    if is_vcs(&update.name) {
        out.push(warning(
            Kind::Vcs,
            Severity::Info,
            "VCS (development branch)",
            "Built from the project's development branch: it may break between versions.",
        ));
    }

    if is_prerelease(&update.new) {
        out.push(warning(
            Kind::PreRelease,
            Severity::Warn,
            "pre-release",
            format!(
                "Version {} is a pre-release (alpha/beta/rc): it may be unstable.",
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
            "testing repo",
            format!("Comes from {repo}, a testing repository."),
        ));
    }

    if let Some(text) = major_bump_text(&update.current, &update.new) {
        out.push(warning(
            Kind::MajorBump,
            Severity::Info,
            "major version",
            text,
        ));
    }

    out
}

/// Foreign package that is no longer in the AUR.
pub fn missing() -> Warning {
    warning(
        Kind::Missing,
        Severity::Warn,
        "no longer in the AUR",
        "Not in the AUR: it was deleted, renamed, or installed by hand. Arch removes \
         malicious packages: if you do not know where it came from, check it.",
    )
}

/// Arch news post that mentions an installed package. `date` is "YYYY-MM-DD".
pub fn news(title: &str, date: &str, intervention: bool) -> Warning {
    let severity = if intervention {
        Severity::Alarm
    } else {
        Severity::Warn
    };
    warning(
        Kind::News,
        severity,
        "Arch news",
        format!("Arch news from {date}: \"{title}\". Read it with n before upgrading."),
    )
}

/// Arch notice about the AUR.
pub fn aur_notice(title: &str, date: &str) -> Warning {
    warning(
        Kind::AurNotice,
        Severity::Warn,
        "Arch notice about the AUR",
        format!("Arch posted a notice about the AUR on {date}: \"{title}\". Read it with n."),
    )
}

/// Foreign package that is not in the AUR, unless it is: by its name; or, if it ends in `-debug`,
/// its stem (the name without the suffix) by name or base; or its pkgbase by base.
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

/// From highest to lowest severity. It is stable: equal severities keep their order.
pub fn sort(warnings: &mut [Warning]) {
    warnings.sort_by_key(|w| std::cmp::Reverse(w.severity));
}

pub fn worst(warnings: &[Warning]) -> Option<Severity> {
    warnings.iter().map(|w| w.severity).max()
}

/// "YYYY-MM-DD" in local time; an invalid timestamp gives "?".
pub fn date(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or_else(|| "?".to_string(), |t| t.format("%Y-%m-%d").to_string())
}

/// Text for an epoch or major version jump, if there is one. A higher new epoch wins even if
/// a version does not start with digits. Without an epoch jump, a new number above the old one
/// (both below 1000) gives the major version text, and both need leading digits.
fn major_bump_text(current: &str, new: &str) -> Option<String> {
    let (old_epoch, old_rest) = split_epoch(current);
    let (new_epoch, new_rest) = split_epoch(new);
    if new_epoch > old_epoch {
        return Some(format!(
            "Epoch change ({current} → {new}): the version numbering changed and it may \
             bring incompatible changes."
        ));
    }
    let old_major = leading_number(old_rest)?;
    let new_major = leading_number(new_rest)?;
    if new_major > old_major && old_major < 1000 && new_major < 1000 {
        return Some(format!(
            "Major version change ({current} → {new}): it may bring incompatible changes."
        ));
    }
    None
}

/// Epoch (0 if none) and the rest of the version.
fn split_epoch(version: &str) -> (u64, &str) {
    match version.split_once(':') {
        Some((epoch, rest)) => (epoch.parse().unwrap_or(0), rest),
        None => (0, version),
    }
}

/// Leading digits as a number. A number that does not fit in u64 counts as huge.
fn leading_number(version: &str) -> Option<u64> {
    let end = version
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(version.len());
    (end > 0).then(|| version[..end].parse().unwrap_or(u64::MAX))
}

/// An ASCII-letter word of the upstream version (without epoch or pkgrel) marks a pre-release:
/// "6.10rc3" does, "r123.gabcdef" does not.
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
/// A path that does not exist: it marks that there is no absolute HOME or XDG_STATE_HOME.
const NO_HOME_PATH: &str = "/nonexistent/facpm/aur-maintainers.tsv";

/// `$XDG_STATE_HOME/facpm/aur-maintainers.tsv` or `~/.local/state/facpm/aur-maintainers.tsv`.
pub fn maintainers_path() -> PathBuf {
    maintainers_path_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// If the record is missing or cannot be read, it is empty.
pub fn load_maintainers() -> Maintainers {
    load_from(&maintainers_path())
}

/// Writes the record to the default path. Fails if there is no absolute HOME or XDG_STATE_HOME.
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
        bail!("no absolute HOME or XDG_STATE_HOME, couldn't save the maintainer record");
    }
    let dir = path
        .parent()
        .context("the maintainer record path has no folder")?;
    fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let tmp = tmp_path(path);
    let result = write_then_rename(&tmp, path, &render_maintainers(m));
    if result.is_err() {
        fs::remove_file(&tmp).ok();
    }
    result.with_context(|| format!("couldn't save {}", path.display()))
}

/// Lines "name\tmaintainer". Lines without a tab or a name are ignored; an empty maintainer
/// is None.
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

/// One line "name\tmaintainer\n" per package, sorted by name.
pub fn render_maintainers(m: &Maintainers) -> String {
    let mut names: Vec<&String> = m.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| format!("{name}\t{}\n", m[name].as_deref().unwrap_or("")))
        .collect()
}

/// Temporary file next to the target, with the pid so it does not clash with another instance.
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

    /// Temporary test directory, deleted on exit even if the test fails.
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

    /// AUR package with what the signals use; the rest has fixed values.
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
        .expect("valid test package")
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
        let changed = find(&from_alice, Kind::MaintainerChanged).expect("alice → bob triggers");
        assert_eq!(changed.severity, Severity::Alarm);
        assert!(changed.text.contains("alice") && changed.text.contains("bob"));

        let from_nobody = aur_signals(&bob, Some(&None), NOW);
        let adopted = find(&from_nobody, Kind::MaintainerChanged).expect("orphaned → bob triggers");
        assert!(adopted.text.contains("(nobody)") && adopted.text.contains("bob"));

        let abandoned = aur(None, 100, 0, None);
        let to_nobody = aur_signals(&abandoned, Some(&Some("alice".to_string())), NOW);
        let text = &find(&to_nobody, Kind::MaintainerChanged)
            .expect("alice → orphaned triggers")
            .text;
        assert!(text.contains("alice") && text.contains("(nobody)"));
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
        let warning = find(&orphan_ws, Kind::Orphan).expect("no maintainer");
        assert_eq!(warning.severity, Severity::Warn);
        assert_eq!(warning.label, "orphaned");

        let owned = aur(Some("alice"), 100, 0, None);
        assert!(!has(&aur_signals(&owned, None, NOW), Kind::Orphan));
    }

    #[test]
    fn new_and_unknown_needs_young_and_few_votes() {
        let young = aur(Some("alice"), 9, NOW - 29 * DAY, None);
        let young_ws = aur_signals(&young, None, NOW);
        let warning = find(&young_ws, Kind::NewAndUnknown).expect("29 days and 9 votes");
        assert_eq!(warning.severity, Severity::Warn);
        assert!(warning.text.contains("29 days"));
        assert!(warning.text.contains("9 votes"));

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
        let warning = find(&pkg_ws, Kind::OutOfDate).expect("flagged out of date");
        assert_eq!(warning.severity, Severity::Info);
        assert_eq!(warning.label, "flagged out of date");
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
            let warning = find(&w, Kind::Vcs).unwrap_or_else(|| panic!("{name} should trigger"));
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
                find(&w, Kind::PreRelease).unwrap_or_else(|| panic!("{new} should trigger"));
            assert_eq!(warning.severity, Severity::Warn);
            assert!(warning.text.contains(new));
        }
        for new in ["1.0-1", "r123.gabcdef-1", "20260817.0-2", "1.0-premium-1"] {
            let w = update_signals(&repo_update("p", "1.0-1", new));
            assert!(!has(&w, Kind::PreRelease), "{new} must not trigger");
        }
    }

    #[test]
    fn testing_repos_by_name() {
        for repo in ["testing", "staging", "core-testing", "extra-staging"] {
            let u = update("p", "1.0-1", "1.1-1", Source::Repo(repo.to_string()));
            let w = update_signals(&u);
            let warning =
                find(&w, Kind::TestingRepo).unwrap_or_else(|| panic!("{repo} should trigger"));
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
        assert_eq!(warning.label, "major version");
        assert!(
            warning
                .text
                .starts_with("Major version change (1.9-1 → 2.0-1)")
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
            "Epoch change (1:1.0-1 → 2:0.5-1): the version numbering changed and it may \
             bring incompatible changes."
        );

        let w = update_signals(&repo_update("p", "1:abc-1", "2:def-1"));
        let warning = find(&w, Kind::MajorBump).expect("epoch 1 → 2 without digits");
        assert!(warning.text.starts_with("Epoch change (1:abc-1 → 2:def-1)"));
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
        assert_eq!(m.label, "no longer in the AUR");

        let alarm = news("Title", "2026-10-17", true);
        assert_eq!((alarm.kind, alarm.severity), (Kind::News, Severity::Alarm));
        assert_eq!(alarm.label, "Arch news");
        assert_eq!(
            alarm.text,
            "Arch news from 2026-10-17: \"Title\". Read it with n before upgrading."
        );
        assert_eq!(news("Title", "2026-10-17", false).severity, Severity::Warn);

        let notice = aur_notice("Notice", "2026-06-01");
        assert_eq!(
            (notice.kind, notice.severity),
            (Kind::AurNotice, Severity::Warn)
        );
        assert_eq!(notice.label, "Arch notice about the AUR");
        assert_eq!(
            notice.text,
            "Arch posted a notice about the AUR on 2026-06-01: \"Notice\". Read it with n."
        );
    }

    #[test]
    fn date_is_ymd_or_question_mark_when_invalid() {
        let s = date(NOW);
        assert_eq!(s.len(), 10, "got {s:?}");
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
            maintainers_path_from(os("relative"), os("/home/u")),
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
            maintainers_path_from(os("relative"), os("relative")),
            PathBuf::from(NO_HOME_PATH)
        );
    }

    #[test]
    fn render_and_parse_are_inverse_with_orphans_sorted() {
        let mut m = Maintainers::new();
        m.insert("yay".to_string(), Some("someone".to_string()));
        m.insert("abandoned".to_string(), None);
        let text = render_maintainers(&m);
        assert_eq!(text, "abandoned\t\nyay\tsomeone\n");
        assert_eq!(parse_maintainers(&text), m);
    }

    #[test]
    fn parse_ignores_lines_without_tab_or_name() {
        let m = parse_maintainers("no tab\n\tsomeone\n\t\n   \nyay\tsomeone\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m["yay"], Some("someone".to_string()));
    }

    #[test]
    fn save_writes_a_file_that_loads_back_and_leaves_no_tmp() {
        let tmp = TempDir::new("save");
        let path = tmp.path().join("anidada").join(MAINTAINERS_FILE);
        let mut m = Maintainers::new();
        m.insert("yay".to_string(), Some("someone".to_string()));
        m.insert("abandoned".to_string(), None);

        save_maintainers_to(&path, &m).unwrap();
        assert_eq!(load_from(&path), m);
        assert!(!tmp_path(&path).exists());

        m.insert("yay".to_string(), Some("someone-else".to_string()));
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
        assert!(load_from(&tmp.path().join("missing.tsv")).is_empty());
    }
}

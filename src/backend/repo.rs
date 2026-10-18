use std::borrow::Cow;
use std::process::Command;

use anyhow::{Context, bail};
use chrono::{DateTime, Local};

use crate::backend::updates::Update;
use crate::pkg::{Details, Package, Source};

const EMPTY: &str = "—";

pub struct RepoDb {
    handle: alpm::Alpm,
    /// Where the synced repos come from. The `app` tests replace it, so they don't run `pacman-conf`.
    repo_list: fn() -> anyhow::Result<Vec<String>>,
}

impl RepoDb {
    pub fn new() -> anyhow::Result<Self> {
        Self::with_repo_list(configured_repos)
    }

    pub(crate) fn with_repo_list(
        repo_list: fn() -> anyhow::Result<Vec<String>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            handle: open_handle(repo_list)?,
            repo_list,
        })
    }

    /// Reopens the handle, for example after installing or uninstalling.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        self.handle = open_handle(self.repo_list)?;
        Ok(())
    }

    /// `source` is the repo if the package is in some syncdb; otherwise `Foreign`.
    pub fn installed(&self) -> Vec<Package> {
        let mut out: Vec<Package> = self
            .handle
            .localdb()
            .pkgs()
            .iter()
            .map(|p| {
                let name = p.name();
                let version = version_of(p);
                Package {
                    name: name.to_string(),
                    description: p.desc().unwrap_or("").to_string(),
                    source: self.source_of(name),
                    installed: Some(version.clone()),
                    version,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// All terms must appear in the name or the description. Order: exact name,
    /// prefix, name with all terms, description only; ties sorted by name.
    pub fn search(&self, query: &str) -> Vec<Package> {
        let lowered = query.to_lowercase();
        let terms: Vec<&str> = lowered.split_whitespace().collect();
        let Some(&first) = terms.first() else {
            return Vec::new();
        };
        let normalized = terms.join(" ");

        let mut hits: Vec<(u8, &alpm::Db, &alpm::Package)> = Vec::new();
        for db in self.handle.syncdbs().iter() {
            for p in db.pkgs().iter() {
                let name = p.name();
                let description = p.desc().unwrap_or("");
                let matches = terms
                    .iter()
                    .all(|t| contains_lc(name, t) || contains_lc(description, t));
                if !matches {
                    continue;
                }
                let all_in_name = terms.iter().all(|t| contains_lc(name, t));
                let name_lc = to_lower(name);
                let rank = if name_lc == normalized.as_str() {
                    0
                } else if name_lc.starts_with(first) {
                    1
                } else if all_in_name {
                    2
                } else {
                    3
                };
                hits.push((rank, db, p));
            }
        }

        // `sort_by` is stable: ties on rank and name keep the repos' order.
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.name().cmp(b.2.name())));
        hits.into_iter()
            .map(|(_, db, p)| {
                let name = p.name();
                Package {
                    name: name.to_string(),
                    version: version_of(p),
                    description: p.desc().unwrap_or("").to_string(),
                    source: Source::Repo(db.name().to_string()),
                    installed: self.installed_version(name),
                }
            })
            .collect()
    }

    /// Details by name: from the localdb if installed; otherwise from the first syncdb that has it.
    pub fn details(&self, name: &str) -> Option<Details> {
        let local = self.handle.localdb().pkg(name).ok();
        let (p, repo) = match (local, self.sync_pkg(name)) {
            (Some(p), Some((db, _))) => (p, db.name()),
            (Some(p), None) => (p, "local"),
            (None, Some((db, p))) => (p, db.name()),
            (None, None) => return None,
        };
        let installed = local.is_some();

        let mut rows: Details = vec![
            row("Name", p.name().to_string()),
            row("Version", version_of(p)),
            row("Description", or_dash(p.desc())),
            row("Repository", repo.to_string()),
            row("URL", or_dash(p.url())),
            row(
                "Licenses",
                join_list(p.licenses().iter().map(|s| s.to_string()), "  "),
            ),
            row(
                "Depends on",
                join_list(p.depends().iter().map(|d| d.to_string()), "  "),
            ),
            row(
                "Optional dependencies",
                join_list(p.optdepends().iter().map(|d| d.to_string()), "\n"),
            ),
        ];
        if installed {
            rows.push(row(
                "Required by",
                join_list(p.required_by().iter().map(|s| s.to_string()), "  "),
            ));
        }
        rows.push(row("Installed size", human_size(size_bytes(p.isize()))));
        if !installed {
            rows.push(row("Download size", human_size(size_bytes(p.size()))));
        }
        rows.push(row("Packager", or_dash(p.packager())));
        rows.push(row("Build date", format_date(p.build_date())));
        if installed {
            let reason = match p.reason() {
                alpm::PackageReason::Explicit => "Explicit",
                alpm::PackageReason::Depend => "Dependency",
            };
            rows.push(row(
                "Install date",
                p.install_date()
                    .map_or_else(|| EMPTY.to_string(), format_date),
            ));
            rows.push(row("Install reason", reason.to_string()));
        }
        Some(rows)
    }

    /// Like `pacman -Qdt`: dependencies that nothing requires and nothing lists as optional.
    pub fn orphans(&self) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = self
            .handle
            .localdb()
            .pkgs()
            .iter()
            .filter(|p| p.reason() == alpm::PackageReason::Depend)
            .filter(|p| p.required_by().is_empty() && p.optional_for().is_empty())
            .map(|p| (p.name().to_string(), size_bytes(p.isize())))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn installed_version(&self, name: &str) -> Option<String> {
        self.handle.localdb().pkg(name).ok().map(version_of)
    }

    /// `Package::base()`: from the localdb if installed; otherwise from the first syncdb.
    pub fn pkgbase(&self, name: &str) -> Option<String> {
        let base = match self.handle.localdb().pkg(name) {
            Ok(pkg) => pkg.base(),
            Err(_) => self.sync_pkg(name)?.1.base(),
        };
        Some(base.unwrap_or(name).to_string())
    }

    pub fn repo_of(&self, name: &str) -> Option<String> {
        self.sync_pkg(name).map(|(db, _)| db.name().to_string())
    }

    /// From the local syncdbs, which may be stale: the fallback if checkupdates fails.
    pub fn local_updates(&self) -> Vec<Update> {
        let mut updates: Vec<Update> = self
            .handle
            .localdb()
            .pkgs()
            .iter()
            .filter_map(|pkg| {
                let name = pkg.name();
                let (db, sync) = self.sync_pkg(name)?;
                let current = version_of(pkg);
                let new = version_of(sync);
                alpm::vercmp(new.as_str(), current.as_str())
                    .is_gt()
                    .then(|| Update {
                        name: name.to_string(),
                        current,
                        new,
                        source: Source::Repo(db.name().to_string()),
                    })
            })
            .collect();
        updates.sort_by(|a, b| a.name.cmp(&b.name));
        updates
    }

    fn sync_pkg(&self, name: &str) -> Option<(&alpm::Db, &alpm::Package)> {
        self.handle
            .syncdbs()
            .iter()
            .find_map(|db| db.pkg(name).ok().map(|p| (db, p)))
    }

    fn source_of(&self, name: &str) -> Source {
        self.repo_of(name).map_or(Source::Foreign, Source::Repo)
    }
}

fn open_handle(repo_list: fn() -> anyhow::Result<Vec<String>>) -> anyhow::Result<alpm::Alpm> {
    let handle = alpm::Alpm::new("/", "/var/lib/pacman")?;
    for repo in repo_list()? {
        handle.register_syncdb(repo, alpm::SigLevel::USE_DEFAULT)?;
    }
    Ok(handle)
}

fn configured_repos() -> anyhow::Result<Vec<String>> {
    let output = Command::new("pacman-conf")
        .arg("--repo-list")
        .output()
        .context("couldn't run pacman-conf")?;
    if !output.status.success() {
        bail!(
            "pacman-conf --repo-list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect())
}

fn version_of(p: &alpm::Package) -> String {
    p.version().as_str().to_string()
}

fn to_lower(s: &str) -> Cow<'_, str> {
    if s.is_ascii() && !s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(s.to_lowercase())
    }
}

fn contains_lc(text: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if text.is_ascii() {
        text.as_bytes()
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
    } else {
        text.to_lowercase().contains(needle)
    }
}

fn row(key: &str, value: String) -> (String, String) {
    (key.to_string(), value)
}

fn or_dash(value: Option<&str>) -> String {
    value.filter(|s| !s.is_empty()).unwrap_or(EMPTY).to_string()
}

fn join_list(items: impl Iterator<Item = String>, sep: &str) -> String {
    let items: Vec<String> = items.collect();
    if items.is_empty() {
        EMPTY.to_string()
    } else {
        items.join(sep)
    }
}

fn size_bytes(size: i64) -> u64 {
    u64::try_from(size).unwrap_or(0)
}

/// Readable size in B, KiB, MiB or GiB, with one decimal except in bytes.
pub(crate) fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = "KiB";
    for next in ["MiB", "GiB"] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

fn format_date(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .map(|d| d.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| EMPTY.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> RepoDb {
        RepoDb::new().expect("libalpm must open on this system")
    }

    fn contains_term(p: &Package, term: &str) -> bool {
        let t = term.to_lowercase();
        p.name.to_lowercase().contains(&t) || p.description.to_lowercase().contains(&t)
    }

    #[test]
    fn human_size_switches_units_at_powers_of_1024() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(1024 * 1024), "1.0 MiB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn contains_lc_ignores_case_in_ascii_and_unicode() {
        assert!(contains_lc("Pacman Tools", "tools"));
        assert!(contains_lc("Ñandú", "ñandú"));
        assert!(!contains_lc("pacman", "pac-x"));
        assert!(contains_lc("abc", ""));
    }

    #[test]
    fn join_list_uses_dash_when_empty() {
        assert_eq!(join_list(std::iter::empty(), "  "), EMPTY);
        let items = ["a", "b"].map(String::from);
        assert_eq!(join_list(items.into_iter(), "  "), "a  b");
    }

    #[test]
    fn installed_is_sorted_and_contains_pacman_from_core() {
        let installed = db().installed();
        assert!(installed.windows(2).all(|w| w[0].name <= w[1].name));
        let pacman = installed
            .iter()
            .find(|p| p.name == "pacman")
            .expect("pacman must be installed");
        assert_eq!(pacman.source, Source::Repo("core".to_string()));
        assert_eq!(pacman.installed.as_deref(), Some(pacman.version.as_str()));
    }

    #[test]
    fn search_puts_exact_name_first_and_every_hit_has_term() {
        let results = db().search("pacman");
        assert_eq!(results[0].name, "pacman");
        assert!(results.iter().all(|p| contains_term(p, "pacman")));
    }

    #[test]
    fn search_requires_every_term() {
        assert!(db().search("pacman facpm-does-not-exist-xyz").is_empty());
        assert!(db().search("   ").is_empty());
    }

    #[test]
    fn details_of_installed_package_show_install_rows() {
        let details = db().details("pacman").expect("pacman exists");
        let keys: Vec<&str> = details.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(details[0], ("Name".to_string(), "pacman".to_string()));
        for expected in ["Install date", "Install reason"] {
            assert!(keys.contains(&expected), "missing the {expected} row");
        }
        assert!(!keys.contains(&"Download size"));
    }

    #[test]
    fn details_of_missing_package_is_none() {
        assert!(db().details("package-that-does-not-exist-xyz").is_none());
    }

    #[test]
    fn pkgbase_and_repo_of_pacman() {
        let db = db();
        assert_eq!(db.pkgbase("pacman").as_deref(), Some("pacman"));
        assert_eq!(db.repo_of("pacman").as_deref(), Some("core"));
    }

    #[test]
    fn pkgbase_repo_and_version_of_missing_package_are_none() {
        let db = db();
        let missing = "package-that-does-not-exist-xyz";
        assert_eq!(db.pkgbase(missing), None);
        assert_eq!(db.repo_of(missing), None);
        assert_eq!(db.installed_version(missing), None);
    }

    #[test]
    fn local_updates_are_newer_and_sorted() {
        let updates = db().local_updates();
        assert!(updates.windows(2).all(|w| w[0].name <= w[1].name));
        for u in &updates {
            assert!(
                alpm::vercmp(u.new.as_str(), u.current.as_str()).is_gt(),
                "{} {} -> {} is not an update",
                u.name,
                u.current,
                u.new
            );
            assert!(matches!(u.source, Source::Repo(_)));
        }
    }
}

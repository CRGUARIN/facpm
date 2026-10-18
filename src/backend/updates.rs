use std::collections::HashMap;
use std::io;
use std::process::Command;

use anyhow::bail;

use crate::backend::aur::AurPkg;
use crate::pkg::{Package, Source};

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub name: String,
    pub current: String,
    pub new: String,
    /// `Source::Repo(repo)` or `Source::Aur`.
    pub source: Source,
}

/// (name, current version, new version), as checkupdates prints them.
pub type RawUpdate = (String, String, String);

/// Runs `checkupdates --nocolor`. It takes seconds, so it's called from a thread.
/// Exit code 2 means "no updates"; without the binary it returns `None`.
pub fn checkupdates() -> anyhow::Result<Option<Vec<RawUpdate>>> {
    run_checkupdates("checkupdates")
}

fn run_checkupdates(program: &str) -> anyhow::Result<Option<Vec<RawUpdate>>> {
    let output = match Command::new(program).arg("--nocolor").output() {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => bail!("couldn't run checkupdates: {err}"),
    };
    match output.status.code() {
        Some(0) => Ok(Some(parse_checkupdates(&String::from_utf8_lossy(
            &output.stdout,
        )))),
        Some(2) => Ok(Some(Vec::new())),
        code => {
            let code = code.map_or_else(|| "no exit code".to_string(), |c| c.to_string());
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            if stderr.is_empty() {
                bail!("checkupdates failed ({code})");
            }
            bail!("checkupdates failed ({code}): {stderr}");
        }
    }
}

/// Foreign packages with a newer version in the AUR (`alpm::vercmp`), by name; the rest are ignored.
pub fn aur_updates(foreign: &[Package], aur: &[AurPkg]) -> Vec<Update> {
    let aur_by_name: HashMap<&str, &AurPkg> =
        aur.iter().map(|pkg| (pkg.name.as_str(), pkg)).collect();
    let mut updates: Vec<Update> = foreign
        .iter()
        .filter(|pkg| pkg.source == Source::Foreign)
        .filter_map(|pkg| {
            let aur_pkg = aur_by_name.get(pkg.name.as_str())?;
            let current = pkg.installed.as_deref().unwrap_or(pkg.version.as_str());
            alpm::vercmp(aur_pkg.version.as_str(), current)
                .is_gt()
                .then(|| Update {
                    name: pkg.name.clone(),
                    current: current.to_string(),
                    new: aur_pkg.version.clone(),
                    source: Source::Aur,
                })
        })
        .collect();
    updates.sort_by(|a, b| a.name.cmp(&b.name));
    updates
}

fn parse_checkupdates(text: &str) -> Vec<RawUpdate> {
    text.lines()
        .filter_map(
            |line| match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                [name, current, "->", new] => {
                    Some((name.to_string(), current.to_string(), new.to_string()))
                }
                _ => None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "nodejs 26.11.1-1 -> 26.12.0-1\nollama 0.40.5-1 -> 0.40.6-1\n:: basura\n";

    fn aur_pkg(name: &str, version: &str) -> AurPkg {
        serde_json::from_value(serde_json::json!({
            "Name": name,
            "Version": version,
            "NumVotes": 0,
            "Popularity": 0.0,
            "FirstSubmitted": 0,
            "LastModified": 0,
            "PackageBase": name,
        }))
        .expect("valid test JSON")
    }

    fn foreign(name: &str, installed: &str) -> Package {
        Package {
            name: name.to_string(),
            version: installed.to_string(),
            description: String::new(),
            source: Source::Foreign,
            installed: Some(installed.to_string()),
        }
    }

    #[test]
    fn parse_keeps_only_arrow_lines() {
        let parsed = parse_checkupdates(SAMPLE);
        assert_eq!(
            parsed,
            vec![
                (
                    "nodejs".to_string(),
                    "26.11.1-1".to_string(),
                    "26.12.0-1".to_string()
                ),
                (
                    "ollama".to_string(),
                    "0.40.5-1".to_string(),
                    "0.40.6-1".to_string()
                ),
            ]
        );
        assert!(parse_checkupdates("\n\n").is_empty());
    }

    #[test]
    fn missing_binary_is_none() {
        let result = run_checkupdates("facpm-binario-inexistente").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn aur_update_when_pkgrel_is_newer() {
        let updates = aur_updates(&[foreign("yay", "1.0-1")], &[aur_pkg("yay", "1.0-2")]);
        assert_eq!(
            updates,
            vec![Update {
                name: "yay".to_string(),
                current: "1.0-1".to_string(),
                new: "1.0-2".to_string(),
                source: Source::Aur,
            }]
        );
    }

    #[test]
    fn aur_epoch_is_compared_first() {
        let none = aur_updates(&[foreign("tool", "2:1.0-1")], &[aur_pkg("tool", "1.5-1")]);
        assert!(none.is_empty());
        let some = aur_updates(&[foreign("tool", "1.5-1")], &[aur_pkg("tool", "2:1.0-1")]);
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].new, "2:1.0-1");
    }

    #[test]
    fn same_or_older_aur_version_is_not_update() {
        let updates = aur_updates(
            &[foreign("same", "1.0-1"), foreign("older", "2.0-1")],
            &[aur_pkg("same", "1.0-1"), aur_pkg("older", "1.9-1")],
        );
        assert!(updates.is_empty());
    }

    #[test]
    fn foreign_missing_from_aur_is_ignored_and_result_sorted() {
        let installed = [
            foreign("zeta", "1.0-1"),
            foreign("alpha", "1.0-1"),
            foreign("outside-the-aur", "1.0-1"),
        ];
        let aur = [aur_pkg("zeta", "1.1-1"), aur_pkg("alpha", "1.1-1")];
        let names: Vec<String> = aur_updates(&installed, &aur)
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(names, ["alpha", "zeta"]);
    }

    #[test]
    fn repo_packages_are_not_aur_updates() {
        let repo_pkg = Package {
            name: "yay".to_string(),
            version: "1.0-1".to_string(),
            description: String::new(),
            source: Source::Repo("extra".to_string()),
            installed: Some("1.0-1".to_string()),
        };
        assert!(aur_updates(&[repo_pkg], &[aur_pkg("yay", "1.0-2")]).is_empty());
    }

    #[test]
    #[ignore = "red"]
    fn checkupdates_online_runs() {
        checkupdates().expect("checkupdates must not fail");
    }
}

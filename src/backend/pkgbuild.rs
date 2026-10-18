use std::time::Duration;

use anyhow::{Context, bail};

/// Where the PKGBUILD is looked up: the AUR (cgit) or the official repos (Arch's GitLab).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PkgbuildSource {
    Aur,
    Repo,
}

const AUR_URL: &str = "https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD";
const GITLAB_PACKAGES: &str = "https://gitlab.archlinux.org/archlinux/packaging/packages";
const TIMEOUT: Duration = Duration::from_secs(10);

/// GitLab path of a pkgbase, following the `pkgctl` rule: `libsigc++` gives `libsigcplusplus`.
pub fn gitlab_project_path(pkgbase: &str) -> String {
    let joined = join_words_around_plus(pkgbase).replace('+', "plus");
    let clean: String = joined
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = collapse_separators(&clean);
    if path == "tree" {
        "unix-tree".to_string()
    } else {
        path
    }
}

/// PKGBUILD URL. On the AUR the pkgbase goes in the query; on GitLab it goes in the path.
pub fn url(source: PkgbuildSource, pkgbase: &str) -> String {
    match source {
        PkgbuildSource::Aur => format!("{AUR_URL}?h={}", percent_encode(pkgbase)),
        PkgbuildSource::Repo => format!(
            "{GITLAB_PACKAGES}/{}/-/raw/main/PKGBUILD",
            gitlab_project_path(pkgbase)
        ),
    }
}

/// Downloads the PKGBUILD. Any reply other than 200 `text/plain` counts as not found.
pub fn fetch(source: PkgbuildSource, pkgbase: &str) -> anyhow::Result<String> {
    // Don't follow redirects: GitLab answers 302 to the login page when the project doesn't exist.
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(TIMEOUT)
        .user_agent(concat!("facpm/", env!("CARGO_PKG_VERSION")))
        .build();
    let request = match source {
        PkgbuildSource::Aur => agent.get(AUR_URL).query("h", pkgbase),
        PkgbuildSource::Repo => agent.get(&url(source, pkgbase)),
    };
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(_, _)) => bail!("PKGBUILD not found"),
        Err(err) => bail!("couldn't download the PKGBUILD: {err}"),
    };
    let is_text = response
        .header("Content-Type")
        .is_some_and(|kind| kind.starts_with("text/plain"));
    if response.status() != 200 || !is_text {
        bail!("PKGBUILD not found");
    }
    response
        .into_string()
        .context("couldn't read the downloaded PKGBUILD")
}

fn join_words_around_plus(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match plus_match(&chars, i) {
            Some((plus, end)) => {
                out.extend(&chars[i..plus]);
                out.push('-');
                out.extend(&chars[plus + 1..end]);
                i = end;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn plus_match(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let plus = start
        + chars[start..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
    if plus == start || chars.get(plus) != Some(&'+') {
        return None;
    }
    let letters = plus + 1;
    let end = letters
        + chars[letters..]
            .iter()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
    (end > letters).then_some((plus, end))
}

fn collapse_separators(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !matches!(c, '_' | '-') {
            out.push(c);
            continue;
        }
        let mut run = 1;
        while chars.next_if(|next| matches!(next, '_' | '-')).is_some() {
            run += 1;
        }
        out.push(if run > 1 { '-' } else { c });
    }
    out
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitlab_project_path_follows_pkgctl_rules() {
        let cases = [
            ("libsigc++", "libsigcplusplus"),
            ("dvd+rw-tools", "dvd-rw-tools"),
            ("tree", "unix-tree"),
            ("pacman", "pacman"),
            ("gtk2+extra", "gtk2-extra"),
            ("mysql++", "mysqlplusplus"),
            ("a+b+c", "a-bplusc"),
            ("foo__bar", "foo-bar"),
            ("foo@bar", "foo-bar"),
        ];
        for (pkgbase, expected) in cases {
            assert_eq!(gitlab_project_path(pkgbase), expected, "pkgbase {pkgbase}");
        }
    }

    #[test]
    fn plus_joins_only_letters_after_an_alphanumeric_run() {
        assert_eq!(gitlab_project_path("a+1"), "aplus1");
        assert_eq!(gitlab_project_path("1+2a+b"), "1plus2a-b");
        assert_eq!(gitlab_project_path("gtk2+extra2"), "gtk2-extra2");
        assert_eq!(gitlab_project_path("a+bc+d"), "a-bcplusd");
    }

    #[test]
    fn single_separators_are_kept() {
        assert_eq!(gitlab_project_path("foo_bar"), "foo_bar");
        assert_eq!(gitlab_project_path("foo-_bar"), "foo-bar");
        assert_eq!(gitlab_project_path("foo.bar"), "foo.bar");
    }

    #[test]
    fn url_encodes_the_aur_query_and_uses_the_gitlab_path() {
        assert_eq!(
            url(PkgbuildSource::Aur, "yay"),
            "https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h=yay"
        );
        assert_eq!(
            url(PkgbuildSource::Aur, "libsigc++"),
            "https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h=libsigc%2B%2B"
        );
        assert_eq!(
            url(PkgbuildSource::Repo, "libsigc++"),
            "https://gitlab.archlinux.org/archlinux/packaging/packages/libsigcplusplus/-/raw/main/PKGBUILD"
        );
    }

    #[test]
    #[ignore = "red"]
    fn fetch_existing_packages_online() {
        let aur = fetch(PkgbuildSource::Aur, "yay").expect("yay is on the AUR");
        assert!(aur.contains("pkgname=yay"));
        let repo = fetch(PkgbuildSource::Repo, "pacman").expect("pacman is on GitLab");
        assert!(repo.contains("pkgname=pacman"));
    }

    #[test]
    #[ignore = "red"]
    fn fetch_missing_packages_is_err() {
        let missing = "package-that-does-not-exist-xyz";
        assert!(fetch(PkgbuildSource::Repo, missing).is_err());
        assert!(fetch(PkgbuildSource::Aur, missing).is_err());
    }
}

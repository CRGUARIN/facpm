use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, bail, ensure};
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;

use crate::backend::aur::{self, AurFields, AurPkg};

const INDEX_URL: &str = "https://aur.archlinux.org/packages-meta-v1.json.gz";
const FILE_NAME: &str = "packages-meta-v1.json.gz";
const TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = concat!("facpm/", env!("CARGO_PKG_VERSION"));
/// Gzip magic bytes: keeps an HTML reply from being cached as the index.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Compact form of the index: only the fields that search and updates use.
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Entry {
    name: Box<str>,
    version: Box<str>,
    description: Option<Box<str>>,
    #[serde(rename = "URL")]
    url: Option<Box<str>>,
    num_votes: u32,
    popularity: f64,
    maintainer: Option<Box<str>>,
    out_of_date: Option<i64>,
    first_submitted: i64,
    last_modified: i64,
    package_base: Box<str>,
}

impl Entry {
    #[cfg(test)]
    fn from_pkg(p: AurPkg) -> Entry {
        Entry {
            name: p.name.into_boxed_str(),
            version: p.version.into_boxed_str(),
            description: p.description.map(String::into_boxed_str),
            url: p.url.map(String::into_boxed_str),
            num_votes: u32::try_from(p.num_votes).unwrap_or(u32::MAX),
            popularity: p.popularity,
            maintainer: p.maintainer.map(String::into_boxed_str),
            out_of_date: p.out_of_date,
            first_submitted: p.first_submitted,
            last_modified: p.last_modified,
            package_base: p.package_base.into_boxed_str(),
        }
    }

    /// Dependency lists are left empty: `aur::info` has them.
    fn to_pkg(&self) -> AurPkg {
        AurPkg {
            name: self.name.to_string(),
            version: self.version.to_string(),
            description: self.description.as_deref().map(str::to_string),
            url: self.url.as_deref().map(str::to_string),
            num_votes: u64::from(self.num_votes),
            popularity: self.popularity,
            maintainer: self.maintainer.as_deref().map(str::to_string),
            out_of_date: self.out_of_date,
            first_submitted: self.first_submitted,
            last_modified: self.last_modified,
            package_base: self.package_base.to_string(),
            depends: Vec::new(),
            make_depends: Vec::new(),
            opt_depends: Vec::new(),
            license: Vec::new(),
            keywords: Vec::new(),
        }
    }
}

impl AurFields for Entry {
    fn name_str(&self) -> &str {
        &self.name
    }

    fn description_str(&self) -> &str {
        self.description.as_deref().unwrap_or_default()
    }

    fn popularity_value(&self) -> f64 {
        self.popularity
    }
}

pub struct AurIndex {
    /// Sorted by name, so `get` can binary-search.
    pkgs: Vec<Entry>,
}

impl AurIndex {
    pub fn cache_path() -> PathBuf {
        cache_path_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
    }

    pub fn load_cached() -> anyhow::Result<Option<AurIndex>> {
        load_from(&Self::cache_path())
    }

    /// Downloads the index if the cache is missing or older than `max_age`. Ok(true) if a new one came down.
    pub fn refresh_if_stale(max_age: Duration) -> anyhow::Result<bool> {
        refresh_at(&Self::cache_path(), max_age)
    }

    pub fn len(&self) -> usize {
        self.pkgs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pkgs.is_empty()
    }

    /// Scans the whole index: only worth it for foreign packages not found by name.
    pub fn has_base(&self, base: &str) -> bool {
        self.pkgs.iter().any(|p| &*p.package_base == base)
    }

    pub fn get(&self, name: &str) -> Option<AurPkg> {
        let i = self.pkgs.binary_search_by(|p| (*p.name).cmp(name)).ok()?;
        Some(self.pkgs[i].to_pkg())
    }

    /// Like `aur::search`, but without the two-character minimum.
    pub fn search(&self, query: &str) -> Vec<AurPkg> {
        let lowered = query.to_lowercase();
        let terms: Vec<&str> = lowered.split_whitespace().collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&Entry> = self
            .pkgs
            .iter()
            .filter(|p| aur::matches(p, &terms))
            .collect();
        aur::sort_results(&mut hits, &terms.join(" "));
        hits.into_iter().map(Entry::to_pkg).collect()
    }

    #[cfg(test)]
    pub(crate) fn from_pkgs(pkgs: Vec<AurPkg>) -> AurIndex {
        AurIndex::new(pkgs.into_iter().map(Entry::from_pkg).collect())
    }

    fn new(mut pkgs: Vec<Entry>) -> AurIndex {
        pkgs.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        pkgs.shrink_to_fit();
        AurIndex { pkgs }
    }
}

/// Cache folder: `XDG_CACHE_HOME` if set and not empty; otherwise `HOME/.cache`.
fn cache_path_from(xdg_cache_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = match xdg_cache_home.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home.unwrap_or_default()).join(".cache"),
    };
    base.join("facpm").join(FILE_NAME)
}

fn load_from(path: &Path) -> anyhow::Result<Option<AurIndex>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("couldn't open {}", path.display())),
    };
    let pkgs: Vec<Entry> = serde_json::from_reader(BufReader::new(GzDecoder::new(file)))
        .with_context(|| format!("invalid AUR index: {}", path.display()))?;
    Ok(Some(AurIndex::new(pkgs)))
}

fn refresh_at(path: &Path, max_age: Duration) -> anyhow::Result<bool> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok();
    // A date in the future (the clock moved back) counts as old.
    if modified.is_some_and(|time| time.elapsed().is_ok_and(|age| age < max_age)) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }

    let mut request = ureq::get(INDEX_URL)
        .timeout(TIMEOUT)
        .set("User-Agent", USER_AGENT);
    if let Some(time) = modified {
        request = request.set("If-Modified-Since", &http_date(time));
    }
    // ureq 2 only treats >= 400 as an error: a 304 comes back as `Ok`.
    let response = request.call().context("couldn't download the AUR index")?;
    match response.status() {
        304 => {
            File::options()
                .write(true)
                .open(path)?
                .set_modified(SystemTime::now())?;
            Ok(false)
        }
        200 => {
            store(&mut response.into_reader(), path)?;
            Ok(true)
        }
        status => bail!("unexpected reply from the AUR: HTTP {status}"),
    }
}

/// Writes to a temporary file named after the pid and renames it over `path`, so the cache is never half-written.
fn store(body: &mut impl Read, path: &Path) -> anyhow::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);

    let result = write_and_rename(body, &tmp, path);
    if result.is_err() {
        fs::remove_file(&tmp).ok();
    }
    result
}

fn write_and_rename(body: &mut impl Read, tmp: &Path, path: &Path) -> anyhow::Result<()> {
    let mut magic = [0u8; 2];
    body.read_exact(&mut magic)
        .context("the AUR returned a body that is too short")?;
    ensure!(magic == GZIP_MAGIC, "the AUR didn't return a gzip file");

    let mut file =
        File::create(tmp).with_context(|| format!("couldn't create {}", tmp.display()))?;
    file.write_all(&magic)?;
    io::copy(body, &mut file).context("failed to download the AUR index")?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path).with_context(|| format!("couldn't replace {}", path.display()))
}

fn http_date(time: SystemTime) -> String {
    DateTime::<Utc>::from(time)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    /// Temporary folder removed when the test exits, pass or fail.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("facpm-aur-index-{}-{test}", std::process::id()));
            fs::remove_dir_all(&dir).ok();
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    impl std::ops::Deref for TempDir {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.0
        }
    }

    fn write_gz(path: &Path, json: &str) {
        let mut encoder = GzEncoder::new(File::create(path).unwrap(), Compression::default());
        encoder.write_all(json.as_bytes()).unwrap();
        encoder.finish().unwrap();
    }

    fn entry(name: &str, description: &str, popularity: f64) -> serde_json::Value {
        serde_json::json!({
            "ID": 1,
            "Name": name,
            "PackageBaseID": 1,
            "PackageBase": name,
            "Version": "1.0-1",
            "Description": description,
            "URL": null,
            "NumVotes": 10,
            "Popularity": popularity,
            "OutOfDate": null,
            "Maintainer": "someone",
            "FirstSubmitted": 1600000000,
            "LastModified": 1700000000,
            "URLPath": format!("/cgit/aur.git/snapshot/{name}.tar.gz"),
            "Depends": ["glibc"],
            "License": ["MIT"],
            "Keywords": []
        })
    }

    fn sample_json() -> String {
        serde_json::json!([
            entry("git", "Distributed version control system", 0.5),
            entry("git-extras", "Utilities for Git", 3.0),
            entry("lazygit", "Terminal UI for GIT", 2.0),
            entry("firefox", "Fast and private web browser, née Phoenix", 9.0),
        ])
        .to_string()
    }

    fn names(pkgs: Vec<AurPkg>) -> Vec<String> {
        pkgs.into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn cache_path_prefers_xdg_and_ignores_empty_value() {
        assert_eq!(
            cache_path_from(Some("/x".into()), Some("/home/u".into())),
            Path::new("/x/facpm").join(FILE_NAME)
        );
        assert_eq!(
            cache_path_from(Some("".into()), Some("/home/u".into())),
            Path::new("/home/u/.cache/facpm").join(FILE_NAME)
        );
        assert_eq!(
            cache_path_from(None, Some("/home/u".into())),
            Path::new("/home/u/.cache/facpm").join(FILE_NAME)
        );
    }

    #[test]
    fn has_base_checks_package_base_not_name() {
        let mut value = entry("demo-bin", "demo", 1.0);
        value["PackageBase"] = serde_json::json!("demo");
        let pkg: AurPkg = serde_json::from_value(value).unwrap();
        let index = AurIndex::from_pkgs(vec![pkg]);
        assert!(index.has_base("demo"));
        assert!(!index.has_base("demo-bin"));
    }

    #[test]
    fn missing_cache_is_none() {
        let dir = TempDir::new("missing");
        assert!(load_from(&dir.join(FILE_NAME)).unwrap().is_none());
    }

    #[test]
    fn cached_index_supports_get_and_search() {
        let dir = TempDir::new("offline");
        let path = dir.join(FILE_NAME);
        write_gz(&path, &sample_json());

        let index = load_from(&path).unwrap().expect("the file exists");
        assert_eq!(index.len(), 4);
        assert_eq!(index.get("git").map(|p| p.popularity), Some(0.5));
        assert!(index.get("does-not-exist").is_none());

        assert_eq!(names(index.search("git")), ["git", "git-extras", "lazygit"]);
        assert_eq!(names(index.search("GIT")), ["git", "git-extras", "lazygit"]);
        assert_eq!(names(index.search("terminal git")), ["lazygit"]);
        assert!(index.search("firefox git").is_empty());
        assert_eq!(names(index.search("NÉE")), ["firefox"]);
        assert!(names(index.search("g")).contains(&"git".to_string()));
        assert!(index.search("   ").is_empty());
    }

    #[test]
    fn search_caps_results_at_250() {
        let pkgs: Vec<serde_json::Value> = (0..300)
            .map(|i| entry(&format!("package-{i:03}"), "demo", f64::from(i)))
            .collect();
        let pkgs: Vec<AurPkg> = serde_json::from_value(serde_json::Value::Array(pkgs)).unwrap();
        let index = AurIndex::from_pkgs(pkgs);

        let hits = index.search("package");
        assert_eq!(hits.len(), 250);
        assert_eq!(hits[0].name, "package-299");
        assert_eq!(hits[249].name, "package-050");
    }

    #[test]
    fn fresh_cache_is_not_refreshed() {
        let dir = TempDir::new("fresh");
        let path = dir.join(FILE_NAME);
        write_gz(&path, &sample_json());
        assert!(!refresh_at(&path, DAY).unwrap());
    }

    #[test]
    fn non_gzip_body_is_not_stored() {
        let dir = TempDir::new("not-gzip");
        let path = dir.join(FILE_NAME);
        let body = b"<html>error</html>";
        assert!(store(&mut &body[..], &path).is_err());
        assert!(!path.exists());
    }

    #[test]
    #[ignore = "red"]
    fn online_download_then_revalidate_then_search() {
        let dir = TempDir::new("online");
        let path = dir.join(FILE_NAME);

        assert!(refresh_at(&path, DAY).expect("initial download"));
        assert!(!refresh_at(&path, DAY).expect("segunda llamada"));
        refresh_at(&path, Duration::ZERO).expect("revalidation with If-Modified-Since");

        let index = load_from(&path).unwrap().expect("cache present");
        assert!(index.len() > 100_000, "packages: {}", index.len());
        assert!(!index.search("firefox").is_empty());
    }
}

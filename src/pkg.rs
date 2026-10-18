#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Repo(String),
    Aur,
    /// Installed and missing from the synced repos: it comes from the AUR or was installed by hand.
    Foreign,
}

impl Source {
    /// Repo name, "aur" or "local".
    pub fn label(&self) -> &str {
        match self {
            Source::Repo(repo) => repo,
            Source::Aur => "aur",
            Source::Foreign => "local",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    /// Version from the repo or the AUR; for `Foreign`, the installed one.
    pub version: String,
    pub description: String,
    pub source: Source,
    pub installed: Option<String>,
}

pub type Details = Vec<(String, String)>;

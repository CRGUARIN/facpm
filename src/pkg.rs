#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Repo(String),
    Aur,
    /// Instalado y ausente de los repos sincronizados: viene del AUR o se instaló a mano.
    Foreign,
}

impl Source {
    /// Nombre del repo, "aur" o "local".
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
    /// Versión del repo o del AUR; en `Foreign`, la instalada.
    pub version: String,
    pub description: String,
    pub source: Source,
    pub installed: Option<String>,
}

pub type Details = Vec<(String, String)>;

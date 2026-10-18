use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::theme;

const FILE_NAME: &str = "config.toml";

/// Ruta que no se puede escribir: sin HOME ni XDG_CONFIG_HOME absolutos no hay dónde guardar.
const NOWHERE: &str = "/nonexistent/facpm/config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `id` de un tema de `theme::THEMES`.
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: theme::default_theme().id.to_string(),
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/facpm/config.toml` o `~/.config/facpm/config.toml`.
    pub fn path() -> PathBuf {
        path_from(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )
    }

    /// Si el archivo falta o no se entiende, usa el valor por defecto. No devuelve error.
    pub fn load() -> Config {
        load_from(&Config::path())
    }

    pub fn save(&self) -> Result<()> {
        let path = Config::path();
        if path == Path::new(NOWHERE) {
            bail!("no hay HOME ni XDG_CONFIG_HOME absolutos: no se puede guardar la configuración");
        }
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .context("la ruta de la configuración no tiene carpeta")?;
        fs::create_dir_all(dir).with_context(|| format!("no se pudo crear {}", dir.display()))?;
        // Temporal en la misma carpeta y rename: un corte a mitad no deja el archivo a medias.
        let tmp = tmp_path(path);
        let written = write_then_rename(&tmp, path, &render(self));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written.with_context(|| format!("no se pudo guardar {}", path.display()))
    }
}

fn path_from(xdg: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = absolute(xdg).or_else(|| absolute(home).map(|home| home.join(".config")));
    match base {
        Some(base) => base.join("facpm").join(FILE_NAME),
        None => PathBuf::from(NOWHERE),
    }
}

fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

fn load_from(path: &Path) -> Config {
    fs::read_to_string(path)
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Un tema desconocido cae en "terminal". Las líneas raras y las claves ajenas se ignoran.
fn parse(text: &str) -> Config {
    let mut config = Config::default();
    for line in text.lines() {
        let Some((key, value)) = strip_comment(line).trim().split_once('=') else {
            continue;
        };
        if key.trim() != "theme" {
            continue;
        }
        let Some(name) = unquote(value.trim()) else {
            continue;
        };
        config.theme = theme::by_id(name)
            .unwrap_or_else(theme::default_theme)
            .id
            .to_string();
    }
    config
}

fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(value: &str) -> Option<&str> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then_some(inner)
}

fn render(config: &Config) -> String {
    format!("theme = \"{}\"\n", config.theme)
}

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

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("facpm-config-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    fn config(theme: &str) -> Config {
        Config {
            theme: theme.to_string(),
        }
    }

    #[test]
    fn path_prefers_absolute_xdg_then_home() {
        assert_eq!(
            path_from(os("/x/cfg"), os("/home/ejemplo")),
            PathBuf::from("/x/cfg/facpm/config.toml")
        );
        assert_eq!(
            path_from(None, os("/home/ejemplo")),
            PathBuf::from("/home/ejemplo/.config/facpm/config.toml")
        );
        assert_eq!(
            path_from(os(""), os("/home/ejemplo")),
            PathBuf::from("/home/ejemplo/.config/facpm/config.toml")
        );
        assert_eq!(
            path_from(os("relativo"), os("/home/ejemplo")),
            PathBuf::from("/home/ejemplo/.config/facpm/config.toml")
        );
    }

    #[test]
    fn path_without_absolute_dirs_is_nowhere() {
        assert_eq!(path_from(None, None), PathBuf::from(NOWHERE));
        assert_eq!(path_from(os(""), os("relativo")), PathBuf::from(NOWHERE));
    }

    #[test]
    fn parse_accepts_spaces_and_comments() {
        let text = "  # comentario\n\n   theme   =   \"nord\"   # otro comentario\n";
        assert_eq!(parse(text), config("nord"));
        assert_eq!(parse("theme=\"dracula\""), config("dracula"));
        assert_eq!(parse(""), Config::default());
        assert_eq!(Config::default().theme, "terminal");
    }

    #[test]
    fn parse_skips_other_keys_and_malformed_lines() {
        let text = "idioma = \"es\"\nlinea rota\ntheme = nord\ntheme = \"nord\" extra\n";
        assert_eq!(parse(text), Config::default());
        assert_eq!(
            parse("color = \"red\"\ntheme = \"gruvbox\"\n"),
            config("gruvbox")
        );
    }

    #[test]
    fn unknown_theme_falls_back_to_terminal() {
        assert_eq!(parse("theme = \"nope\""), config("terminal"));
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        assert_eq!(strip_comment(r#"theme = "a#b" # c"#), r#"theme = "a#b" "#);
    }

    #[test]
    fn render_then_parse_gives_the_same_config() {
        assert_eq!(render(&config("nord")), "theme = \"nord\"\n");
        for theme in theme::THEMES {
            let original = config(theme.id);
            assert_eq!(parse(&render(&original)), original);
        }
    }

    #[test]
    fn save_creates_folders_and_load_reads_it_back() {
        let tmp = TempDir::new("save");
        let path = tmp.path().join("anidada").join("facpm").join(FILE_NAME);

        config("nord").save_to(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme = \"nord\"\n");
        assert_eq!(load_from(&path), config("nord"));
        assert!(!tmp_path(&path).exists());
    }

    #[test]
    fn load_missing_or_unreadable_file_is_default() {
        let tmp = TempDir::new("load");
        assert_eq!(
            load_from(&tmp.path().join("missing.toml")),
            Config::default()
        );

        let bad = tmp.path().join("bad.toml");
        fs::write(&bad, [0xff_u8, 0xfe, 0x00]).unwrap();
        assert_eq!(load_from(&bad), Config::default());
    }
}

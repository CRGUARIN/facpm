use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::bail;

use crate::backend::aur_index::AurIndex;
use crate::backend::helper::Helper;

/// En el orden de la lista. `PkgCacheAll` va al final por ser la más destructiva.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanKind {
    Orphans,
    PkgCacheKeepLatest,
    PkgCacheUninstalled,
    AurBuildCache,
    FacpmCache,
    PkgCacheAll,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CleanTask {
    pub kind: CleanKind,
    pub title: String,
    pub description: String,
    /// Nombres, rutas absolutas o la ruta del índice. Vacío en paccache y en `-Scc`.
    pub items: Vec<String>,
    pub count: usize,
    /// Espacio a liberar; None = desconocido.
    pub bytes: Option<u64>,
    /// Aviso o motivo de indisponibilidad.
    pub note: Option<String>,
    /// Falso si no se puede ejecutar: sin paccache, sin helper AUR o sin candidatos.
    pub available: bool,
}

pub fn pkg_cache_dirs() -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = Command::new("pacman-conf")
        .arg("CacheDir")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>()
        })
        .unwrap_or_default();
    if dirs.is_empty() {
        vec![PathBuf::from("/var/cache/pacman/pkg/")]
    } else {
        dirs
    }
}

/// Tamaño recursivo en bytes, sin seguir enlaces.
pub fn dir_size(path: &Path) -> u64 {
    tree_stats(path).0
}

/// Un enlace cuenta por su propio tamaño y no se recorre.
fn tree_stats(path: &Path) -> (u64, usize) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        return (meta.len(), 1);
    }
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    entries
        .flatten()
        .map(|entry| tree_stats(&entry.path()))
        .fold((0, 0), |(bytes, files), (b, f)| (bytes + b, files + f))
}

/// `paccache -d <args>` con LC_ALL=C. Ok(None) si paccache no está instalado.
pub fn paccache_estimate(args: &[&str]) -> anyhow::Result<Option<(usize, u64)>> {
    run_paccache("paccache", args)
}

fn run_paccache(program: &str, args: &[&str]) -> anyhow::Result<Option<(usize, u64)>> {
    let output = match Command::new(program)
        .arg("-d")
        .args(args)
        .env("LC_ALL", "C")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => bail!("no se pudo ejecutar paccache: {err}"),
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let Some(estimate) = parse_paccache_output(&text) else {
        let last = text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("(sin salida)");
        bail!("salida de paccache no reconocida: {last}");
    };
    Ok(Some(estimate))
}

fn parse_paccache_output(text: &str) -> Option<(usize, u64)> {
    text.lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find_map(parse_paccache_line)
}

fn parse_paccache_line(line: &str) -> Option<(usize, u64)> {
    if line == "==> no candidate packages found for pruning" {
        return Some((0, 0));
    }
    let rest = line.strip_prefix("==> finished dry run: ")?;
    let (count, rest) = rest.split_once(' ')?;
    let count: usize = count.parse().ok()?;
    let rest = rest
        .strip_prefix("candidates")
        .or_else(|| rest.strip_prefix("candidate"))?;
    let size = rest
        .trim_start()
        .strip_prefix("(disk space saved: ")?
        .strip_suffix(')')?;
    Some((count, parse_size(size)?))
}

fn parse_size(text: &str) -> Option<u64> {
    let (value, unit) = text.split_once(' ')?;
    let factor: f64 = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0_f64.powi(2),
        "GiB" => 1024.0_f64.powi(3),
        "TiB" => 1024.0_f64.powi(4),
        _ => return None,
    };
    let value: f64 = value.parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some((value * factor) as u64)
}

/// Directorio de compilación del helper, bajo `XDG_CACHE_HOME` o `~/.cache`. Pacman no tiene.
pub fn aur_cache_dir(helper: Helper) -> Option<PathBuf> {
    aur_cache_dir_from(
        helper,
        std::env::var_os("XDG_CACHE_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Las rutas relativas se ignoran: `rm -rf` no debe resolverlas contra el directorio actual.
fn aur_cache_dir_from(
    helper: Helper,
    xdg_cache_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let subdir = match helper {
        Helper::Yay => PathBuf::from("yay"),
        Helper::Paru => Path::new("paru").join("clone"),
        Helper::Pikaur => Path::new("pikaur").join("build"),
        Helper::Pacman => return None,
    };
    let base =
        absolute_dir(xdg_cache_home).or_else(|| absolute_dir(home).map(|h| h.join(".cache")))?;
    Some(base.join(subdir))
}

fn absolute_dir(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Las seis tareas, en el orden del enum. Puede tardar (paccache y tamaños de directorios):
/// conviene llamarla desde un hilo. Los errores van a la `note` de cada tarea.
pub fn scan(orphans: Vec<(String, u64)>, helper: Helper) -> Vec<CleanTask> {
    vec![
        orphans_task(orphans),
        paccache_task(
            CleanKind::PkgCacheKeepLatest,
            "Caché pacman: versiones viejas",
            "Borra de la caché todas las versiones excepto la última de cada paquete. Sin versiones anteriores no podrás volver atrás sin conexión.",
            &["-k1"],
        ),
        paccache_task(
            CleanKind::PkgCacheUninstalled,
            "Caché pacman: desinstalados",
            "Borra de la caché los paquetes que ya no están instalados.",
            &["-uk0"],
        ),
        aur_build_cache_task(helper, aur_cache_dir(helper)),
        facpm_cache_task(&AurIndex::cache_path()),
        pkg_cache_all_task(&pkg_cache_dirs()),
    ]
}

/// Comando de la tarea, o None si no está disponible. No lo ejecuta.
pub fn command(task: &CleanTask, helper: Helper) -> Option<Command> {
    if !task.available {
        return None;
    }
    Some(match task.kind {
        CleanKind::Orphans => helper.remove_command(&task.items),
        CleanKind::PkgCacheKeepLatest => sudo(&["paccache", "-rk1"]),
        CleanKind::PkgCacheUninstalled => sudo(&["paccache", "-ruk0"]),
        CleanKind::AurBuildCache => rm(&["-rf", "--"], &task.items),
        CleanKind::FacpmCache => rm(&["-f", "--"], &task.items),
        CleanKind::PkgCacheAll => sudo(&["pacman", "-Scc"]),
    })
}

fn sudo(args: &[&str]) -> Command {
    let mut cmd = Command::new("sudo");
    cmd.args(args);
    cmd
}

fn rm(flags: &[&str], items: &[String]) -> Command {
    let mut cmd = Command::new("rm");
    cmd.args(flags).args(items);
    cmd
}

fn new_task(kind: CleanKind, title: &str, description: impl Into<String>) -> CleanTask {
    CleanTask {
        kind,
        title: title.to_string(),
        description: description.into(),
        items: Vec::new(),
        count: 0,
        bytes: None,
        note: None,
        available: false,
    }
}

fn orphans_task(orphans: Vec<(String, u64)>) -> CleanTask {
    let mut task = new_task(
        CleanKind::Orphans,
        "Paquetes huérfanos",
        "Paquetes instalados como dependencia que ya ningún paquete necesita. Revisa la lista: algunos (p. ej. compiladores) pueden ser herramientas que usas directamente.",
    );
    task.count = orphans.len();
    task.bytes = Some(orphans.iter().map(|(_, size)| size).sum::<u64>());
    task.items = orphans.into_iter().map(|(name, _)| name).collect();
    task.available = task.count > 0;
    task
}

fn paccache_task(kind: CleanKind, title: &str, description: &str, args: &[&str]) -> CleanTask {
    let mut task = new_task(kind, title, description);
    match paccache_estimate(args) {
        Ok(Some((count, bytes))) => {
            task.count = count;
            task.bytes = Some(bytes);
        }
        Ok(None) => task.note = Some("requiere pacman-contrib (paccache)".to_string()),
        Err(err) => task.note = Some(format!("no se pudo estimar con paccache: {err:#}")),
    }
    task.available = task.count > 0;
    task
}

fn aur_build_cache_task(helper: Helper, dir: Option<PathBuf>) -> CleanTask {
    let title = format!("Caché de compilación AUR ({})", helper.name());
    let location = dir
        .as_ref()
        .map_or(String::new(), |dir| format!(" en {}", dir.display()));
    let description = format!(
        "Borra los directorios de compilación del AUR{location}. Se vuelven a descargar al reinstalar o actualizar."
    );
    let mut task = new_task(CleanKind::AurBuildCache, &title, description);
    let Some(dir) = dir else {
        let note = if helper.supports_aur() {
            "no se encontró el directorio de caché"
        } else {
            "no hay AUR helper"
        };
        task.note = Some(note.to_string());
        return task;
    };
    match fs::read_dir(&dir) {
        Ok(entries) => {
            // Solo directorios: `file_type` no sigue enlaces, y los archivos sueltos (vcs.json,
            // completion.cache) quedan fuera. Las rutas no UTF-8 también: el comando debe nombrar
            // exactamente lo que se cuenta.
            let mut items: Vec<String> = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .filter_map(|entry| entry.path().into_os_string().into_string().ok())
                .collect();
            items.sort();
            task.bytes = Some(items.iter().map(|item| dir_size(Path::new(item))).sum());
            task.count = items.len();
            task.items = items;
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => task.bytes = Some(0),
        Err(err) => task.note = Some(format!("no se pudo leer {}: {err}", dir.display())),
    }
    task.available = task.count > 0;
    task
}

fn facpm_cache_task(path: &Path) -> CleanTask {
    let mut task = new_task(
        CleanKind::FacpmCache,
        "Caché de facpm (índice AUR)",
        "Borra el índice local del AUR. Se vuelve a descargar la próxima vez que inicies facpm.",
    );
    task.bytes = Some(0);
    let meta = fs::symlink_metadata(path)
        .ok()
        .filter(|meta| !meta.is_dir());
    if let (Some(meta), Some(item)) = (meta, path.to_str()) {
        task.items = vec![item.to_string()];
        task.count = 1;
        task.bytes = Some(meta.len());
    }
    task.available = task.count > 0;
    task
}

fn pkg_cache_all_task(dirs: &[PathBuf]) -> CleanTask {
    let mut task = new_task(
        CleanKind::PkgCacheAll,
        "Caché pacman completa",
        "Vacía por completo la caché de pacman (pacman -Scc). No podrás reinstalar ni volver a versiones anteriores sin conexión.",
    );
    let (bytes, files) = dirs
        .iter()
        .map(|dir| tree_stats(dir))
        .fold((0u64, 0usize), |(b, f), (db, df)| (b + db, f + df));
    task.bytes = Some(bytes);
    task.count = files;
    task.available = task.count > 0;
    task
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::helper::command_line;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("facpm-clean-{}-{test}", std::process::id()));
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

    fn task(kind: CleanKind, items: &[&str], available: bool) -> CleanTask {
        CleanTask {
            kind,
            title: String::new(),
            description: String::new(),
            items: items.iter().map(|s| s.to_string()).collect(),
            count: items.len(),
            bytes: None,
            note: None,
            available,
        }
    }

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    #[test]
    fn paccache_summary_lines_are_parsed() {
        assert_eq!(
            parse_paccache_output(
                "\n==> finished dry run: 51 candidates (disk space saved: 4.59 GiB)\n"
            ),
            Some((51, (4.59 * 1024f64.powi(3)) as u64))
        );
        assert_eq!(
            parse_paccache_output("==> no candidate packages found for pruning\n"),
            Some((0, 0))
        );
        // Singular, y la última línea reconocible gana aunque haya ruido después.
        assert_eq!(
            parse_paccache_output(
                "==> finished dry run: 1 candidate (disk space saved: 512 B)\nruido\n"
            ),
            Some((1, 512))
        );
        assert_eq!(parse_paccache_output("error: otra cosa\n"), None);
        assert_eq!(parse_paccache_output(""), None);
    }

    #[test]
    fn size_units_are_powers_of_1024() {
        assert_eq!(parse_size("1 B"), Some(1));
        assert_eq!(parse_size("1 KiB"), Some(1024));
        assert_eq!(
            parse_size("258.46 MiB"),
            Some((258.46 * 1024f64 * 1024f64) as u64)
        );
        assert_eq!(parse_size("2 TiB"), Some(2 * 1024u64.pow(4)));
        assert_eq!(parse_size("1 PiB"), None);
        assert_eq!(parse_size("-1 B"), None);
        assert_eq!(parse_size("NaN GiB"), None);
        assert_eq!(parse_size("4.59GiB"), None);
    }

    #[test]
    fn missing_paccache_binary_is_none() {
        let estimate = run_paccache("facpm-paccache-inexistente", &["-k1"]).unwrap();
        assert!(estimate.is_none());
    }

    #[test]
    fn dir_size_sums_files_and_subdirs() {
        let tmp = TempDir::new("dir-size");
        let root = tmp.path();
        fs::write(root.join("a"), vec![0u8; 100]).unwrap();
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(root.join("sub/b"), vec![0u8; 250]).unwrap();
        fs::write(root.join("sub/deep/c"), vec![0u8; 1]).unwrap();
        fs::create_dir(root.join("vacio")).unwrap();
        assert_eq!(dir_size(root), 351);
        assert_eq!(dir_size(&root.join("sub")), 251);
        assert_eq!(dir_size(&root.join("vacio")), 0);
        assert_eq!(dir_size(&root.join("no-existe")), 0);
    }

    #[test]
    fn dir_size_does_not_follow_symlinks() {
        let tmp = TempDir::new("dir-size-link");
        let root = tmp.path();
        fs::write(root.join("file"), vec![0u8; 10]).unwrap();
        // Si el enlace se siguiera, la recursión no terminaría.
        std::os::unix::fs::symlink(".", root.join("loop")).unwrap();
        // El enlace cuenta por su propio tamaño (la ruta "." tiene 1 byte).
        assert_eq!(dir_size(root), 11);
    }

    #[test]
    fn aur_cache_dir_for_each_helper() {
        assert_eq!(
            aur_cache_dir_from(Helper::Yay, os("/x"), os("/home/u")),
            Some(PathBuf::from("/x/yay"))
        );
        assert_eq!(
            aur_cache_dir_from(Helper::Paru, os("/x"), os("/home/u")),
            Some(PathBuf::from("/x/paru/clone"))
        );
        assert_eq!(
            aur_cache_dir_from(Helper::Pikaur, os("/x"), os("/home/u")),
            Some(PathBuf::from("/x/pikaur/build"))
        );
        assert_eq!(
            aur_cache_dir_from(Helper::Pacman, os("/x"), os("/home/u")),
            None
        );
    }

    #[test]
    fn aur_cache_dir_falls_back_to_home_and_ignores_relative_paths() {
        assert_eq!(
            aur_cache_dir_from(Helper::Yay, os(""), os("/home/u")),
            Some(PathBuf::from("/home/u/.cache/yay"))
        );
        assert_eq!(
            aur_cache_dir_from(Helper::Yay, os("relativo"), os("/home/u")),
            Some(PathBuf::from("/home/u/.cache/yay"))
        );
        assert_eq!(aur_cache_dir_from(Helper::Yay, None, None), None);
        assert_eq!(
            aur_cache_dir_from(Helper::Yay, os(""), os("relativo")),
            None
        );
    }

    #[test]
    fn command_lines_per_task() {
        let names = ["go", "rust"];
        let orphans = task(CleanKind::Orphans, &names, true);
        assert_eq!(
            command_line(&command(&orphans, Helper::Yay).unwrap()),
            "yay -Rns go rust"
        );
        assert_eq!(
            command_line(&command(&orphans, Helper::Pacman).unwrap()),
            "sudo pacman -Rns go rust"
        );

        let keep = task(CleanKind::PkgCacheKeepLatest, &[], true);
        assert_eq!(
            command_line(&command(&keep, Helper::Yay).unwrap()),
            "sudo paccache -rk1"
        );

        let uninstalled = task(CleanKind::PkgCacheUninstalled, &[], true);
        assert_eq!(
            command_line(&command(&uninstalled, Helper::Yay).unwrap()),
            "sudo paccache -ruk0"
        );

        let aur = task(CleanKind::AurBuildCache, &["/c/yay/a", "/c/yay/b"], true);
        assert_eq!(
            command_line(&command(&aur, Helper::Yay).unwrap()),
            "rm -rf -- /c/yay/a /c/yay/b"
        );

        let facpm = task(CleanKind::FacpmCache, &["/c/facpm/p.json.gz"], true);
        assert_eq!(
            command_line(&command(&facpm, Helper::Yay).unwrap()),
            "rm -f -- /c/facpm/p.json.gz"
        );

        let all = task(CleanKind::PkgCacheAll, &[], true);
        assert_eq!(
            command_line(&command(&all, Helper::Yay).unwrap()),
            "sudo pacman -Scc"
        );
    }

    #[test]
    fn command_is_none_when_unavailable() {
        let kinds = [
            CleanKind::Orphans,
            CleanKind::PkgCacheKeepLatest,
            CleanKind::PkgCacheUninstalled,
            CleanKind::AurBuildCache,
            CleanKind::FacpmCache,
            CleanKind::PkgCacheAll,
        ];
        for kind in kinds {
            let unavailable = task(kind, &["x"], false);
            assert!(command(&unavailable, Helper::Yay).is_none(), "{kind:?}");
        }
    }

    #[test]
    fn orphans_task_sums_sizes_in_order() {
        let t = orphans_task(vec![("go".to_string(), 10), ("rust".to_string(), 5)]);
        assert_eq!(t.kind, CleanKind::Orphans);
        assert_eq!(t.items, vec!["go", "rust"]);
        assert_eq!(t.count, 2);
        assert_eq!(t.bytes, Some(15));
        assert!(t.available);
        assert!(t.note.is_none());

        let empty = orphans_task(Vec::new());
        assert_eq!(empty.count, 0);
        assert!(!empty.available);
    }

    #[test]
    fn aur_build_cache_lists_only_directories_sorted() {
        let tmp = TempDir::new("aur-cache");
        let dir = tmp.path().join("yay");
        fs::create_dir(&dir).unwrap();
        fs::create_dir_all(dir.join("zeta")).unwrap();
        fs::write(dir.join("zeta/y"), vec![0u8; 7]).unwrap();
        fs::create_dir_all(dir.join("alfa")).unwrap();
        fs::write(dir.join("alfa/x"), vec![0u8; 5]).unwrap();
        // Archivos sueltos y un enlace a un directorio: ninguno entra en items.
        fs::write(dir.join("vcs.json"), b"{}").unwrap();
        fs::write(dir.join("completion.cache"), vec![0u8; 100]).unwrap();
        std::os::unix::fs::symlink("alfa", dir.join("enlace")).unwrap();

        let t = aur_build_cache_task(Helper::Yay, Some(dir.clone()));
        assert_eq!(t.count, 2);
        assert_eq!(t.bytes, Some(12));
        assert!(t.available);
        assert!(t.note.is_none());
        assert_eq!(
            t.items,
            vec![
                dir.join("alfa").to_string_lossy().into_owned(),
                dir.join("zeta").to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn aur_build_cache_empty_missing_and_pacman() {
        let tmp = TempDir::new("aur-cache-empty");
        let empty = tmp.path().join("vacio");
        fs::create_dir(&empty).unwrap();
        let t = aur_build_cache_task(Helper::Yay, Some(empty));
        assert_eq!((t.count, t.available, t.note), (0, false, None));

        let missing = aur_build_cache_task(Helper::Yay, Some(tmp.path().join("no-existe")));
        assert_eq!(
            (missing.count, missing.available, missing.bytes),
            (0, false, Some(0))
        );

        let pacman = aur_build_cache_task(Helper::Pacman, None);
        assert!(!pacman.available);
        assert_eq!(pacman.note.as_deref(), Some("no hay AUR helper"));
        assert_eq!(pacman.bytes, None);
    }

    #[test]
    fn facpm_cache_is_the_file_if_it_exists() {
        let tmp = TempDir::new("facpm-cache");
        let file = tmp.path().join("packages-meta-v1.json.gz");
        fs::write(&file, vec![0u8; 42]).unwrap();
        let t = facpm_cache_task(&file);
        assert_eq!(t.items, vec![file.to_string_lossy().into_owned()]);
        assert_eq!(t.count, 1);
        assert_eq!(t.bytes, Some(42));
        assert!(t.available);

        let missing = facpm_cache_task(&tmp.path().join("no-existe.gz"));
        assert!(missing.items.is_empty());
        assert_eq!((missing.count, missing.available), (0, false));
    }
}

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Helper {
    Paru,
    Yay,
    Pikaur,
    Pacman,
}

const AUR_HELPERS: [Helper; 3] = [Helper::Paru, Helper::Yay, Helper::Pikaur];

impl Helper {
    /// `FACPM_HELPER` si está instalado; si no, el primer AUR helper del PATH, o pacman.
    pub fn detect() -> Self {
        let forced = std::env::var_os("FACPM_HELPER");
        let path = std::env::var_os("PATH");
        Self::detect_with(forced.as_deref(), path.as_deref())
    }

    pub fn name(&self) -> &'static str {
        match self {
            Helper::Paru => "paru",
            Helper::Yay => "yay",
            Helper::Pikaur => "pikaur",
            Helper::Pacman => "pacman",
        }
    }

    pub fn supports_aur(&self) -> bool {
        *self != Helper::Pacman
    }

    pub fn install_command(&self, pkgs: &[String]) -> Command {
        self.command_with_op("-S", pkgs)
    }

    pub fn remove_command(&self, pkgs: &[String]) -> Command {
        self.command_with_op("-Rns", pkgs)
    }

    pub fn upgrade_command(&self) -> Command {
        self.command_with_op("-Syu", &[])
    }

    fn command_with_op(&self, op: &str, pkgs: &[String]) -> Command {
        let mut cmd = match self {
            Helper::Pacman => {
                let mut sudo = Command::new("sudo");
                sudo.arg("pacman");
                sudo
            }
            _ => Command::new(self.name()),
        };
        cmd.arg(op).args(pkgs);
        cmd
    }

    fn from_name(name: &str) -> Option<Self> {
        [Helper::Paru, Helper::Yay, Helper::Pikaur, Helper::Pacman]
            .into_iter()
            .find(|h| h.name().eq_ignore_ascii_case(name))
    }

    fn detect_with(forced: Option<&OsStr>, path: Option<&OsStr>) -> Self {
        let forced = forced
            .and_then(|value| Self::from_name(&value.to_string_lossy()))
            .filter(|helper| in_path(helper.name(), path));
        forced.unwrap_or_else(|| {
            AUR_HELPERS
                .into_iter()
                .find(|h| in_path(h.name(), path))
                .unwrap_or(Helper::Pacman)
        })
    }
}

fn in_path(binary: &str, path: Option<&OsStr>) -> bool {
    path.is_some_and(|p| std::env::split_paths(p).any(|dir| is_executable(&dir.join(binary))))
}

fn is_executable(candidate: &Path) -> bool {
    fs::metadata(candidate).is_ok_and(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
}

pub fn command_line(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// Carpeta temporal que se borra al salir del test, pase o falle.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("facpm-helper-{}-{test}", std::process::id()));
            fs::remove_dir_all(&dir).ok();
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    /// PATH falso: una carpeta temporal con un archivo vacío por cada `(nombre, modo)`.
    fn fake_path(test: &str, files: &[(&str, u32)]) -> (TempDir, OsString) {
        let dir = TempDir::new(test);
        for (name, mode) in files {
            let file = dir.0.join(name);
            fs::write(&file, b"").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(*mode)).unwrap();
        }
        let path = std::env::join_paths([&dir.0]).unwrap();
        (dir, path)
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn install_and_remove_for_aur_helpers() {
        let pkgs = strings(&["a", "b"]);
        assert_eq!(
            command_line(&Helper::Yay.install_command(&pkgs)),
            "yay -S a b"
        );
        assert_eq!(
            command_line(&Helper::Paru.remove_command(&pkgs)),
            "paru -Rns a b"
        );
        assert_eq!(
            command_line(&Helper::Pikaur.remove_command(&pkgs)),
            "pikaur -Rns a b"
        );
    }

    #[test]
    fn pacman_commands_go_through_sudo() {
        let pkgs = strings(&["a"]);
        assert_eq!(
            command_line(&Helper::Pacman.install_command(&pkgs)),
            "sudo pacman -S a"
        );
        assert_eq!(
            command_line(&Helper::Pacman.remove_command(&pkgs)),
            "sudo pacman -Rns a"
        );
        assert_eq!(
            command_line(&Helper::Pacman.upgrade_command()),
            "sudo pacman -Syu"
        );
    }

    #[test]
    fn upgrade_command_for_aur_helpers() {
        assert_eq!(command_line(&Helper::Yay.upgrade_command()), "yay -Syu");
        assert_eq!(command_line(&Helper::Paru.upgrade_command()), "paru -Syu");
    }

    #[test]
    fn names_and_aur_support() {
        assert_eq!(Helper::Pikaur.name(), "pikaur");
        assert_eq!(Helper::Pacman.name(), "pacman");
        assert!(Helper::Yay.supports_aur());
        assert!(!Helper::Pacman.supports_aur());
    }

    #[test]
    fn detect_takes_first_aur_helper_on_path() {
        let (_first, path) = fake_path("order-first", &[("yay", 0o755), ("paru", 0o755)]);
        assert_eq!(
            Helper::detect_with(None, Some(path.as_os_str())),
            Helper::Paru
        );
        let (_second, path) = fake_path("order-second", &[("pikaur", 0o755), ("yay", 0o755)]);
        assert_eq!(
            Helper::detect_with(None, Some(path.as_os_str())),
            Helper::Yay
        );
    }

    #[test]
    fn detect_skips_files_without_exec_bit() {
        let (_dir, path) = fake_path("no-exec", &[("yay", 0o644)]);
        assert_eq!(
            Helper::detect_with(None, Some(path.as_os_str())),
            Helper::Pacman
        );
    }

    #[test]
    fn forced_helper_is_used_only_when_installed() {
        let (_dir, path) = fake_path(
            "forced",
            &[("paru", 0o755), ("yay", 0o755), ("pacman", 0o755)],
        );
        let path = Some(path.as_os_str());
        assert_eq!(
            Helper::detect_with(Some(OsStr::new("YAY")), path),
            Helper::Yay
        );
        assert_eq!(
            Helper::detect_with(Some(OsStr::new("pacman")), path),
            Helper::Pacman
        );
    }

    #[test]
    fn missing_or_unknown_forced_helper_falls_back_to_detection() {
        let (_dir, path) = fake_path("fallback", &[("yay", 0o755)]);
        let path = Some(path.as_os_str());
        assert_eq!(
            Helper::detect_with(Some(OsStr::new("paru")), path),
            Helper::Yay
        );
        assert_eq!(
            Helper::detect_with(Some(OsStr::new("xyz")), path),
            Helper::Yay
        );
    }

    #[test]
    fn no_helper_on_path_means_pacman() {
        let (_dir, path) = fake_path("none", &[("otro", 0o755)]);
        assert_eq!(
            Helper::detect_with(None, Some(path.as_os_str())),
            Helper::Pacman
        );
        assert_eq!(Helper::detect_with(None, None), Helper::Pacman);
    }
}

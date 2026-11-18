#[cfg(target_os = "linux")]
use std::ffi::{OsStr, OsString};
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    Editor,
    FileManager,
}

struct AppSpec {
    id: &'static str,
    name: &'static str,
    kind: AppKind,
    #[cfg(target_os = "linux")]
    programs: &'static [&'static str],
    #[cfg(target_os = "linux")]
    desktops: &'static [&'static str],
}

const CATALOGUE: &[AppSpec] = &[
    AppSpec {
        id: "code",
        name: "VS Code",
        kind: AppKind::Editor,
        #[cfg(target_os = "linux")]
        programs: &["code"],
        #[cfg(target_os = "linux")]
        desktops: &["code.desktop", "com.visualstudio.code.desktop"],
    },
    AppSpec {
        id: "zed",
        name: "Zed",
        kind: AppKind::Editor,
        #[cfg(target_os = "linux")]
        programs: &["zed", "zeditor"],
        #[cfg(target_os = "linux")]
        desktops: &["dev.zed.Zed.desktop", "zed.desktop"],
    },
    AppSpec {
        id: "codium",
        name: "VSCodium",
        kind: AppKind::Editor,
        #[cfg(target_os = "linux")]
        programs: &["codium"],
        #[cfg(target_os = "linux")]
        desktops: &["codium.desktop", "com.vscodium.codium.desktop"],
    },
    AppSpec {
        id: "code-insiders",
        name: "VS Code Insiders",
        kind: AppKind::Editor,
        #[cfg(target_os = "linux")]
        programs: &["code-insiders"],
        #[cfg(target_os = "linux")]
        desktops: &["code-insiders.desktop"],
    },
    AppSpec {
        id: "kate",
        name: "Kate",
        kind: AppKind::Editor,
        #[cfg(target_os = "linux")]
        programs: &["kate"],
        #[cfg(target_os = "linux")]
        desktops: &["org.kde.kate.desktop"],
    },
    AppSpec {
        id: "sublime",
        name: "Sublime Text",
        kind: AppKind::Editor,
        #[cfg(target_os = "linux")]
        programs: &["subl", "sublime_text"],
        #[cfg(target_os = "linux")]
        desktops: &["sublime_text.desktop"],
    },
    AppSpec {
        id: "nautilus",
        name: "Nautilus",
        kind: AppKind::FileManager,
        #[cfg(target_os = "linux")]
        programs: &["nautilus"],
        #[cfg(target_os = "linux")]
        desktops: &["org.gnome.Nautilus.desktop", "nautilus.desktop"],
    },
    AppSpec {
        id: "dolphin",
        name: "Dolphin",
        kind: AppKind::FileManager,
        #[cfg(target_os = "linux")]
        programs: &["dolphin"],
        #[cfg(target_os = "linux")]
        desktops: &["org.kde.dolphin.desktop"],
    },
    AppSpec {
        id: "nemo",
        name: "Nemo",
        kind: AppKind::FileManager,
        #[cfg(target_os = "linux")]
        programs: &["nemo"],
        #[cfg(target_os = "linux")]
        desktops: &["nemo.desktop"],
    },
    AppSpec {
        id: "thunar",
        name: "Thunar",
        kind: AppKind::FileManager,
        #[cfg(target_os = "linux")]
        programs: &["thunar", "Thunar"],
        #[cfg(target_os = "linux")]
        desktops: &["thunar.desktop"],
    },
    AppSpec {
        id: "caja",
        name: "Caja",
        kind: AppKind::FileManager,
        #[cfg(target_os = "linux")]
        programs: &["caja"],
        #[cfg(target_os = "linux")]
        desktops: &["caja-folder-handler.desktop", "caja.desktop"],
    },
    AppSpec {
        id: "pcmanfm",
        name: "PCManFM",
        kind: AppKind::FileManager,
        #[cfg(target_os = "linux")]
        programs: &["pcmanfm", "pcmanfm-qt"],
        #[cfg(target_os = "linux")]
        desktops: &["pcmanfm.desktop", "pcmanfm-qt.desktop"],
    },
];

pub fn known(kind: AppKind, id: &str) -> bool {
    id == "auto" || CATALOGUE.iter().any(|app| app.kind == kind && app.id == id)
}

pub fn name(kind: AppKind, id: &str) -> Option<&'static str> {
    CATALOGUE.iter().find(|app| app.kind == kind && app.id == id).map(|app| app.name)
}

#[derive(Clone, Debug)]
pub struct Application {
    pub id: &'static str,
    pub name: &'static str,
    #[cfg(target_os = "linux")]
    program: PathBuf,
    #[cfg(target_os = "linux")]
    prefix: Vec<OsString>,
}

#[cfg(target_os = "linux")]
impl Application {
    fn arguments(&self, directory: &Path) -> io::Result<Vec<OsString>> {
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "folder is unavailable"));
        }
        let mut args = self.prefix.clone();
        args.push(directory.as_os_str().to_owned());
        Ok(args)
    }

    #[cfg(target_os = "linux")]
    pub fn launch(&self, directory: &Path, master_fd: i32, shell_pid: u32) -> io::Result<()> {
        let args = self.arguments(directory)?;
        let program = self.program.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "application path is not supported")
        })?;
        crate::daemon::spawn_daemon(program, &args, master_fd, shell_pid)
    }
}

#[derive(Default)]
pub struct InstalledApps {
    pub editors: Vec<Application>,
    pub file_managers: Vec<Application>,
    default_manager: Option<Application>,
}

impl InstalledApps {
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            let mut roots = Vec::new();
            let home = std::env::var_os("HOME").map(PathBuf::from);
            if let Some(data) = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .or_else(|| home.as_ref().map(|home| home.join(".local/share")))
            {
                roots.push(data.join("applications"));
            }
            let data_dirs = std::env::var_os("XDG_DATA_DIRS")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            roots.extend(
                std::env::split_paths(&data_dirs)
                    .filter(|path| path.is_absolute())
                    .map(|path| path.join("applications")),
            );
            if let Some(home) = home {
                roots.push(home.join(".local/share/flatpak/exports/share/applications"));
            }
            roots.push("/var/lib/flatpak/exports/share/applications".into());
            Self::detect_in(&std::env::var_os("PATH").unwrap_or_default(), &roots)
        }
        #[cfg(not(target_os = "linux"))]
        Self::default()
    }

    pub fn choices(&self, kind: AppKind) -> &[Application] {
        match kind {
            AppKind::Editor => &self.editors,
            AppKind::FileManager => &self.file_managers,
        }
    }

    pub fn resolve(&self, kind: AppKind, preference: &str) -> Option<&Application> {
        if preference == "auto" {
            if kind == AppKind::FileManager {
                return self.default_manager.as_ref().or_else(|| self.file_managers.first());
            }
            self.editors.first()
        } else {
            self.choices(kind).iter().find(|app| app.id == preference)
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn detect_in(path: &OsStr, desktop_dirs: &[PathBuf]) -> Self {
        let executable = |name: &str| {
            std::env::split_paths(path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(name))
                .find(|path| is_executable(path))
        };
        let gio = executable("gio");
        let mut apps = Self::default();
        for spec in CATALOGUE {
            let program = spec
                .programs
                .iter()
                .find_map(|name| executable(name))
                .map(|program| (program, Vec::new()))
                .or_else(|| {
                    let gio = gio.as_ref()?;
                    let desktop = desktop_dirs
                        .iter()
                        .flat_map(|dir| spec.desktops.iter().map(move |name| dir.join(name)))
                        .find(|path| path.is_file())?;
                    Some((gio.clone(), vec!["launch".into(), desktop.into_os_string()]))
                });
            let Some((program, prefix)) = program else { continue };
            let app = Application { id: spec.id, name: spec.name, program, prefix };
            match spec.kind {
                AppKind::Editor => apps.editors.push(app),
                AppKind::FileManager => apps.file_managers.push(app),
            }
        }
        apps.default_manager = executable("xdg-open")
            .map(|program| Application { id: "auto", name: "", program, prefix: Vec::new() })
            .or_else(|| {
                gio.map(|program| Application {
                    id: "auto",
                    name: "",
                    program,
                    prefix: vec!["open".into()],
                })
            });
        apps
    }
}

#[cfg(target_os = "linux")]
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|metadata| {
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn program(root: &Path, name: &str) {
        let path = root.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn detection_filters_missing_nonexecutable_and_duplicate_aliases() {
        let root = tempfile::tempdir().unwrap();
        for name in ["code", "zed", "zeditor", "nautilus", "dolphin", "xdg-open"] {
            program(root.path(), name);
        }
        fs::write(root.path().join("codium"), "not executable").unwrap();
        fs::create_dir(root.path().join("kate")).unwrap();
        let apps = InstalledApps::detect_in(root.path().as_os_str(), &[]);
        assert_eq!(apps.editors.iter().map(|app| app.id).collect::<Vec<_>>(), ["code", "zed"]);
        assert_eq!(apps.file_managers.iter().map(|app| app.id).collect::<Vec<_>>(), [
            "nautilus", "dolphin"
        ]);
        assert_eq!(apps.resolve(AppKind::Editor, "auto").unwrap().id, "code");
        assert_eq!(apps.resolve(AppKind::FileManager, "auto").unwrap().id, "auto");
        assert!(apps.resolve(AppKind::Editor, "kate").is_none());
        assert!(apps.resolve(AppKind::Editor, "nautilus").is_none());
        assert!(InstalledApps::detect_in(OsStr::new(".:"), &[]).editors.is_empty());
    }

    #[test]
    fn registered_desktop_launchers_keep_file_arguments_separate() {
        let root = tempfile::tempdir().unwrap();
        program(root.path(), "gio");
        let desktop = root.path().join("dev.zed.Zed.desktop");
        fs::write(&desktop, "[Desktop Entry]\nType=Application\nExec=zed %U\n").unwrap();
        let folder = root.path().join("ação $(touch injected); 'space'");
        fs::create_dir(&folder).unwrap();
        let apps = InstalledApps::detect_in(root.path().as_os_str(), &[root.path().into()]);
        let app = apps.resolve(AppKind::Editor, "zed").unwrap();
        assert_eq!(app.arguments(&folder).unwrap(), [
            OsString::from("launch"),
            desktop.into_os_string(),
            folder.into_os_string()
        ]);
        assert!(app.arguments(Path::new("relative")).is_err());
        assert!(app.arguments(&root.path().join("missing")).is_err());
    }

    #[test]
    fn direct_launch_preserves_literal_unicode_and_metacharacters() {
        let root = tempfile::tempdir().unwrap();
        program(root.path(), "code");
        let folder = root.path().join("--ação ; $(false) 'quoted'");
        fs::create_dir(&folder).unwrap();
        let apps = InstalledApps::detect_in(root.path().as_os_str(), &[]);
        let app = apps.resolve(AppKind::Editor, "code").unwrap();
        assert_eq!(app.arguments(&folder).unwrap(), [folder.into_os_string()]);
        assert!(known(AppKind::Editor, "code"));
        assert!(!known(AppKind::FileManager, "code"));
    }
}

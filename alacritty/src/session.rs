use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_COMMANDS: usize = 20;
const MAX_TABS: usize = 64;
const MAX_FILE_SIZE: usize = 4 * 1024 * 1024;
const MAX_COMMAND_SIZE: usize = 16 * 1024;
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CommandRecord {
    pub command: String,
    pub working_directory: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedShell {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SavedTab {
    pub title: Option<String>,
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub shell: Option<SavedShell>,
    #[serde(default)]
    pub commands: Vec<CommandRecord>,
    #[serde(default)]
    pub replay_last_command: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<SavedTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<crate::panes::PaneNode>,
    #[serde(default)]
    pub active_pane: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedSession {
    #[serde(default = "current_version")]
    pub version: u32,
    pub active: usize,
    pub tabs: Vec<SavedTab>,
}

#[derive(Clone, Debug)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn default_path() -> Option<PathBuf> {
        #[cfg(windows)]
        let root = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(home::home_dir);

        #[cfg(not(windows))]
        let root = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home::home_dir().map(|home| home.join(".local/state")));

        root.map(|root| root.join("alacritty/tabs.json"))
    }

    pub fn recovery_path() -> Option<PathBuf> {
        Self::default_path().map(|path| path.with_file_name("recovery.json"))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<Option<SavedSession>> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };

        if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE as u64 {
            return Err(invalid_data());
        }

        let mut data = Vec::new();
        File::open(&self.path)?.take((MAX_FILE_SIZE + 1) as u64).read_to_end(&mut data)?;
        if data.len() > MAX_FILE_SIZE {
            return Err(invalid_data());
        }

        let snapshot: SavedSession = serde_json::from_slice(&data).map_err(|_| invalid_data())?;
        validate(&snapshot)?;
        Ok(Some(snapshot))
    }

    pub fn save(&self, snapshot: &SavedSession) -> io::Result<()> {
        validate(snapshot)?;
        let data = serde_json::to_vec(snapshot).map_err(|_| invalid_data())?;
        if data.len() > MAX_FILE_SIZE {
            return Err(invalid_data());
        }

        let parent = self.path.parent().filter(|path| !path.as_os_str().is_empty());
        let parent = parent.unwrap_or_else(|| Path::new("."));
        let mut directories = fs::DirBuilder::new();
        directories.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directories.mode(0o700);
        }
        directories.create(parent)?;

        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temporary.write_all(&data)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;

        #[cfg(unix)]
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceStore {
    path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct WorkspaceDocument {
    #[serde(default = "current_version")]
    version: u32,
    workspaces: std::collections::BTreeMap<String, SavedSession>,
}

impl WorkspaceStore {
    pub fn default_path() -> Option<PathBuf> {
        SessionStore::default_path().map(|path| path.with_file_name("workspaces.json"))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self) -> io::Result<Vec<String>> {
        let (document, _) = self.read()?;
        Ok(document.workspaces.into_keys().collect())
    }

    pub fn load(&self, name: &str) -> io::Result<Option<SavedSession>> {
        let name = workspace_name(name)?;
        let (mut document, _) = self.read()?;
        Ok(document.workspaces.remove(name))
    }

    pub fn save(&self, name: &str, snapshot: &SavedSession) -> io::Result<()> {
        let name = workspace_name(name)?;
        validate(snapshot)?;
        let (mut document, previous) = self.read()?;
        document.workspaces.insert(name.to_owned(), snapshot.clone());
        self.write(&document, previous.as_deref())
    }

    pub fn remove(&self, name: &str) -> io::Result<bool> {
        let name = workspace_name(name)?;
        let (mut document, previous) = self.read()?;
        if document.workspaces.remove(name).is_none() {
            return Ok(false);
        }
        self.write(&document, previous.as_deref())?;
        Ok(true)
    }

    fn read(&self) -> io::Result<(WorkspaceDocument, Option<Vec<u8>>)> {
        let Some(data) = read_workspace_bytes(&self.path)? else {
            let document = WorkspaceDocument { version: VERSION, ..Default::default() };
            return Ok((document, None));
        };
        let document: WorkspaceDocument =
            serde_json::from_slice(&data).map_err(|_| invalid_data())?;
        validate_workspaces(&document)?;
        Ok((document, Some(data)))
    }

    fn write(&self, document: &WorkspaceDocument, previous: Option<&[u8]>) -> io::Result<()> {
        validate_workspaces(document)?;
        let data = serde_json::to_vec(document).map_err(|_| invalid_data())?;
        if data.len() > MAX_WORKSPACE_FILE_SIZE {
            return Err(invalid_data());
        }

        let parent = self.path.parent().filter(|path| !path.as_os_str().is_empty());
        let parent = parent.unwrap_or_else(|| Path::new("."));
        let mut directories = fs::DirBuilder::new();
        directories.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directories.mode(0o700);
        }
        directories.create(parent)?;

        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temporary.write_all(&data)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        if read_workspace_bytes(&self.path)?.as_deref() != previous {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "workspace store changed while saving",
            ));
        }
        if previous.is_some() {
            temporary.persist(&self.path).map_err(|error| error.error)?;
        } else {
            temporary.persist_noclobber(&self.path).map_err(|error| error.error)?;
        }
        #[cfg(unix)]
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }
}

const MAX_WORKSPACES: usize = 32;
const MAX_WORKSPACE_NAME_CHARS: usize = 80;
const MAX_WORKSPACE_FILE_SIZE: usize = 8 * 1024 * 1024;

fn workspace_name(name: &str) -> io::Result<&str> {
    if name.chars().any(char::is_control) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid workspace name"));
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_WORKSPACE_NAME_CHARS {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid workspace name"));
    }
    Ok(name)
}

fn validate_workspaces(document: &WorkspaceDocument) -> io::Result<()> {
    if document.version != VERSION || document.workspaces.len() > MAX_WORKSPACES {
        return Err(invalid_data());
    }
    for (name, snapshot) in &document.workspaces {
        if workspace_name(name).ok() != Some(name.as_str()) {
            return Err(invalid_data());
        }
        validate(snapshot)?;
        if serde_json::to_vec(snapshot).map_err(|_| invalid_data())?.len() > MAX_FILE_SIZE {
            return Err(invalid_data());
        }
    }
    Ok(())
}

fn read_workspace_bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.len() > MAX_WORKSPACE_FILE_SIZE as u64 {
        return Err(invalid_data());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(invalid_data());
        }
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() > MAX_WORKSPACE_FILE_SIZE as u64 {
        return Err(invalid_data());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.nlink() != 1
            || opened.uid() != metadata.uid()
        {
            return Err(invalid_data());
        }
    }
    let mut data = Vec::new();
    file.take((MAX_WORKSPACE_FILE_SIZE + 1) as u64).read_to_end(&mut data)?;
    if data.len() > MAX_WORKSPACE_FILE_SIZE {
        return Err(invalid_data());
    }
    Ok(Some(data))
}

fn current_version() -> u32 {
    VERSION
}

fn invalid_data() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid tab session data")
}

fn validate(snapshot: &SavedSession) -> io::Result<()> {
    if snapshot.version != VERSION
        || snapshot.tabs.is_empty()
        || snapshot.tabs.len() > MAX_TABS
        || snapshot.active >= snapshot.tabs.len()
    {
        return Err(invalid_data());
    }

    for group in &snapshot.tabs {
        let count = group.panes.len() + 1;
        if count > crate::panes::MAX_PANES
            || group.active_pane >= count
            || group.panes.iter().any(|pane| {
                !pane.panes.is_empty() || pane.layout.is_some() || pane.active_pane != 0
            })
        {
            return Err(invalid_data());
        }
        match &group.layout {
            Some(layout) => {
                if !layout.validate() {
                    return Err(invalid_data());
                }
                let mut ids = layout.leaf_ids();
                ids.sort_unstable();
                if ids != (0..count as u64).collect::<Vec<_>>() {
                    return Err(invalid_data());
                }
            },
            None if count != 1 => return Err(invalid_data()),
            None => (),
        }
    }
    for tab in snapshot.tabs.iter().flat_map(|tab| std::iter::once(tab).chain(tab.panes.iter())) {
        if tab.title.as_ref().is_some_and(|title| title.contains('\0'))
            || tab.commands.len() > MAX_COMMANDS
        {
            return Err(invalid_data());
        }

        if let Some(shell) = &tab.shell {
            if shell.program.is_empty()
                || shell.program.len() > 4096
                || shell.program.contains('\0')
                || shell.args.len() > 32
                || shell.args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0'))
            {
                return Err(invalid_data());
            }

            let name = Path::new(&shell.program).file_name().and_then(|name| name.to_str());
            let supported = match name {
                Some("bash") => shell
                    .args
                    .iter()
                    .all(|arg| matches!(arg.as_str(), "-i" | "--noprofile" | "--norc")),
                Some("zsh") => shell.args.iter().all(|arg| arg == "-i"),
                Some("fish") => {
                    shell.args.iter().all(|arg| matches!(arg.as_str(), "-i" | "--interactive"))
                },
                _ => false,
            };
            if !supported {
                return Err(invalid_data());
            }
        }

        for command in &tab.commands {
            if command.command.len() > MAX_COMMAND_SIZE || command.command.contains('\0') {
                return Err(invalid_data());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_paths_are_separate_from_manual_sessions() {
        let path = SessionStore::default_path();
        assert_eq!(
            WorkspaceStore::default_path(),
            path.as_ref().map(|path| path.with_file_name("workspaces.json")),
        );
        assert_eq!(
            SessionStore::recovery_path(),
            path.map(|path| path.with_file_name("recovery.json")),
        );
    }

    #[test]
    fn workspace_roundtrip_sorted_names_replacement_and_removal() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path().join("state/workspaces.json"));
        let original = snapshot();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.load("Backend").unwrap(), None);
        assert!(!store.remove("Backend").unwrap());
        store.save("  Infra  ", &original).unwrap();
        store.save("Backend", &original).unwrap();
        store.save("日本語", &original).unwrap();
        assert_eq!(store.list().unwrap(), ["Backend", "Infra", "日本語"]);
        assert_eq!(store.load(" Infra ").unwrap(), Some(original.clone()));
        let mut updated = original;
        updated.active = 0;
        updated.tabs[0].replay_last_command = false;
        store.save("Backend", &updated).unwrap();
        assert_eq!(store.load("Backend").unwrap(), Some(updated));
        assert!(store.remove(" Infra ").unwrap());
        assert!(!store.remove("Infra").unwrap());
        assert_eq!(store.list().unwrap(), ["Backend", "日本語"]);
        assert_eq!(fs::read_dir(directory.path().join("state")).unwrap().count(), 1);
    }

    #[test]
    fn workspace_names_are_validated_and_never_used_as_paths() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path().join("workspaces.json"));
        let original = snapshot();
        let too_long = "🦀".repeat(MAX_WORKSPACE_NAME_CHARS + 1);
        for name in ["", "   ", "tab\nname", "\nBackend", "name\0", too_long.as_str()] {
            assert_eq!(
                store.save(name, &original).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(store.load(name).unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(store.remove(name).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!store.path.exists());
        let longest = "🦀".repeat(MAX_WORKSPACE_NAME_CHARS);
        store.save(&longest, &original).unwrap();
        store.save("../separate.json", &original).unwrap();
        assert_eq!(store.load("../separate.json").unwrap(), Some(original));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_workspace_stores_are_preserved_and_errors_hide_content() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path().join("workspaces.json"));
        for corrupt in [
            "secret-command",
            r#"{"version":999,"workspaces":{}}"#,
            r#"{"version":1,"workspaces":{" broken ":{"active":0,"tabs":[{}]}}}"#,
            r#"{"version":1,"workspaces":{"Backend":{"active":7,"tabs":[{}]}}}"#,
        ] {
            fs::write(&store.path, corrupt).unwrap();
            let error = store.list().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!error.to_string().contains("secret-command"));
            assert!(store.save("Backend", &snapshot()).is_err());
            assert!(store.remove("Backend").is_err());
            assert_eq!(fs::read_to_string(&store.path).unwrap(), corrupt);
        }
    }

    #[test]
    fn workspace_count_limit_allows_replacement_and_rejects_new_entries() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path().join("workspaces.json"));
        let original = snapshot();
        for index in 0..MAX_WORKSPACES {
            store.save(&format!("Project {index:02}"), &original).unwrap();
        }
        assert_eq!(store.list().unwrap().len(), MAX_WORKSPACES);
        store.save("Project 00", &original).unwrap();
        let previous = fs::read(&store.path).unwrap();
        assert_eq!(
            store.save("One more", &original).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&store.path).unwrap(), previous);
        assert!(store.remove("Project 00").unwrap());
        store.save("One more", &original).unwrap();
    }

    #[test]
    fn invalid_workspace_snapshots_and_total_size_never_replace_saved_data() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path().join("workspaces.json"));
        let original = snapshot();
        store.save("Backend", &original).unwrap();
        let previous = fs::read(&store.path).unwrap();
        let mut invalid = original.clone();
        invalid.version = VERSION + 1;
        assert!(store.save("Backend", &invalid).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), previous);
        invalid = original.clone();
        invalid.tabs[0].title = Some("x".repeat(MAX_FILE_SIZE));
        assert!(store.save("Backend", &invalid).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), previous);
        let mut large = original;
        large.tabs[0].title = Some("x".repeat(3 * 1024 * 1024));
        store.save("Large one", &large).unwrap();
        store.save("Large two", &large).unwrap();
        let previous = fs::read(&store.path).unwrap();
        assert!(store.save("Large three", &large).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), previous);
    }

    #[test]
    fn oversized_workspace_files_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspaces.json");
        File::create(&path).unwrap().set_len((MAX_WORKSPACE_FILE_SIZE + 1) as u64).unwrap();
        let store = WorkspaceStore::new(path.clone());
        assert!(store.list().is_err());
        assert!(store.save("Backend", &snapshot()).is_err());
        assert_eq!(fs::metadata(path).unwrap().len(), (MAX_WORKSPACE_FILE_SIZE + 1) as u64);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_storage_is_private_and_rejects_linked_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path().join("private/state/workspaces.json"));
        store.save("Backend", &snapshot()).unwrap();
        assert_eq!(fs::metadata(&store.path).unwrap().permissions().mode() & 0o777, 0o600);
        let parent = store.path.parent().unwrap();
        assert_eq!(fs::metadata(parent).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(parent.parent().unwrap()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        store.save("Infra", &snapshot()).unwrap();
        assert_eq!(fs::metadata(parent).unwrap().permissions().mode() & 0o777, 0o755);
        let original = fs::read(&store.path).unwrap();
        let hard_link = directory.path().join("hard.json");
        fs::hard_link(&store.path, &hard_link).unwrap();
        assert!(store.list().is_err());
        assert!(store.save("Backend", &snapshot()).is_err());
        assert!(store.remove("Backend").is_err());
        assert_eq!(fs::read(&hard_link).unwrap(), original);
        fs::remove_file(&hard_link).unwrap();
        let link = directory.path().join("link.json");
        symlink(&store.path, &link).unwrap();
        let linked = WorkspaceStore::new(link.clone());
        assert!(linked.list().is_err());
        assert!(linked.save("Backend", &snapshot()).is_err());
        assert!(linked.remove("Backend").is_err());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }

    fn snapshot() -> SavedSession {
        SavedSession {
            version: VERSION,
            active: 1,
            tabs: vec![
                SavedTab {
                    title: Some("API — produção 🦀".into()),
                    working_directory: Some(PathBuf::from("/srv/projeto atual")),
                    shell: Some(SavedShell {
                        program: "/bin/bash".into(),
                        args: vec!["--norc".into(), "-i".into()],
                    }),
                    commands: vec![CommandRecord {
                        command: "printf '%s\\n' 'olá'\n  echo terminado".into(),
                        working_directory: PathBuf::from("/srv/outro projeto"),
                    }],
                    replay_last_command: true,
                    ..SavedTab::default()
                },
                SavedTab::default(),
            ],
        }
    }

    fn split_snapshot() -> SavedSession {
        use crate::panes::{Axis, PaneNode};

        let mut original = snapshot();
        let group = &mut original.tabs[0];
        group.panes = vec![
            SavedTab {
                title: Some("Logs".into()),
                working_directory: Some(PathBuf::from("/srv/logs diários")),
                shell: Some(SavedShell { program: "/bin/zsh".into(), args: vec!["-i".into()] }),
                commands: vec![CommandRecord {
                    command: "tail -f app.log".into(),
                    working_directory: PathBuf::from("/srv/logs originais"),
                }],
                ..SavedTab::default()
            },
            SavedTab {
                title: Some("Testes".into()),
                working_directory: Some(PathBuf::from("/srv/tests")),
                shell: Some(SavedShell {
                    program: "/bin/fish".into(),
                    args: vec!["--interactive".into()],
                }),
                commands: vec![CommandRecord {
                    command: "cargo test".into(),
                    working_directory: PathBuf::from("/srv/projeto original"),
                }],
                replay_last_command: true,
                ..SavedTab::default()
            },
        ];
        group.layout = Some(PaneNode::Split {
            axis: Axis::Vertical,
            ratio: 650,
            first: Box::new(PaneNode::Leaf(0)),
            second: Box::new(PaneNode::Split {
                axis: Axis::Horizontal,
                ratio: 300,
                first: Box::new(PaneNode::Leaf(1)),
                second: Box::new(PaneNode::Leaf(2)),
            }),
        });
        group.active_pane = 2;
        original
    }

    #[test]
    fn legacy_session_files_load_as_single_panes_without_replay_opt_in() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));
        fs::write(
            store.path(),
            br#"{"active":0,"tabs":[{"title":"Backend","working_directory":"/srv/api"}]}"#,
        )
        .unwrap();

        let restored = store.load().unwrap().unwrap();
        assert_eq!(restored.version, VERSION);
        assert_eq!(restored.active, 0);
        assert_eq!(restored.tabs.len(), 1);
        assert_eq!(restored.tabs[0].title.as_deref(), Some("Backend"));
        assert_eq!(restored.tabs[0].working_directory, Some(PathBuf::from("/srv/api")));
        assert!(restored.tabs[0].panes.is_empty());
        assert!(restored.tabs[0].layout.is_none());
        assert_eq!(restored.tabs[0].active_pane, 0);
        assert!(!restored.tabs[0].replay_last_command);
        store.save(&restored).unwrap();
        assert_eq!(store.load().unwrap(), Some(restored));
    }

    #[test]
    fn split_session_round_trip_preserves_layout_focus_and_each_shell_context() {
        let directory = tempfile::tempdir().unwrap();
        let manual = SessionStore::new(directory.path().join("tabs.json"));
        let recovery = SessionStore::new(directory.path().join("recovery.json"));
        let workspaces = WorkspaceStore::new(directory.path().join("workspaces.json"));
        let original = split_snapshot();

        manual.save(&original).unwrap();
        recovery.save(&original).unwrap();
        workspaces.save("Desenvolvimento", &original).unwrap();
        assert_eq!(manual.load().unwrap(), Some(original.clone()));
        assert_eq!(recovery.load().unwrap(), Some(original.clone()));
        assert_eq!(workspaces.load("Desenvolvimento").unwrap(), Some(original));
    }

    #[test]
    fn invalid_split_metadata_never_replaces_manual_or_named_sessions() {
        use crate::panes::{Axis, MAX_PANES, PaneNode};

        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));
        let workspaces = WorkspaceStore::new(directory.path().join("workspaces.json"));
        let original = split_snapshot();
        store.save(&original).unwrap();
        workspaces.save("Backend", &original).unwrap();
        let previous = fs::read(store.path()).unwrap();
        let previous_workspaces = fs::read(&workspaces.path).unwrap();
        let mut cases = Vec::new();

        let mut duplicate = original.clone();
        duplicate.tabs[0].layout =
            Some(duplicate.tabs[0].layout.as_ref().unwrap().remap_ids(|id| id.min(1)));
        cases.push(duplicate);
        let mut missing = original.clone();
        missing.tabs[0].layout = Some(PaneNode::Leaf(0));
        cases.push(missing);
        let mut unknown = original.clone();
        unknown.tabs[0].layout =
            Some(unknown.tabs[0].layout.as_ref().unwrap().remap_ids(|id| id + 1));
        cases.push(unknown);
        let mut absent = original.clone();
        absent.tabs[0].layout = None;
        cases.push(absent);
        for ratio in [0, 99, 901, u16::MAX] {
            let mut invalid_ratio = original.clone();
            let Some(PaneNode::Split { ratio: saved_ratio, .. }) =
                &mut invalid_ratio.tabs[0].layout
            else {
                unreachable!()
            };
            *saved_ratio = ratio;
            cases.push(invalid_ratio);
        }
        let mut nested = original.clone();
        nested.tabs[0].panes[0].panes.push(SavedTab::default());
        cases.push(nested);
        let mut nested_layout = original.clone();
        nested_layout.tabs[0].panes[0].layout = Some(PaneNode::Leaf(0));
        cases.push(nested_layout);
        let mut nested_focus = original.clone();
        nested_focus.tabs[0].panes[0].active_pane = 1;
        cases.push(nested_focus);
        let mut invalid_focus = original.clone();
        invalid_focus.tabs[0].active_pane = 3;
        cases.push(invalid_focus);
        let mut invalid_single_focus = original.clone();
        invalid_single_focus.tabs[1].active_pane = 1;
        cases.push(invalid_single_focus);
        let mut too_many = original.clone();
        too_many.tabs[0].panes.resize(MAX_PANES, SavedTab::default());
        cases.push(too_many);
        let mut excessive_depth = original.clone();
        let mut tree = PaneNode::Leaf(0);
        for id in 1..=20 {
            tree = PaneNode::Split {
                axis: Axis::Horizontal,
                ratio: 500,
                first: Box::new(tree),
                second: Box::new(PaneNode::Leaf(id)),
            };
        }
        excessive_depth.tabs[0].layout = Some(tree);
        cases.push(excessive_depth);
        let mut invalid_child_shell = original.clone();
        invalid_child_shell.tabs[0].panes[0].shell.as_mut().unwrap().args =
            vec!["-c".into(), "private command".into()];
        cases.push(invalid_child_shell);
        let mut invalid_child_command = original.clone();
        invalid_child_command.tabs[0].panes[0].commands[0].command.push('\0');
        cases.push(invalid_child_command);

        for invalid in cases {
            assert_eq!(store.save(&invalid).unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read(store.path()).unwrap(), previous);
            assert_eq!(store.load().unwrap(), Some(original.clone()));
            assert_eq!(
                workspaces.save("Backend", &invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
            );
            assert_eq!(fs::read(&workspaces.path).unwrap(), previous_workspaces);
            assert_eq!(workspaces.load("Backend").unwrap(), Some(original.clone()));
        }
    }

    #[test]
    fn malformed_split_files_fail_with_private_errors_and_preserve_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));
        let original = serde_json::to_value(split_snapshot()).unwrap();
        let mut cases = Vec::new();
        for layout in [
            serde_json::json!({"leaf": 99}),
            serde_json::json!({"leaf": -1}),
            serde_json::json!({"leaf": 18446744073709551616_u128.to_string()}),
            serde_json::json!({"split": {
                "axis": "private command", "ratio": 500,
                "first": {"leaf": 0}, "second": {"leaf": 1}
            }}),
            serde_json::json!({"split": {
                "axis": "vertical", "ratio": 65536,
                "first": {"leaf": 0}, "second": {"leaf": 1}
            }}),
        ] {
            let mut invalid = original.clone();
            invalid["tabs"][0]["layout"] = layout;
            cases.push(serde_json::to_vec(&invalid).unwrap());
        }

        let mut deep_layout = String::from(r#"{"leaf":0}"#);
        let mut deep_tab = String::from(r#"{"title":null,"working_directory":null}"#);
        for _ in 0..160 {
            deep_layout = format!(
                r#"{{"split":{{"axis":"vertical","ratio":500,"first":{deep_layout},"second":{{"leaf":1}}}}}}"#,
            );
            deep_tab = format!(r#"{{"title":null,"working_directory":null,"panes":[{deep_tab}]}}"#);
        }
        cases.push(format!(
            r#"{{"active":0,"tabs":[{{"title":null,"working_directory":null,"layout":{deep_layout}}}]}}"#,
        ).into_bytes());
        cases.push(format!(r#"{{"active":0,"tabs":[{deep_tab}]}}"#).into_bytes());

        for data in cases {
            fs::write(store.path(), &data).unwrap();
            let error = store.load().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "invalid tab session data");
            assert_eq!(fs::read(store.path()).unwrap(), data);
        }
    }

    #[test]
    fn preserves_names_directories_multiline_commands_and_replay() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("state/tabs.json"));
        let original = snapshot();

        store.save(&original).unwrap();

        assert_eq!(store.load().unwrap(), Some(original));
        assert_eq!(store.path(), directory.path().join("state/tabs.json"));
    }

    #[test]
    fn missing_session_is_absent() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));

        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn replay_requires_explicit_opt_in_and_version_defaults_to_current() {
        let original = snapshot();
        let mut value = serde_json::to_value(original).unwrap();
        value.as_object_mut().unwrap().remove("version");
        value["tabs"][0].as_object_mut().unwrap().remove("replay_last_command");
        value["tabs"][0].as_object_mut().unwrap().remove("shell");

        let restored: SavedSession = serde_json::from_value(value).unwrap();

        assert_eq!(restored.version, VERSION);
        assert!(!restored.tabs[0].replay_last_command);
        assert_eq!(restored.tabs[0].shell, None);
    }

    #[test]
    fn malformed_and_unknown_version_are_rejected_without_data_in_errors() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));
        let mut unsupported = snapshot();
        unsupported.version += 1;
        let invalid =
            [b"{private command data".to_vec(), serde_json::to_vec(&unsupported).unwrap()];

        for data in invalid {
            fs::write(store.path(), data).unwrap();
            let error = store.load().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "invalid tab session data");
        }
    }

    #[test]
    fn invalid_snapshots_never_replace_the_previous_session() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));
        let original = snapshot();
        store.save(&original).unwrap();
        let previous = fs::read(store.path()).unwrap();

        let mut invalid = Vec::new();
        let mut unknown_version = original.clone();
        unknown_version.version += 1;
        invalid.push(unknown_version);
        let mut empty = original.clone();
        empty.tabs.clear();
        invalid.push(empty);
        let mut active = original.clone();
        active.active = active.tabs.len();
        invalid.push(active);
        let mut tabs = original.clone();
        tabs.tabs.resize(MAX_TABS + 1, SavedTab::default());
        invalid.push(tabs);
        let mut commands = original.clone();
        let command = commands.tabs[0].commands[0].clone();
        commands.tabs[0].commands.resize(MAX_COMMANDS + 1, command);
        invalid.push(commands);
        let mut long_command = original.clone();
        long_command.tabs[0].commands[0].command = "x".repeat(MAX_COMMAND_SIZE + 1);
        invalid.push(long_command);
        let mut nul_command = original.clone();
        nul_command.tabs[0].commands[0].command.push('\0');
        invalid.push(nul_command);
        let mut nul_title = original.clone();
        nul_title.tabs[0].title = Some("invalid\0title".into());
        invalid.push(nul_title);
        let mut oversized = original.clone();
        oversized.tabs[0].title = Some("x".repeat(MAX_FILE_SIZE));
        invalid.push(oversized);

        let invalid_shells = [
            SavedShell { program: String::new(), args: Vec::new() },
            SavedShell { program: "/bin/bash\0".into(), args: Vec::new() },
            SavedShell { program: format!("{}/bash", "x".repeat(4096)), args: Vec::new() },
            SavedShell { program: "/bin/python".into(), args: Vec::new() },
            SavedShell {
                program: "/bin/bash".into(),
                args: vec!["-c".into(), "echo private".into()],
            },
            SavedShell {
                program: "/bin/bash".into(),
                args: vec!["--rcfile".into(), "/tmp/script".into()],
            },
            SavedShell { program: "/bin/zsh".into(), args: vec!["-l".into()] },
            SavedShell {
                program: "/bin/fish".into(),
                args: vec!["-c".into(), "echo private".into()],
            },
            SavedShell { program: "/bin/bash".into(), args: vec!["-i\0".into()] },
            SavedShell { program: "/bin/bash".into(), args: vec!["x".repeat(4097)] },
            SavedShell { program: "/bin/bash".into(), args: vec!["-i".into(); 33] },
        ];
        for shell in invalid_shells {
            let mut snapshot = original.clone();
            snapshot.tabs[0].shell = Some(shell);
            invalid.push(snapshot);
        }

        for snapshot in invalid {
            assert_eq!(store.save(&snapshot).unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read(store.path()).unwrap(), previous);
            assert_eq!(store.load().unwrap(), Some(original.clone()));
        }
    }

    #[test]
    fn oversized_files_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("tabs.json"));
        File::create(store.path()).unwrap().set_len((MAX_FILE_SIZE + 1) as u64).unwrap();

        assert_eq!(store.load().unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn new_state_is_private_and_existing_parent_permissions_are_preserved() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let store = SessionStore::new(directory.path().join("state/alacritty/tabs.json"));
        store.save(&snapshot()).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(directory.path()), 0o755);
        assert_eq!(mode(&directory.path().join("state")), 0o700);
        assert_eq!(mode(&directory.path().join("state/alacritty")), 0o700);
        assert_eq!(mode(store.path()), 0o600);

        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).unwrap();
        store.save(&snapshot()).unwrap();
        assert_eq!(mode(store.path()), 0o600);
        assert_eq!(mode(directory.path()), 0o755);
    }
}

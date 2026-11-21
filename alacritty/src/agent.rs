use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use alacritty_terminal::tty::Options;
use serde::Deserialize;
use tempfile::TempDir;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    Opencode,
    Agy,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Opencode => "OpenCode",
            Self::Agy => "Antigravity",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Ready,
    Working,
    Input,
    Permission,
    Done,
    Error,
    Unknown,
}

impl AgentState {
    pub fn priority(self) -> u8 {
        match self {
            Self::Permission => 7,
            Self::Input => 6,
            Self::Error => 5,
            Self::Done => 4,
            Self::Working => 3,
            Self::Unknown => 2,
            Self::Ready => 1,
        }
    }

    pub fn attention(self) -> bool {
        matches!(self, Self::Input | Self::Permission | Self::Done | Self::Error)
    }

    pub fn animated(self) -> bool {
        matches!(self, Self::Working | Self::Input | Self::Permission)
    }

    pub fn marker(self, frame: Option<usize>) -> &'static str {
        match self {
            Self::Permission => "!",
            Self::Error => "×",
            Self::Input => "?",
            Self::Done => "✓",
            Self::Working => frame.map_or("●", |frame| ["◐", "◓", "◑", "◒"][(frame / 2) % 4]),
            Self::Ready => "·",
            Self::Unknown => "◌",
        }
    }
}

pub fn animation_frame() -> usize {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    (START.get_or_init(std::time::Instant::now).elapsed().as_millis() / 125 % 8) as usize
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentStatus {
    pub provider: Provider,
    pub state: AgentState,
    pub limited: bool,
    pub unread: bool,
}

#[derive(Deserialize)]
struct Record {
    v: u32,
    run: String,
    provider: Provider,
    state: AgentState,
    pid: u32,
    sequence: u64,
    #[serde(default)]
    updated_ms: u64,
    #[serde(default)]
    limited: bool,
}

struct Tracked {
    sequence: u64,
    pid: u32,
    status: AgentStatus,
    native: bool,
    updated_ms: u64,
}

pub struct AgentIntegration {
    directory: TempDir,
    records: BTreeMap<String, Tracked>,
}

impl AgentIntegration {
    pub fn prepare(config: &mut Options, enabled: bool) -> io::Result<Option<Self>> {
        let inherited_path = config
            .env
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let previous_bin = config
            .env
            .get("TERMUST_AGENT_BIN")
            .cloned()
            .or_else(|| std::env::var("TERMUST_AGENT_BIN").ok());
        let paths = std::env::split_paths(&inherited_path)
            .filter(|path| previous_bin.as_ref().is_none_or(|bin| path != Path::new(bin)))
            .collect::<Vec<_>>();
        let original_path =
            std::env::join_paths(paths).map_err(io::Error::other)?.to_string_lossy().into_owned();
        config.env.insert("PATH".into(), original_path.clone());
        for key in [
            "TERMUST_AGENT_DIR",
            "TERMUST_AGENT_BIN",
            "TERMUST_AGENT_RUN",
            "TERMUST_AGENT_PID",
            "TERMUST_AGENT_BRIDGE",
            "TERMUST_AGENT_PYTHON",
            "TERMUST_AGENT_ORIGINAL_PATH",
        ] {
            config.env.insert(key.into(), String::new());
        }
        if !enabled || cfg!(windows) {
            return Ok(None);
        }
        let Some(python) = find_python(&original_path, &toolchain_dirs()) else { return Ok(None) };
        let directory = tempfile::Builder::new().prefix("termust-agents-").tempdir()?;
        let root = directory.path();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        fs::create_dir(root.join("bin"))?;
        fs::create_dir(root.join("proxies"))?;
        private_write(
            &root.join("agent_bridge.py"),
            include_str!("agent_bridge.py").as_bytes(),
            false,
        )?;
        private_write(
            &root.join("agent_opencode.ts"),
            include_str!("agent_opencode.ts").as_bytes(),
            false,
        )?;
        for provider in ["claude", "codex", "opencode", "agy"] {
            let script = format!(
                "#!/bin/sh\nexec {} {} run {provider} \"$@\"\n",
                quote(&python.to_string_lossy()),
                quote(&root.join("agent_bridge.py").to_string_lossy())
            );
            private_write(&root.join("proxies").join(provider), script.as_bytes(), true)?;
            if std::env::split_paths(&original_path).any(|path| executable(&path.join(provider))) {
                #[cfg(unix)]
                std::os::unix::fs::symlink(
                    format!("../proxies/{provider}"),
                    root.join("bin").join(provider),
                )?;
            }
        }
        let bin = root.join("bin").to_string_lossy().into_owned();
        config.env.insert("TERMUST_AGENT_DIR".into(), root.to_string_lossy().into_owned());
        config.env.insert("TERMUST_AGENT_BIN".into(), bin.clone());
        config.env.insert(
            "TERMUST_AGENT_BRIDGE".into(),
            root.join("agent_bridge.py").to_string_lossy().into_owned(),
        );
        config.env.insert("TERMUST_AGENT_PYTHON".into(), python.to_string_lossy().into_owned());
        config.env.insert("TERMUST_AGENT_ORIGINAL_PATH".into(), original_path.clone());
        config.env.insert("PATH".into(), format!("{bin}:{original_path}"));
        Ok(Some(Self { directory, records: BTreeMap::new() }))
    }

    pub fn poll(&mut self, focused: bool) -> bool {
        let before = self.statuses();
        if let Ok(entries) = fs::read_dir(self.directory.path()) {
            for entry in entries.take(256).flatten() {
                let path = entry.path();
                let Some(run) = path.file_stem().and_then(|s| s.to_str()) else { continue };
                if path.extension().is_none_or(|extension| extension != "json")
                    || run.len() > 64
                    || run.len() < 8
                    || !run.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                {
                    continue;
                }
                let Ok(record) = read_record(&path) else { continue };
                if record.v != 1
                    || record.run != run
                    || record.pid < 2
                    || record.pid > i32::MAX as u32
                    || record.sequence == 0
                {
                    continue;
                }
                if let Some(previous) = self.records.get(run) {
                    if previous.pid != record.pid
                        || previous.status.provider != record.provider
                        || previous.sequence >= record.sequence
                    {
                        continue;
                    }
                } else if self.records.len() >= 32 {
                    let remove = self
                        .records
                        .iter()
                        .find(|(_, value)| !process_alive(value.pid))
                        .map(|(key, _)| key.clone());
                    if let Some(remove) = remove {
                        self.records.remove(&remove);
                    } else {
                        continue;
                    }
                }
                let changed =
                    self.records.get(run).is_none_or(|old| old.status.state != record.state);
                let unread = !focused
                    && record.state.attention()
                    && (changed || self.records.get(run).is_some_and(|old| old.status.unread));
                let native = self.records.get(run).is_some_and(|old| old.native);
                self.records.insert(run.to_owned(), Tracked {
                    sequence: record.sequence,
                    pid: record.pid,
                    native,
                    updated_ms: record.updated_ms,
                    status: AgentStatus {
                        provider: record.provider,
                        state: record.state,
                        limited: record.limited && !native,
                        unread,
                    },
                });
            }
        }
        for record in self.records.values_mut() {
            if !process_alive(record.pid)
                && !matches!(
                    record.status.state,
                    AgentState::Done | AgentState::Error | AgentState::Ready
                )
            {
                record.status.state = AgentState::Error;
                record.status.unread = !focused;
            }
            if focused {
                record.status.unread = false;
            }
        }
        before != self.statuses()
    }

    pub fn native(
        &mut self,
        state: &str,
        kind: Option<&str>,
        focused: bool,
        fallback_pid: u32,
    ) -> bool {
        self.poll(focused);
        let before = self.primary();
        let terminal = matches!(state, "done" | "clear" | "idle" | "error");
        let candidate = self
            .records
            .iter()
            .filter(|(_, record)| {
                record.status.provider == Provider::Codex && (process_alive(record.pid) || terminal)
            })
            .max_by_key(|(_, record)| {
                (process_alive(record.pid), record.updated_ms, record.sequence)
            })
            .map(|(key, _)| key.clone());
        if candidate.is_none() && fallback_pid < 2 {
            return false;
        }
        let key = candidate.unwrap_or_else(|| "native".into());
        let previous = self.records.get(&key).map(|record| record.status.state);
        let state = match (state, kind) {
            ("working", _) => AgentState::Working,
            ("done", _) => AgentState::Done,
            ("error", _) => AgentState::Error,
            ("blocked", Some("permission")) => AgentState::Permission,
            ("blocked", Some("question" | "input")) => AgentState::Input,
            ("idle", _) => match previous {
                Some(
                    AgentState::Working
                    | AgentState::Input
                    | AgentState::Permission
                    | AgentState::Done,
                ) => AgentState::Done,
                _ => AgentState::Ready,
            },
            ("clear", _) => match previous {
                Some(AgentState::Done) => AgentState::Done,
                Some(AgentState::Error) => AgentState::Error,
                _ => AgentState::Ready,
            },
            _ => return false,
        };
        let unread = !focused
            && state.attention()
            && (previous != Some(state)
                || self.records.get(&key).is_some_and(|record| record.status.unread));
        let record = self.records.entry(key).or_insert(Tracked {
            sequence: 0,
            pid: fallback_pid,
            native: true,
            updated_ms: 0,
            status: AgentStatus { provider: Provider::Codex, state, limited: false, unread },
        });
        if fallback_pid >= 2 && record.sequence == 0 {
            record.pid = fallback_pid;
        }
        record.native = true;
        record.status = AgentStatus { provider: Provider::Codex, state, limited: false, unread };
        before != self.primary()
    }

    pub fn statuses(&self) -> Vec<AgentStatus> {
        self.records
            .values()
            .filter(|record| process_alive(record.pid) || record.status.unread)
            .map(|record| record.status.clone())
            .collect()
    }

    pub fn primary(&self) -> Option<AgentStatus> {
        self.records
            .values()
            .filter(|record| process_alive(record.pid) || record.status.unread)
            .max_by_key(|record| {
                (
                    process_alive(record.pid),
                    record.status.unread,
                    record.status.state.priority(),
                    record.sequence,
                )
            })
            .map(|record| record.status.clone())
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

const FALLBACK_PYTHON_DIRS: &[&str] =
    if cfg!(target_os = "macos") { &["/opt/homebrew/bin", "/usr/local/bin"] } else { &[] };

fn toolchain_dirs() -> Vec<PathBuf> {
    std::env::var_os("DEVELOPER_DIR")
        .map(PathBuf::from)
        .into_iter()
        .chain(
            ["/Library/Developer/CommandLineTools", "/Applications/Xcode.app/Contents/Developer"]
                .map(PathBuf::from),
        )
        .collect()
}

fn find_python(search_path: &str, toolchains: &[PathBuf]) -> Option<PathBuf> {
    std::env::split_paths(search_path)
        .chain(FALLBACK_PYTHON_DIRS.iter().map(PathBuf::from))
        .map(|path| path.join("python3"))
        .find(|path| executable(path) && !installer_stub(path, toolchains))
}

fn installer_stub(path: &Path, toolchains: &[PathBuf]) -> bool {
    cfg!(target_os = "macos")
        && path == Path::new("/usr/bin/python3")
        && !toolchains.iter().any(|dir| dir.join("usr/bin/python3").is_file())
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

fn private_write(path: &Path, data: &[u8], executable: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    let mut file = options.open(path)?;
    file.write_all(data)
}

fn read_record(path: &Path) -> io::Result<Record> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file: File = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    serde_json::from_slice(&bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn python_lookup_skips_the_macos_installer_stub_without_developer_tools() {
        use std::os::unix::fs::PermissionsExt;
        let custom = tempfile::tempdir().unwrap();
        let python = custom.path().join("python3");
        fs::write(&python, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let toolchain = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        let search = format!("/usr/bin:{}", custom.path().display());
        let stub = Path::new("/usr/bin/python3");
        if executable(stub) {
            let expected = if cfg!(target_os = "macos") { python.clone() } else { stub.into() };
            assert_eq!(find_python(&search, &[empty.path().into()]), Some(expected));
        }
        fs::create_dir_all(toolchain.path().join("usr/bin")).unwrap();
        fs::write(toolchain.path().join("usr/bin/python3"), "").unwrap();
        assert!(!installer_stub(stub, &[toolchain.path().into()]));
        assert_eq!(installer_stub(stub, &[empty.path().into()]), cfg!(target_os = "macos"));
        assert!(!installer_stub(&python, &[empty.path().into()]));
        assert_eq!(find_python(custom.path().to_str().unwrap(), &[]), Some(python));
    }

    fn integration() -> AgentIntegration {
        AgentIntegration { directory: tempfile::tempdir().unwrap(), records: BTreeMap::new() }
    }

    fn record(
        integration: &AgentIntegration,
        run: &str,
        state: &str,
        sequence: u64,
    ) -> std::path::PathBuf {
        let path = integration.directory.path().join(format!("{run}.json"));
        let _ = fs::remove_file(&path);
        let json = serde_json::json!({"v":1,"run":run,"provider":"claude","state":state,"pid":std::process::id(),"sequence":sequence,"limited":false});
        private_write(&path, &serde_json::to_vec(&json).unwrap(), false).unwrap();
        path
    }

    #[test]
    fn disabled_integration_detaches_inherited_pane_environment() {
        let mut config = Options::default();
        config.env.insert("PATH".into(), "/old/private/bin:/usr/bin".into());
        config.env.insert("TERMUST_AGENT_BIN".into(), "/old/private/bin".into());
        config.env.insert("TERMUST_AGENT_DIR".into(), "/old/private".into());
        config.env.insert("TERMUST_AGENT_RUN".into(), "old-run".into());
        assert!(AgentIntegration::prepare(&mut config, false).unwrap().is_none());
        assert_eq!(config.env["PATH"], "/usr/bin");
        assert!(config.env["TERMUST_AGENT_DIR"].is_empty());
        assert!(config.env["TERMUST_AGENT_RUN"].is_empty());
    }

    #[test]
    fn canceled_sessions_stay_ready_and_new_work_wins_over_old_completion() {
        let mut bridge = integration();
        let path = record(&bridge, "abcdef01", "done", 1);
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        data["pid"] = serde_json::json!(i32::MAX);
        fs::write(path, serde_json::to_vec(&data).unwrap()).unwrap();
        record(&bridge, "abcdef02", "working", 1);
        bridge.poll(false);
        assert_eq!(bridge.primary().unwrap().state, AgentState::Working);
        let path = record(&bridge, "abcdef03", "ready", 1);
        data["run"] = serde_json::json!("abcdef03");
        data["state"] = serde_json::json!("ready");
        fs::write(path, serde_json::to_vec(&data).unwrap()).unwrap();
        bridge.poll(false);
        assert!(bridge.records["abcdef03"].status.state == AgentState::Ready);
        assert!(!bridge.records["abcdef03"].status.unread);
    }

    #[test]
    fn late_native_completion_does_not_create_an_immortal_agent_or_hide_new_work() {
        let mut bridge = integration();
        let path = record(&bridge, "abcdef01", "done", 2);
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        data["provider"] = serde_json::json!("codex");
        data["pid"] = serde_json::json!(i32::MAX);
        fs::write(path, serde_json::to_vec(&data).unwrap()).unwrap();
        bridge.poll(false);
        bridge.native("done", None, false, 0);
        assert!(!bridge.records.contains_key("native"));
        assert_eq!(bridge.primary().unwrap().provider, Provider::Codex);
        bridge.poll(true);
        assert!(bridge.primary().is_none());
        bridge.native("clear", None, false, 0);
        assert!(bridge.primary().is_none());
        record(&bridge, "abcdef02", "working", 1);
        bridge.poll(false);
        assert_eq!(bridge.primary().unwrap().provider, Provider::Claude);
        assert_eq!(bridge.primary().unwrap().state, AgentState::Working);
    }

    #[test]
    fn native_codex_question_permission_and_done_need_no_hooks() {
        let mut bridge = integration();
        assert!(bridge.native("idle", None, true, std::process::id()));
        assert_eq!(bridge.primary().unwrap().state, AgentState::Ready);
        bridge.native("working", None, true, std::process::id());
        bridge.native("blocked", Some("question"), false, std::process::id());
        assert_eq!(bridge.primary().unwrap().state, AgentState::Input);
        assert!(bridge.primary().unwrap().unread);
        bridge.native("blocked", Some("permission"), false, std::process::id());
        assert_eq!(bridge.primary().unwrap().state, AgentState::Permission);
        bridge.native("done", None, false, std::process::id());
        bridge.native("clear", None, false, std::process::id());
        assert_eq!(bridge.primary().unwrap().state, AgentState::Done);
        assert!(!bridge.primary().unwrap().limited);
    }

    #[test]
    fn attention_acknowledges_only_the_focused_pane_and_rejects_old_events() {
        let mut first = integration();
        let mut second = integration();
        record(&first, "abcdef01", "permission", 2);
        record(&second, "abcdef01", "input", 2);
        assert!(first.poll(false));
        second.poll(false);
        assert!(first.primary().unwrap().unread);
        first.poll(true);
        assert!(!first.primary().unwrap().unread);
        assert!(second.primary().unwrap().unread);
        record(&first, "abcdef01", "working", 1);
        first.poll(false);
        assert_eq!(first.primary().unwrap().state, AgentState::Permission);
        record(&first, "abcdef01", "working", 3);
        first.poll(false);
        assert_eq!(first.primary().unwrap().state, AgentState::Working);
        assert!(!first.primary().unwrap().unread);
    }

    #[test]
    fn permission_and_questions_win_over_background_work_and_completion() {
        let mut bridge = integration();
        record(&bridge, "abcdef01", "working", 1);
        record(&bridge, "abcdef02", "done", 1);
        record(&bridge, "abcdef03", "input", 1);
        record(&bridge, "abcdef04", "permission", 1);
        bridge.poll(false);
        assert_eq!(bridge.primary().unwrap().state, AgentState::Permission);
        assert_eq!(bridge.statuses().len(), 4);
    }

    #[test]
    fn malformed_oversized_and_linked_records_are_ignored() {
        let mut bridge = integration();
        let path = bridge.directory.path().join("abcdef01.json");
        private_write(&path, b"not-json", false).unwrap();
        bridge.poll(false);
        assert!(bridge.primary().is_none());
        fs::write(&path, vec![b' '; 4097]).unwrap();
        bridge.poll(false);
        assert!(bridge.primary().is_none());
        let path = record(&bridge, "abcdef01", "permission", 1);
        let linked = bridge.directory.path().join("abcdef02.json");
        fs::hard_link(path, linked).unwrap();
        bridge.poll(false);
        assert!(bridge.primary().is_none());
    }
}

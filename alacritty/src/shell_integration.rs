use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use alacritty_terminal::tty::{Options, Shell};
use tempfile::TempDir;

use crate::session::CommandRecord;

const RECORD_COUNT: usize = 20;
const MAX_RECORD_BYTES: usize = 16 * 1024;

pub struct ShellIntegration {
    directory: TempDir,
    original_shell: Shell,
}

#[derive(Default)]
pub struct ShellSnapshot {
    pub working_directory: Option<PathBuf>,
    pub commands: Vec<CommandRecord>,
    pub ready: bool,
    pub completion: Option<(u64, i32)>,
}

impl ShellIntegration {
    #[cfg(unix)]
    pub fn prepare(config: &mut Options) -> io::Result<Option<Self>> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        #[cfg(target_os = "macos")]
        if config.shell.is_none() {
            return Ok(None);
        }

        let program = match &config.shell {
            Some(shell) => shell.program().to_owned(),
            None => match std::env::var("SHELL") {
                Ok(shell) if !shell.is_empty() => shell,
                _ => return Ok(None),
            },
        };
        let shell_name = Path::new(&program).file_name().and_then(|name| name.to_str());
        let original_args = config.shell.as_ref().map_or(&[][..], Shell::args);
        let supported = match shell_name {
            Some("bash") => original_args
                .iter()
                .all(|arg| matches!(arg.as_str(), "-i" | "--noprofile" | "--norc")),
            Some("fish") => {
                original_args.iter().all(|arg| matches!(arg.as_str(), "-i" | "--interactive"))
            },
            Some("zsh") => original_args.iter().all(|arg| arg == "-i"),
            _ => false,
        };
        if !supported {
            return Ok(None);
        }
        let original_shell = Shell::new(program.clone(), original_args.to_vec());
        let original_zdotdir = if shell_name == Some("zsh") {
            match config.env.get("ZDOTDIR") {
                Some(value) => Some(value.clone()),
                None => match std::env::var("ZDOTDIR") {
                    Ok(value) => Some(value),
                    Err(std::env::VarError::NotPresent) => None,
                    Err(std::env::VarError::NotUnicode(_)) => return Ok(None),
                },
            }
        } else {
            None
        };

        let directory = tempfile::Builder::new().prefix("alacritty-shell-").tempdir()?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        for name in [
            "sequence",
            "execution-sequence",
            "completion",
            "history",
            "cwd",
            "ready",
            "replay-marker",
        ] {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.path().join(name))?;
            if matches!(name, "sequence" | "execution-sequence") {
                file.write_all(b"0\n")?;
            }
        }
        for index in 0..RECORD_COUNT {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.path().join(format!("event{index}")))?;
        }

        let script_name = match shell_name {
            Some("bash") => "bashrc",
            Some("zsh") => ".zshrc",
            _ => "integration.fish",
        };
        let script_path = directory.path().join(script_name);
        let quoted_directory = shell_quote(&directory.path().to_string_lossy());
        let script = match shell_name {
            Some("bash") => {
                let source_user_rc = !original_args.iter().any(|arg| arg == "--norc");
                bash_script(&quoted_directory, source_user_rc)
            },
            Some("zsh") => ZSH_RC.to_owned(),
            _ => fish_script(&quoted_directory),
        };
        let mut script_file =
            OpenOptions::new().write(true).create_new(true).mode(0o600).open(&script_path)?;
        script_file.write_all(script.as_bytes())?;
        if shell_name == Some("zsh") {
            let script = zsh_env_script(&quoted_directory, original_zdotdir.as_deref());
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.path().join(".zshenv"))?;
            file.write_all(script.as_bytes())?;
        }

        let args = match shell_name {
            Some("bash") => vec![
                "--noprofile".into(),
                "--rcfile".into(),
                script_path.to_string_lossy().into_owned(),
                "-i".into(),
            ],
            Some("zsh") => vec!["-i".into()],
            _ => vec![
                "--init-command".into(),
                format!("source {}", shell_quote(&script_path.to_string_lossy())),
                "--interactive".into(),
            ],
        };
        if shell_name == Some("zsh") {
            config.env.insert("ZDOTDIR".into(), directory.path().to_string_lossy().into_owned());
        }
        config.shell = Some(Shell::new(program, args));
        Ok(Some(Self { directory, original_shell }))
    }

    #[cfg(not(unix))]
    pub fn prepare(_config: &mut Options) -> io::Result<Option<Self>> {
        Ok(None)
    }

    pub fn original_shell(&self) -> &Shell {
        &self.original_shell
    }

    pub fn replay(&self, record: &CommandRecord) -> io::Result<String> {
        let directory = record.working_directory.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Replay directory is not valid UTF-8")
        })?;
        if !record.working_directory.is_absolute()
            || record.command.is_empty()
            || record.command.contains('\0')
            || record.command.len() + directory.len() + 24 > MAX_RECORD_BYTES
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid replay command"));
        }
        let quoted_directory = shell_quote(directory);
        let shell_name =
            Path::new(self.original_shell.program()).file_name().and_then(|name| name.to_str());
        let script = if shell_name == Some("fish") {
            format!("if builtin cd -- {quoted_directory}\n{}\nend\n", record.command)
        } else {
            format!("if builtin cd -- {quoted_directory}; then\n{}\nfi\n", record.command)
        };
        let mut file =
            tempfile::Builder::new().prefix("replay-").tempfile_in(self.directory.path())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(script.as_bytes())?;
        file.flush()?;
        let (_file, path) = file.keep().map_err(|error| error.error)?;
        self.append_record(record)?;
        write_owned_file(&self.directory.path().join("replay-marker"), b"1\n")?;
        write_owned_file(&self.directory.path().join("ready"), b"0\n")?;
        Ok(format!("builtin source {}\r", shell_quote(&path.to_string_lossy())))
    }

    fn append_record(&self, record: &CommandRecord) -> io::Result<()> {
        let bytes = read_owned_file(&self.directory.path().join("sequence"))?;
        let sequence = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid command sequence")
            })?;
        let mut bytes = sequence.to_string().into_bytes();
        bytes.push(0);
        bytes.extend_from_slice(record.working_directory.as_os_str().as_encoded_bytes());
        bytes.push(0);
        bytes.extend_from_slice(record.command.as_bytes());
        bytes.push(0);
        write_owned_file(
            &self.directory.path().join(format!("event{}", sequence % RECORD_COUNT as u64)),
            &bytes,
        )?;
        write_owned_file(
            &self.directory.path().join("sequence"),
            format!("{sequence}\n").as_bytes(),
        )
    }

    pub fn snapshot(&self) -> io::Result<ShellSnapshot> {
        let cwd = read_owned_file(&self.directory.path().join("cwd"))?;
        let working_directory = parse_cwd(&cwd);
        let mut records = Vec::new();
        for index in 0..RECORD_COUNT {
            let bytes = read_owned_file(&self.directory.path().join(format!("event{index}")))?;
            if let Some((sequence, command)) = parse_record(&bytes) {
                if sequence % RECORD_COUNT as u64 == index as u64 {
                    records.push((sequence, command));
                }
            }
        }
        records.sort_unstable_by_key(|record| record.0);
        records.dedup_by_key(|record| record.0);
        let commands = records.into_iter().map(|record| record.1).collect();
        let ready = read_owned_file(&self.directory.path().join("ready"))? == b"1\n";
        let completion =
            parse_completion(&read_owned_file(&self.directory.path().join("completion"))?);
        Ok(ShellSnapshot { working_directory, commands, ready, completion })
    }
}

fn read_owned_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES as u64 {
        return Ok(Vec::new());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Ok(Vec::new());
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECORD_BYTES {
        bytes.clear();
    }
    Ok(bytes)
}

fn write_owned_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Command metadata exceeds its size limit",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Command metadata is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Command metadata ownership changed",
            ));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.set_len(0)?;
    file.write_all(bytes)
}

fn parse_completion(bytes: &[u8]) -> Option<(u64, i32)> {
    if bytes.len() > 25 || !bytes.ends_with(b"\n") {
        return None;
    }
    let mut fields = bytes[..bytes.len().checked_sub(1)?].split(|byte| *byte == b' ');
    let sequence = fields.next()?;
    let status = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    if sequence.is_empty()
        || status.is_empty()
        || !sequence.iter().all(u8::is_ascii_digit)
        || !status.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let sequence = std::str::from_utf8(sequence).ok()?.parse::<u64>().ok()?;
    let status = std::str::from_utf8(status).ok()?.parse::<i32>().ok()?;
    (sequence != 0 && (0..=255).contains(&status)).then_some((sequence, status))
}

fn parse_record(bytes: &[u8]) -> Option<(u64, CommandRecord)> {
    if bytes.len() > MAX_RECORD_BYTES || !bytes.ends_with(b"\0") {
        return None;
    }
    let mut fields = bytes[..bytes.len().checked_sub(1)?].split(|byte| *byte == 0);
    let sequence = std::str::from_utf8(fields.next()?).ok()?.parse::<u64>().ok()?;
    let working_directory = bytes_path(fields.next()?)?;
    let command = std::str::from_utf8(fields.next()?).ok()?.to_owned();
    if sequence == 0 || command.is_empty() || fields.next().is_some() {
        return None;
    }
    Some((sequence, CommandRecord { command, working_directory }))
}

fn parse_cwd(bytes: &[u8]) -> Option<PathBuf> {
    let bytes = bytes.strip_suffix(b"\0")?;
    if bytes.len() > MAX_RECORD_BYTES || bytes.contains(&0) {
        return None;
    }
    bytes_path(bytes)
}

fn bytes_path(bytes: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    };
    #[cfg(not(unix))]
    let path = PathBuf::from(std::str::from_utf8(bytes).ok()?);
    path.is_absolute().then_some(path)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn bash_script(directory: &str, source_user_rc: bool) -> String {
    let source_rc = if source_user_rc {
        "if [[ -f \"$HOME/.bashrc\" ]]; then builtin source \"$HOME/.bashrc\"; fi\n"
    } else {
        ""
    };
    format!("{source_rc}__alacritty_session_dir={directory}\n{BASH_HOOKS}")
}

const BASH_HOOKS: &str = r#"
__alacritty_session_history() {
    HISTTIMEFORMAT='' builtin history 1
}
__alacritty_session_preexec() {
    builtin printf '0\n' > "$__alacritty_session_dir/ready"
    local __alacritty_execution
    IFS= builtin read -r __alacritty_execution < "$__alacritty_session_dir/execution-sequence" || __alacritty_execution=0
    [[ $__alacritty_execution =~ ^[0-9]{1,18}$ ]] || __alacritty_execution=0
    builtin printf '%s\n' "$((__alacritty_execution + 1))" > "$__alacritty_session_dir/execution-sequence"
    local __alacritty_line __alacritty_previous __alacritty_command
    local __alacritty_seq __alacritty_slot __alacritty_replay LC_ALL=C
    IFS= builtin read -r __alacritty_replay < "$__alacritty_session_dir/replay-marker" || :
    if [[ $__alacritty_replay == 1 ]]; then
        builtin printf '0\n' > "$__alacritty_session_dir/replay-marker"
        builtin printf '0\n' > "$__alacritty_session_dir/ready"
        return 0
    fi
    [[ :$SHELLOPTS: == *:history:* ]] || return 0
    __alacritty_line=$(__alacritty_session_history)
    IFS= builtin read -r -d '' __alacritty_previous < "$__alacritty_session_dir/history" || :
    [[ $__alacritty_line != "$__alacritty_previous" ]] || return 0
    [[ $__alacritty_line =~ ^[[:space:]]*[0-9]+[[:space:]]+(.*)$ ]] || return 0
    __alacritty_command=${BASH_REMATCH[1]}
    [[ -n $__alacritty_command ]] || return 0
    (( ${#__alacritty_command} + ${#PWD} + 24 <= 16384 )) || return 0
    IFS= builtin read -r __alacritty_seq < "$__alacritty_session_dir/sequence" || __alacritty_seq=0
    [[ $__alacritty_seq =~ ^[0-9]{1,18}$ ]] || __alacritty_seq=0
    __alacritty_seq=$((__alacritty_seq + 1))
    __alacritty_slot=$((__alacritty_seq % 20))
    builtin printf '%s\n' "$__alacritty_seq" > "$__alacritty_session_dir/sequence"
    builtin printf '%s\0%s\0%s\0' "$__alacritty_seq" "$PWD" "$__alacritty_command" > "$__alacritty_session_dir/event$__alacritty_slot"
    builtin printf '0\n' > "$__alacritty_session_dir/ready"
    return 0
}
__alacritty_session_capture_status() {
    __alacritty_session_exit_status=$?
    return "$__alacritty_session_exit_status"
}
__alacritty_session_prompt() {
    local __alacritty_status=$? __alacritty_execution __alacritty_ready
    IFS= builtin read -r __alacritty_ready < "$__alacritty_session_dir/ready" || :
    IFS= builtin read -r __alacritty_execution < "$__alacritty_session_dir/execution-sequence" || :
    if [[ $__alacritty_ready == 0 && $__alacritty_execution =~ ^[0-9]{1,18}$ ]]; then
        builtin printf '%s %s\n' "$__alacritty_execution" "$__alacritty_session_exit_status" > "$__alacritty_session_dir/completion"
    fi
    builtin printf '%s\0' "$PWD" > "$__alacritty_session_dir/cwd"
    builtin printf '%s\0' "$(__alacritty_session_history)" > "$__alacritty_session_dir/history"
    builtin printf '1\n' > "$__alacritty_session_dir/ready"
    return "$__alacritty_status"
}
PS0='$(__alacritty_session_preexec)'${PS0-}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
    PROMPT_COMMAND=(__alacritty_session_capture_status "${PROMPT_COMMAND[@]}" __alacritty_session_prompt)
else
    PROMPT_COMMAND=(__alacritty_session_capture_status ${PROMPT_COMMAND+"$PROMPT_COMMAND"} __alacritty_session_prompt)
fi
"#;

fn fish_script(directory: &str) -> String {
    format!("set -g __alacritty_session_dir {directory}\n{FISH_HOOKS}")
}

fn zsh_env_script(directory: &str, original_zdotdir: Option<&str>) -> String {
    let restore = match original_zdotdir {
        Some(value) => format!("ZDOTDIR={}\nexport ZDOTDIR", shell_quote(value)),
        None => "unset ZDOTDIR".to_owned(),
    };
    format!("__alacritty_session_dir={directory}\n{restore}\n{ZSH_ENV}")
}

const ZSH_ENV: &str = r#"
if [[ -f "${ZDOTDIR-$HOME}/.zshenv" ]]; then
    builtin source "${ZDOTDIR-$HOME}/.zshenv"
fi
[[ ${parameters[ZDOTDIR]-} != *readonly* ]] || return 0
__alacritty_zdotdir_set=${+ZDOTDIR}
__alacritty_zdotdir_value=${ZDOTDIR-}
__alacritty_zdotdir_exported=0
[[ ${parameters[ZDOTDIR]-} == *export* ]] && __alacritty_zdotdir_exported=1
ZDOTDIR=$__alacritty_session_dir
"#;

const ZSH_RC: &str = r#"
unset ZDOTDIR
if [[ $__alacritty_zdotdir_set == 1 ]]; then
    ZDOTDIR=$__alacritty_zdotdir_value
    [[ $__alacritty_zdotdir_exported == 1 ]] && export ZDOTDIR
fi
unset __alacritty_zdotdir_set __alacritty_zdotdir_value __alacritty_zdotdir_exported
if [[ -f "${ZDOTDIR-$HOME}/.zshrc" ]]; then
    builtin source "${ZDOTDIR-$HOME}/.zshrc"
fi
__alacritty_session_preexec() {
    builtin printf '0\n' > "$__alacritty_session_dir/ready"
    local __alacritty_execution
    IFS= builtin read -r __alacritty_execution < "$__alacritty_session_dir/execution-sequence" || __alacritty_execution=0
    [[ $__alacritty_execution =~ '^[0-9]{1,18}$' ]] || __alacritty_execution=0
    builtin printf '%s\n' "$((__alacritty_execution + 1))" > "$__alacritty_session_dir/execution-sequence"
    local __alacritty_command=$1
    local __alacritty_seq __alacritty_slot __alacritty_replay LC_ALL=C
    IFS= builtin read -r __alacritty_replay < "$__alacritty_session_dir/replay-marker" || :
    if [[ $__alacritty_replay == 1 ]]; then
        builtin printf '0\n' > "$__alacritty_session_dir/replay-marker"
        builtin printf '0\n' > "$__alacritty_session_dir/ready"
        return 0
    fi
    [[ -n $__alacritty_command ]] || return 0
    [[ -o HIST_IGNORE_SPACE && $__alacritty_command == [[:space:]]* ]] && return 0
    emulate -L zsh
    (( ${#__alacritty_command} + ${#PWD} + 24 <= 16384 )) || return 0
    IFS= builtin read -r __alacritty_seq < "$__alacritty_session_dir/sequence" || __alacritty_seq=0
    [[ $__alacritty_seq =~ '^[0-9]{1,18}$' ]] || __alacritty_seq=0
    __alacritty_seq=$((__alacritty_seq + 1))
    __alacritty_slot=$((__alacritty_seq % 20))
    builtin printf '%s\n' "$__alacritty_seq" > "$__alacritty_session_dir/sequence"
    builtin printf '%s\0%s\0%s\0' "$__alacritty_seq" "$PWD" "$__alacritty_command" > "$__alacritty_session_dir/event$__alacritty_slot"
    builtin printf '0\n' > "$__alacritty_session_dir/ready"
    return 0
}
__alacritty_session_prompt() {
    local __alacritty_status=$? __alacritty_execution __alacritty_ready
    emulate -L zsh
    IFS= builtin read -r __alacritty_ready < "$__alacritty_session_dir/ready" || :
    IFS= builtin read -r __alacritty_execution < "$__alacritty_session_dir/execution-sequence" || :
    if [[ $__alacritty_ready == 0 && $__alacritty_execution =~ '^[0-9]{1,18}$' ]]; then
        builtin printf '%s %s\n' "$__alacritty_execution" "$__alacritty_status" > "$__alacritty_session_dir/completion"
    fi
    builtin printf '%s\0' "$PWD" > "$__alacritty_session_dir/cwd"
    builtin printf '1\n' > "$__alacritty_session_dir/ready"
    return 0
}
autoload -Uz add-zsh-hook
add-zsh-hook preexec __alacritty_session_preexec
precmd_functions=(__alacritty_session_prompt ${precmd_functions:#__alacritty_session_prompt})
"#;

const FISH_HOOKS: &str = r#"
function __alacritty_session_preexec --on-event fish_preexec
    builtin printf '0\n' > "$__alacritty_session_dir/ready"
    read -l execution < "$__alacritty_session_dir/execution-sequence"
    string match -qr '^[0-9]{1,15}$' -- "$execution"; or set execution 0
    printf '%s\n' (math "$execution + 1") > "$__alacritty_session_dir/execution-sequence"
    read -l replay < "$__alacritty_session_dir/replay-marker"
    if test "$replay" = 1
        builtin printf '0\n' > "$__alacritty_session_dir/replay-marker"
        builtin printf '0\n' > "$__alacritty_session_dir/ready"
        return
    end
    set -l commandline $argv[1]
    string match -qr '^[[:space:]]' -- "$commandline"; and return
    test (string length -- "$commandline") -le 3000; or return
    test (string length -- "$PWD") -le 1000; or return
    read -l sequence < "$__alacritty_session_dir/sequence"
    string match -qr '^[0-9]{1,15}$' -- "$sequence"; or set sequence 0
    set sequence (math "$sequence + 1")
    set -l slot (math "$sequence % 20")
    printf '%s\n' "$sequence" > "$__alacritty_session_dir/sequence"
    printf '%s\0%s\0%s\0' "$sequence" "$PWD" "$commandline" > "$__alacritty_session_dir/event$slot"
    printf '0\n' > "$__alacritty_session_dir/ready"
end
function __alacritty_session_complete --on-event fish_postexec
    set -l exit_status $status
    read -l execution < "$__alacritty_session_dir/execution-sequence"
    if string match -qr '^[0-9]{1,15}$' -- "$execution"
        printf '%s %s\n' "$execution" "$exit_status" > "$__alacritty_session_dir/completion"
    end
    return $exit_status
end
function __alacritty_session_prompt --on-event fish_prompt
    printf '%s\0' "$PWD" > "$__alacritty_session_dir/cwd"
    printf '1\n' > "$__alacritty_session_dir/ready"
end
"#;

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn parses_multiline_commands_and_execution_directory() {
        let (sequence, record) =
            parse_record(b"42\0/tmp/project\0printf 'first\nsecond'\0").unwrap();
        assert_eq!(sequence, 42);
        assert_eq!(record.working_directory, PathBuf::from("/tmp/project"));
        assert_eq!(record.command, "printf 'first\nsecond'");
    }

    #[test]
    fn rejects_incomplete_invalid_and_oversized_records() {
        for bytes in [
            b"1\0/tmp\0echo secret".as_slice(),
            b"0\0/tmp\0echo hello\0".as_slice(),
            b"1\0relative\0echo hello\0".as_slice(),
            b"1\0/tmp\0\0".as_slice(),
            b"1\0/tmp\0echo hello\0unexpected\0".as_slice(),
            b"1\0/tmp\0\xff\0".as_slice(),
        ] {
            assert!(parse_record(bytes).is_none());
        }
        let oversized = format!("1\0/tmp\0{}\0", "x".repeat(MAX_RECORD_BYTES));
        assert!(parse_record(oversized.as_bytes()).is_none());
    }

    #[test]
    fn completion_requires_a_complete_bounded_sequence_and_exit_status() {
        assert_eq!(parse_completion(b"1 0\n"), Some((1, 0)));
        assert_eq!(parse_completion(b"18446744073709551615 255\n"), Some((u64::MAX, 255)));
        for bytes in [
            &b""[..],
            &b"0 0\n"[..],
            &b"1 0"[..],
            &b"1 -1\n"[..],
            &b"1 256\n"[..],
            &b"1 +1\n"[..],
            &b"1  0\n"[..],
            &b"1 0\n\n"[..],
            &b"1 0 extra\n"[..],
            &b"18446744073709551616 0\n"[..],
            &b"1 \xff\n"[..],
        ] {
            assert_eq!(parse_completion(bytes), None, "Unexpected completion: {bytes:?}");
        }
        assert_eq!(parse_completion(&[b'9'; 26]), None);
    }

    #[test]
    fn rejects_incomplete_or_relative_directories() {
        assert_eq!(parse_cwd(b"/tmp/project\0"), Some(PathBuf::from("/tmp/project")));
        assert!(parse_cwd(b"/tmp/project").is_none());
        assert!(parse_cwd(b"relative\0").is_none());
        assert!(parse_cwd(b"/tmp\0extra\0").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn leaves_custom_commands_and_unsupported_shells_unchanged() {
        for (program, args) in [
            ("/bin/bash", vec!["-c", "echo hello"]),
            ("/bin/bash", vec!["script.sh"]),
            ("/bin/bash", vec!["-l"]),
            ("/bin/zsh", vec!["-l"]),
            ("/bin/zsh", vec!["-c", "echo hello"]),
            ("/bin/sh", vec![]),
            ("/usr/bin/python", vec![]),
        ] {
            let mut options = Options {
                shell: Some(Shell::new(
                    program.into(),
                    args.into_iter().map(str::to_owned).collect(),
                )),
                ..Options::default()
            };
            let original = options.clone();
            assert!(ShellIntegration::prepare(&mut options).unwrap().is_none());
            assert_eq!(options, original);
        }
    }

    #[cfg(unix)]
    #[test]
    fn zsh_wraps_private_startup_and_restores_the_original_config_directory() {
        let mut options = Options {
            shell: Some(Shell::new("/bin/zsh".into(), vec!["-i".into()])),
            ..Options::default()
        };
        let original_zdotdir = "/tmp/user's zsh config";
        options.env.insert("ZDOTDIR".into(), original_zdotdir.into());
        let integration = ShellIntegration::prepare(&mut options).unwrap().unwrap();
        let directory = integration.directory.path();
        assert_eq!(options.env.get("ZDOTDIR").unwrap(), &directory.to_string_lossy());
        assert_eq!(options.shell.as_ref().unwrap().args(), ["-i"]);
        let environment = fs::read_to_string(directory.join(".zshenv")).unwrap();
        let rc = fs::read_to_string(directory.join(".zshrc")).unwrap();
        assert!(
            environment
                .contains(&format!("ZDOTDIR={}\nexport ZDOTDIR", shell_quote(original_zdotdir)))
        );
        assert!(environment.contains("builtin source \"${ZDOTDIR-$HOME}/.zshenv\""));
        assert!(environment.contains("__alacritty_zdotdir_exported"));
        assert!(rc.starts_with("\nunset ZDOTDIR\n"));
        assert!(rc.contains("builtin source \"${ZDOTDIR-$HOME}/.zshrc\""));
        assert!(rc.contains("add-zsh-hook preexec __alacritty_session_preexec"));
        assert!(rc.contains("precmd_functions=(__alacritty_session_prompt"));
        let absent = zsh_env_script("'/tmp/private'", None);
        assert!(absent.contains("unset ZDOTDIR\n"));
        assert!(!absent.contains("export ZDOTDIR\n"));
    }

    #[cfg(unix)]
    #[test]
    fn snapshots_only_complete_records_from_the_private_ring() {
        use std::os::unix::fs::PermissionsExt;

        let mut options = Options {
            shell: Some(Shell::new("/bin/bash".into(), vec![
                "--noprofile".into(),
                "--norc".into(),
            ])),
            ..Options::default()
        };
        let integration = ShellIntegration::prepare(&mut options).unwrap().unwrap();
        let directory = integration.directory.path();
        assert_eq!(fs::metadata(directory).unwrap().permissions().mode() & 0o777, 0o700);
        for sequence in 1..=25 {
            let record = format!("{sequence}\0/tmp/project\0echo {sequence}\0");
            fs::write(directory.join(format!("event{}", sequence % RECORD_COUNT)), record).unwrap();
        }
        fs::write(directory.join("cwd"), b"/tmp/project\0").unwrap();
        fs::write(directory.join("ready"), b"1\n").unwrap();
        let snapshot = integration.snapshot().unwrap();
        assert!(snapshot.ready);
        assert_eq!(snapshot.working_directory, Some(PathBuf::from("/tmp/project")));
        assert_eq!(snapshot.commands.len(), RECORD_COUNT);
        assert_eq!(snapshot.commands.first().unwrap().command, "echo 6");
        assert_eq!(snapshot.commands.last().unwrap().command, "echo 25");
        assert_eq!(
            fs::metadata(directory.join("event1")).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn does_not_read_symlinks_or_oversized_metadata() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, b"outside content").unwrap();
        let link = directory.path().join("link");
        symlink(&outside, &link).unwrap();
        assert!(read_owned_file(&link).is_err());
        fs::write(&outside, vec![b'x'; MAX_RECORD_BYTES + 1]).unwrap();
        assert!(read_owned_file(&outside).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn replay_preserves_command_bytes_and_only_sends_the_private_source_path() {
        use std::os::unix::fs::PermissionsExt;

        for program in ["/bin/bash", "/bin/zsh", "/bin/fish"] {
            let original = Shell::new(program.into(), vec!["-i".into()]);
            let mut options = Options { shell: Some(original.clone()), ..Options::default() };
            let integration = ShellIntegration::prepare(&mut options).unwrap().unwrap();
            assert_eq!(integration.original_shell(), &original);
            let record = CommandRecord {
                command: "printf '%s' 'first\trow\nsecond\u{1b}X'\n".into(),
                working_directory: PathBuf::from("/tmp/user's working directory"),
            };
            let payload = integration.replay(&record).unwrap();
            let scripts: Vec<_> = fs::read_dir(integration.directory.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name().unwrap().to_string_lossy().starts_with("replay-")
                        && path.file_name().unwrap() != "replay-marker"
                })
                .collect();
            assert_eq!(scripts.len(), 1);
            let script = fs::read_to_string(&scripts[0]).unwrap();
            assert!(script.contains(&record.command));
            assert!(script.starts_with("if builtin cd -- '/tmp/user'\\''s working directory'"));
            assert!(script.ends_with(if program.ends_with("fish") { "\nend\n" } else { "\nfi\n" }));
            assert_eq!(
                payload,
                format!("builtin source {}\r", shell_quote(&scripts[0].to_string_lossy()))
            );
            assert!(!payload.contains('\t'));
            assert!(!payload.contains('\u{1b}'));
            assert!(!payload.contains("first"));
            assert_eq!(fs::metadata(&scripts[0]).unwrap().permissions().mode() & 0o777, 0o600);
            let snapshot = integration.snapshot().unwrap();
            assert!(!snapshot.ready);
            assert_eq!(snapshot.commands.last().unwrap().command, record.command);
            assert_eq!(
                snapshot.commands.last().unwrap().working_directory,
                record.working_directory
            );
            assert_eq!(
                fs::read(integration.directory.path().join("replay-marker")).unwrap(),
                b"1\n"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn writing_metadata_rejects_symlinks_and_hardlinks_without_truncating_targets() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        fs::write(&target, b"untouched").unwrap();
        let link = directory.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(write_owned_file(&link, b"replacement").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
        let hardlink = directory.path().join("hardlink");
        fs::hard_link(&target, &hardlink).unwrap();
        assert!(write_owned_file(&hardlink, b"replacement").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
    }

    #[cfg(unix)]
    #[test]
    fn bash_reports_executed_commands_without_password_input() {
        use std::fs::File;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::process::{Child, Command, Stdio};
        use std::time::{Duration, Instant};

        struct PtyShell(Child);
        impl Drop for PtyShell {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        fn wait_for(master: &mut File, pending: &mut Vec<u8>, marker: &[u8]) {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                if let Some(index) =
                    pending.windows(marker.len()).position(|window| window == marker)
                {
                    pending.drain(..index + marker.len());
                    return;
                }
                assert!(Instant::now() < deadline, "Bash did not reach the expected prompt");
                let mut bytes = [0; 4096];
                match master.read(&mut bytes) {
                    Ok(length) => pending.extend_from_slice(&bytes[..length]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => (),
                    Err(error) => panic!("PTY read failed: {error}"),
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn wait_for_busy(integration: &ShellIntegration) {
            let deadline = Instant::now() + Duration::from_millis(400);
            while integration.snapshot().unwrap().ready {
                assert!(
                    Instant::now() < deadline,
                    "Running command did not clear prompt readiness"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        let Some(bash) =
            ["/bin/bash", "/usr/bin/bash"].into_iter().find(|path| Path::new(path).is_file())
        else {
            return;
        };
        let home = tempfile::tempdir().unwrap();
        fs::write(
            home.path().join(".bashrc"),
            "__alacritty_user_prompt() { printf '%s\\n' \"$?\" > \"$HOME/prompt-status\"; :; \
             }\nPROMPT_COMMAND=(__alacritty_user_prompt)\n",
        )
        .unwrap();
        let mut options = Options {
            shell: Some(Shell::new(bash.into(), vec!["--noprofile".into()])),
            ..Options::default()
        };
        let integration = ShellIntegration::prepare(&mut options).unwrap().unwrap();
        let mut master_fd = -1;
        let mut slave_fd = -1;
        let result = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(result, 0);
        let mut master = unsafe { File::from_raw_fd(master_fd) };
        let slave = unsafe { File::from_raw_fd(slave_fd) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            -1
        );
        let configured_shell = options.shell.as_ref().unwrap();
        let child = Command::new(configured_shell.program())
            .args(configured_shell.args())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("HISTFILE", home.path().join("history"))
            .env("HISTCONTROL", "")
            .env("HISTIGNORE", "")
            .env("PS1", "ALACRITTY_TEST_READY> ")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .unwrap();
        let _shell = PtyShell(child);
        let mut pending = Vec::new();
        let prompt = b"ALACRITTY_TEST_READY> ";
        wait_for(&mut master, &mut pending, prompt);
        assert!(integration.snapshot().unwrap().ready);
        assert_eq!(integration.snapshot().unwrap().completion, None);
        master.write_all(b"false\n").unwrap();
        wait_for(&mut master, &mut pending, prompt);
        let failed = integration.snapshot().unwrap().completion.unwrap();
        assert_eq!(failed, (1, 1));
        assert_eq!(fs::read(home.path().join("prompt-status")).unwrap(), b"1\n");
        master.write_all(b"\n").unwrap();
        wait_for(&mut master, &mut pending, prompt);
        assert_eq!(integration.snapshot().unwrap().completion, Some(failed));
        master.write_all(b"true\n").unwrap();
        wait_for(&mut master, &mut pending, prompt);
        assert_eq!(integration.snapshot().unwrap().completion, Some((2, 0)));
        assert_eq!(fs::read(home.path().join("prompt-status")).unwrap(), b"0\n");

        for (input, expected) in [
            ("printf 'one\\n'\n", "printf 'one\\n'"),
            ("\u{1b}[A\n", "printf 'one\\n'"),
            ("\u{1b}[A\u{1}\u{b}printf 'two\\n'\n", "printf 'two\\n'"),
            ("printf '%s\\n' 'first\nsecond'\n", "printf '%s\\n' 'first\nsecond'"),
        ] {
            master.write_all(input.as_bytes()).unwrap();
            wait_for(&mut master, &mut pending, prompt);
            assert_eq!(integration.snapshot().unwrap().commands.last().unwrap().command, expected);
        }

        let record = CommandRecord {
            command: "printf '%s' 'first\trow\nsecond\u{1b}X' > replayed-output".into(),
            working_directory: home.path().to_owned(),
        };
        let previous_count = integration.snapshot().unwrap().commands.len();
        let replay = integration.replay(&record).unwrap();
        master.write_all(replay.as_bytes()).unwrap();
        wait_for(&mut master, &mut pending, prompt);
        assert_eq!(
            fs::read(home.path().join("replayed-output")).unwrap(),
            b"first\trow\nsecond\x1bX"
        );
        let snapshot = integration.snapshot().unwrap();
        assert_eq!(snapshot.commands.len(), previous_count + 1);
        assert_eq!(snapshot.commands.last().unwrap().command, record.command);
        assert_eq!(snapshot.working_directory, Some(home.path().to_owned()));
        assert_eq!(fs::read(integration.directory.path().join("replay-marker")).unwrap(), b"0\n");

        master.write_all(b"read -s -p 'PRIVATE_INPUT> ' reply; printf '\\n'\n").unwrap();
        wait_for(&mut master, &mut pending, b"PRIVATE_INPUT> ");
        master.write_all(b"synthetic-password-value\n").unwrap();
        wait_for(&mut master, &mut pending, prompt);
        assert!(
            integration
                .snapshot()
                .unwrap()
                .commands
                .iter()
                .all(|record| !record.command.contains("synthetic-password-value"))
        );

        let completed = integration.snapshot().unwrap().completion.unwrap();
        master.write_all(b"sleep 0.5\n").unwrap();
        let deadline = Instant::now() + Duration::from_millis(400);
        loop {
            let snapshot = integration.snapshot().unwrap();
            if snapshot.commands.last().is_some_and(|record| record.command == "sleep 0.5") {
                assert!(!snapshot.ready);
                assert_eq!(snapshot.completion, Some(completed));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Running command was not captured before completing"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        wait_for(&mut master, &mut pending, prompt);
        assert_eq!(integration.snapshot().unwrap().completion, Some((completed.0 + 1, 0)));

        master.write_all(b"HISTCONTROL=ignorespace\n").unwrap();
        wait_for(&mut master, &mut pending, prompt);
        let count = integration.snapshot().unwrap().commands.len();
        let completed = integration.snapshot().unwrap().completion.unwrap();
        master.write_all(b" sleep 0.5; false\n").unwrap();
        wait_for_busy(&integration);
        assert_eq!(integration.snapshot().unwrap().commands.len(), count);
        assert_eq!(integration.snapshot().unwrap().completion, Some(completed));
        wait_for(&mut master, &mut pending, prompt);
        assert_eq!(integration.snapshot().unwrap().commands.len(), count);
        assert!(integration.snapshot().unwrap().ready);
        assert_eq!(integration.snapshot().unwrap().completion, Some((completed.0 + 1, 1)));
        master.write_all(b"set +o history\n").unwrap();
        wait_for(&mut master, &mut pending, prompt);
        let count = integration.snapshot().unwrap().commands.len();
        let completed = integration.snapshot().unwrap().completion.unwrap();
        master.write_all(b"sleep 0.5; false\n").unwrap();
        wait_for_busy(&integration);
        assert_eq!(integration.snapshot().unwrap().commands.len(), count);
        assert_eq!(integration.snapshot().unwrap().completion, Some(completed));
        wait_for(&mut master, &mut pending, prompt);
        assert_eq!(integration.snapshot().unwrap().commands.len(), count);
        assert!(integration.snapshot().unwrap().ready);
        assert_eq!(integration.snapshot().unwrap().completion, Some((completed.0 + 1, 1)));
    }
}

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::debug;
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use winit::event_loop::EventLoopProxy;

use crate::event::{Event, EventType};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);
const START_DELAY: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    Current,
    Available {
        version: String,
    },
    Installing,
    Installed,
    Failed,
    Unmanaged,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Offer {
    id: String,
    version: String,
    sha256: String,
}

impl Offer {
    fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 128
            && self.id.as_bytes()[0].is_ascii_alphanumeric()
            && self.id.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            && !self.version.trim().is_empty()
            && self.version.len() <= 128
            && !self.version.chars().any(char::is_control)
            && valid_hash(&self.sha256)
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum Reply {
    Available(Offer),
    Current,
    Installed,
    Unmanaged,
}

#[cfg(any(target_os = "linux", test))]
fn linux_prefix(exe: &Path) -> Option<PathBuf> {
    let prefix = exe.parent()?.parent()?;
    (exe == prefix.join("bin/termust") && prefix.join("share/doc/termust/install.json").is_file())
        .then(|| prefix.to_owned())
}

#[cfg(any(target_os = "macos", test))]
fn mac_prefix(exe: &Path) -> Option<PathBuf> {
    let app = exe.ancestors().nth(3)?;
    if !app.ends_with("Termust.app") || exe != app.join("Contents/MacOS/termust") {
        return None;
    }
    let parent = app.parent()?;
    let prefix = if parent.parent()?.ends_with(".termust/releases") {
        if !valid_hash(parent.file_name()?.to_str()?) {
            return None;
        }
        parent.ancestors().nth(3)?
    } else {
        parent
    };
    let link = prefix.join("Termust.app");
    let active = link.is_symlink().then(|| link.canonicalize().ok()).flatten()?;
    let release = active.parent()?;
    (app.canonicalize().ok()? == active
        && active.ends_with("Termust.app")
        && release.parent()? == prefix.join(".termust/releases")
        && valid_hash(release.file_name()?.to_str()?))
    .then(|| prefix.to_owned())
}

#[derive(Debug)]
enum Job {
    Check { automatic: bool },
    Install(Offer),
}

fn run_helper(prefix: &Path, job: &Job) -> io::Result<Reply> {
    let temporary = tempfile::tempdir()?;
    let script = temporary.path().join("install.sh");
    std::fs::write(&script, include_bytes!("../../install.sh"))?;
    let mut cmd = Command::new("/bin/bash");
    cmd.arg("-p").arg(script).arg("--prefix").arg(prefix);
    match job {
        Job::Check { .. } => cmd.arg("--update-status"),
        Job::Install(offer) => {
            cmd.arg("--apply-update").arg(&offer.id).arg("--expected-sha256").arg(&offer.sha256)
        },
    };
    let output = cmd
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env("PYTHONNOUSERSITE", "1")
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin")
        .current_dir(temporary.path())
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
    }
    let line = output.stdout.split(|&b| b == b'\n').rfind(|line| !line.is_empty());
    let reply: Reply = serde_json::from_slice(line.unwrap_or_default())?;
    match (job, &reply) {
        (Job::Check { .. }, Reply::Available(offer)) if offer.valid() => Ok(reply),
        (_, Reply::Current | Reply::Unmanaged) | (Job::Install(_), Reply::Installed) => Ok(reply),
        _ => Err(io::Error::other("invalid response from the Termust update helper")),
    }
}

#[derive(Deserialize, Serialize)]
struct Cache {
    active: String,
    next: u64,
    reply: Option<Reply>,
}

impl Cache {
    fn remaining(&self, active: &str, now: u64) -> Option<Duration> {
        let remaining = self.next.checked_sub(now)?;
        (self.active == active && remaining > 0 && remaining <= DAY.as_secs())
            .then(|| Duration::from_secs(remaining))
    }
}

fn cache_file(prefix: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(prefix.join(".termust-update-check"))?;
    let info = file.metadata()?;
    if !info.is_file() || info.nlink() != 1 || info.uid() != prefix.metadata()?.uid() {
        return Err(io::Error::other("invalid Termust update cache"));
    }
    flock(&file, FlockOperation::NonBlockingLockExclusive)?;
    Ok(file)
}

fn active_id(prefix: &Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let receipt = File::open(prefix.join("share/doc/termust/install.json")).ok()?;
        let json: serde_json::Value = serde_json::from_reader(receipt.take(128 * 1024)).ok()?;
        let sha = json.get("archive_sha256")?.as_str()?;
        valid_hash(sha).then(|| sha.to_owned())
    }
    #[cfg(target_os = "macos")]
    {
        let app = prefix.join("Termust.app").canonicalize().ok()?;
        let sha = app.parent()?.file_name()?.to_str()?;
        valid_hash(sha).then(|| sha.to_owned())
    }
}

#[derive(Clone, Debug)]
pub struct Completion {
    reply: Option<Reply>,
    retry_in: Duration,
    fresh: bool,
    failed: bool,
}

impl Completion {
    fn from_result(result: io::Result<Reply>) -> Self {
        match result {
            Ok(reply) => Self { reply: Some(reply), retry_in: DAY, fresh: true, failed: false },
            Err(error) => {
                debug!("Termust update: {error}");
                Self { reply: None, retry_in: RETRY, fresh: false, failed: true }
            },
        }
    }
}

fn check(prefix: &Path, automatic: bool, query: impl FnOnce() -> io::Result<Reply>) -> Completion {
    let Some(active) = active_id(prefix) else {
        return Completion::from_result(Ok(Reply::Unmanaged));
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut locked = cache_file(prefix).ok();
    if automatic {
        if let Some(file) = locked.as_mut() {
            if let Ok(cache) = serde_json::from_reader::<_, Cache>((&mut *file).take(4096)) {
                if let Some(retry_in) = cache.remaining(&active, now) {
                    let reply = cache.reply.filter(|r| match r {
                        Reply::Available(offer) => offer.valid(),
                        Reply::Current => true,
                        _ => false,
                    });
                    return Completion { reply, retry_in, fresh: false, failed: false };
                }
            }
        } else {
            return Completion { reply: None, retry_in: RETRY, fresh: false, failed: false };
        }
    }
    let result = Completion::from_result(query());
    if let Some(mut file) = locked {
        let cache = Cache {
            active,
            next: now.saturating_add(result.retry_in.as_secs()),
            reply: result.reply.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&cache) {
            let _ =
                file.rewind().and_then(|()| file.set_len(0)).and_then(|()| file.write_all(&bytes));
        }
    }
    result
}

pub struct Updater {
    prefix: Option<PathBuf>,
    pub status: Status,
    offer: Option<Offer>,
    next: Instant,
    automatic: bool,
}

impl Updater {
    pub fn new() -> Self {
        #[cfg(target_os = "linux")]
        let locate = linux_prefix;
        #[cfg(target_os = "macos")]
        let locate = mac_prefix;
        Self::for_prefix(std::env::current_exe().ok().and_then(|path| locate(&path)))
    }

    fn for_prefix(prefix: Option<PathBuf>) -> Self {
        let status = if prefix.is_some() { Status::Idle } else { Status::Unmanaged };
        Self { prefix, status, offer: None, next: Instant::now() + START_DELAY, automatic: false }
    }

    pub fn deadline(&self, enabled: bool) -> Option<Instant> {
        match self.status {
            Status::Checking | Status::Installing | Status::Installed | Status::Unmanaged => None,
            _ => enabled.then_some(self.next),
        }
    }

    pub fn check(&mut self, automatic: bool, proxy: &EventLoopProxy<Event>) {
        match self.status {
            Status::Checking => self.automatic &= automatic,
            Status::Installing | Status::Installed | Status::Unmanaged => (),
            _ => {
                self.automatic = automatic;
                self.status = Status::Checking;
                self.start(Job::Check { automatic }, proxy);
            },
        }
    }

    fn confirmed_offer(&mut self, displayed_version: &str) -> Option<Offer> {
        match &self.status {
            Status::Available { version } if version == displayed_version => self.offer.take(),
            _ => None,
        }
    }

    pub fn install(&mut self, displayed_version: &str, proxy: &EventLoopProxy<Event>) {
        if let Some(offer) = self.confirmed_offer(displayed_version) {
            self.automatic = false;
            self.status = Status::Installing;
            self.start(Job::Install(offer), proxy);
        }
    }

    fn start(&mut self, job: Job, proxy: &EventLoopProxy<Event>) {
        let Some(prefix) = self.prefix.clone() else { return };
        let proxy = proxy.clone();
        let result = thread::Builder::new().name("termust-update".into()).spawn(move || {
            let result = match &job {
                Job::Check { automatic } => {
                    check(&prefix, *automatic, || run_helper(&prefix, &job))
                },
                Job::Install(_) => Completion::from_result(run_helper(&prefix, &job)),
            };
            let _ = proxy.send_event(Event::new(EventType::UpdateFinished(result), None));
        });
        if let Err(error) = result {
            self.finish(Completion::from_result(Err(error)));
        }
    }

    pub fn complete(&mut self, result: Completion, proxy: &EventLoopProxy<Event>) -> bool {
        let manual = self.status == Status::Checking && !self.automatic;
        let cached = !result.fresh && !result.failed;
        let notify = self.finish(result);
        if manual && cached {
            self.check(false, proxy);
        }
        notify
    }

    fn finish(&mut self, result: Completion) -> bool {
        let installing = self.status == Status::Installing;
        self.next = Instant::now() + result.retry_in;
        self.offer = None;
        self.status = match result.reply {
            Some(Reply::Available(offer)) => {
                let version = offer.version.clone();
                self.offer = Some(offer);
                Status::Available { version }
            },
            Some(Reply::Installed) => Status::Installed,
            Some(Reply::Current) if installing => Status::Installed,
            Some(Reply::Current) => Status::Current,
            Some(Reply::Unmanaged) => Status::Unmanaged,
            None if result.failed => Status::Failed,
            None => Status::Idle,
        };
        self.automatic && result.fresh && matches!(self.status, Status::Available { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn offer() -> Offer {
        Offer {
            id: "0.18.0-termust.abc".into(),
            version: "0.18.0-dev (build 2)".into(),
            sha256: "a".repeat(64),
        }
    }

    fn managed_linux(prefix: &Path, sha: &str) -> PathBuf {
        let receipt = prefix.join("share/doc/termust/install.json");
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(receipt, serde_json::to_vec(&serde_json::json!({"archive_sha256":sha})).unwrap())
            .unwrap();
        prefix.join("bin/termust")
    }

    fn managed_mac(prefix: &Path, sha: &str) -> PathBuf {
        let app = prefix.join(".termust/releases").join(sha).join("Termust.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let link = prefix.join("Termust.app");
        if link.is_symlink() {
            fs::remove_file(&link).unwrap();
        }
        symlink(&app, link).unwrap();
        app.join("Contents/MacOS/termust")
    }

    #[test]
    fn only_recognizes_installations_with_an_official_layout() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().canonicalize().unwrap();
        assert!(linux_prefix(&prefix.join("bin/termust")).is_none());
        let exe = managed_linux(&prefix, &"1".repeat(64));
        assert_eq!(linux_prefix(&exe).as_deref(), Some(prefix.as_path()));
        assert!(linux_prefix(&prefix.join("target/debug/termust")).is_none());
        let bundle_exe = prefix.join("Termust.app/Contents/MacOS/termust");
        assert!(mac_prefix(&bundle_exe).is_none());
        let original = managed_mac(&prefix, &"2".repeat(64));
        let active = managed_mac(&prefix, &"3".repeat(64));
        assert!(mac_prefix(&original).is_none());
        for path in [&active, &bundle_exe] {
            assert_eq!(mac_prefix(path).as_deref(), Some(prefix.as_path()));
        }
        fs::remove_file(prefix.join("Termust.app")).unwrap();
        fs::create_dir(prefix.join("unmanaged")).unwrap();
        symlink(prefix.join("unmanaged"), prefix.join("Termust.app")).unwrap();
        assert!(mac_prefix(&original).is_none());
    }

    #[test]
    fn checks_never_install_and_only_the_displayed_offer_can_be_confirmed() {
        let mut updater = Updater::for_prefix(Some("/tmp/test".into()));
        assert!(updater.deadline(false).is_none());
        assert!(updater.deadline(true).is_some());
        assert!(updater.confirmed_offer("anything").is_none());
        updater.automatic = true;
        assert!(updater.finish(Completion::from_result(Ok(Reply::Available(offer())))));
        assert!(updater.confirmed_offer("stale build").is_none());
        assert_eq!(updater.confirmed_offer(&offer().version).unwrap().sha256, offer().sha256);
        assert!(updater.confirmed_offer(&offer().version).is_none());
        updater.status = Status::Installing;
        assert!(updater.deadline(true).is_none());
        assert!(!updater.finish(Completion::from_result(Ok(Reply::Installed))));
        assert_eq!(updater.status, Status::Installed);
        assert!(updater.deadline(true).is_none());
        assert!(Updater::for_prefix(None).deadline(true).is_none());
    }

    #[test]
    fn cached_checks_are_scoped_to_the_installed_build_and_manual_checks_bypass_them() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().canonicalize().unwrap();
        #[cfg(target_os = "linux")]
        let managed = managed_linux;
        #[cfg(target_os = "macos")]
        let managed = managed_mac;
        managed(&prefix, &"1".repeat(64));
        let first = check(&prefix, true, || Ok(Reply::Available(offer())));
        assert!(first.fresh);
        let cached = check(&prefix, true, || panic!("duplicate network check"));
        assert!(!cached.fresh);
        assert!(matches!(cached.reply, Some(Reply::Available(_))));
        let forced = check(&prefix, false, || Ok(Reply::Current));
        assert!(forced.fresh);
        assert!(matches!(forced.reply, Some(Reply::Current)));
        managed(&prefix, &"2".repeat(64));
        let offline = check(&prefix, true, || Err(io::Error::other("offline")));
        assert!(offline.failed);
        assert_eq!(offline.retry_in, RETRY);
        let retry_later = check(&prefix, true, || panic!("should back off offline"));
        assert!(!retry_later.fresh);
        assert!(retry_later.reply.is_none());
        let _lock = cache_file(&prefix).unwrap();
        let concurrent = check(&prefix, true, || panic!("another process is checking"));
        assert!(!concurrent.fresh);
        assert!(concurrent.reply.is_none());
    }

    #[test]
    fn rejects_invalid_offer_fields_and_unreasonable_cache_deadlines() {
        for id in ["../other", "--prefix", "a/b", "x\ny", ""] {
            assert!(!Offer { id: id.into(), ..offer() }.valid());
        }
        assert!(!Offer { sha256: "0".repeat(63), ..offer() }.valid());
        assert!(!Offer { version: "a\nb".into(), ..offer() }.valid());
        let cache = Cache { active: "x".into(), next: DAY.as_secs() + 2, reply: None };
        assert!(cache.remaining("x", 0).is_none());
        assert!(cache.remaining("x", cache.next).is_none());
        assert!(cache.remaining("y", 2).is_none());
    }
}

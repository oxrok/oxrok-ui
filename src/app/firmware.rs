//! Bundled resources (firmware, decoders, Python), and the Install
//! Firmware dialog's download or import job.

use crate::backend::fetch;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the bundled resources are, if anywhere.
pub fn resource_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LA_RESOURCE_DIR").map(PathBuf::from).filter(|d| d.is_dir()) {
        return Some(d);
    }
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let candidates = exe_dir
        .iter()
        .flat_map(|d| [d.join("../Resources"), d.join("../share/oxrok-ui"), d.join("../share/logic-analyzer")])
        .chain(std::iter::once(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor")));
    candidates.into_iter().find(|d| d.join("sigrok-firmware").is_dir() || d.join("decoders").is_dir())
}

/// The bundled open firmware, for `SIGROK_FIRMWARE_DIR`.
pub fn bundled_firmware_dir() -> Option<PathBuf> {
    resource_dir().map(|d| d.join("sigrok-firmware")).filter(|d| d.is_dir())
}

/// 22 Packaging: a bundle's own Python (`Frameworks/Python.framework/
/// Versions/3.x` on macOS, `python/` in the resources elsewhere).
pub fn bundled_python_home() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let versions = exe_dir.join("../Frameworks/Python.framework/Versions");
    let framework = std::fs::read_dir(&versions)
        .ok()
        .and_then(|it| it.flatten().map(|e| e.path()).find(|p| p.join("lib").is_dir()));
    framework.or_else(|| resource_dir().map(|r| r.join("python")).filter(|p| p.is_dir()))
}

/// Packaging: points oxrok at the bundled firmware and decoders,
/// unless already set.
///
/// # Safety
/// Sets environment variables: call before any other thread starts.
pub unsafe fn apply_bundle_env() {
    let set = |key: &str, value: Option<PathBuf>| {
        if std::env::var_os(key).is_none()
            && let Some(v) = value
        {
            // SAFETY: the caller guarantees no other thread runs yet.
            unsafe { std::env::set_var(key, v) };
        }
    };
    set("OXROK_FIRMWARE_DIR", bundled_firmware_dir());
    set("SIGROK_FIRMWARE_DIR", bundled_firmware_dir());
    set("OXROK_DECODERS_DIR", resource_dir().map(|r| r.join("decoders")).filter(|d| d.is_dir()));
    set("SIGROKDECODE_DIR", resource_dir().map(|r| r.join("decoders")).filter(|d| d.is_dir()));
    set("PYTHONHOME", bundled_python_home());
    // A signed bundle must not change: no `__pycache__` beside the decoders
    // or the standard library.
    if resource_dir().is_some_and(|r| r.join("decoders").is_dir()) {
        set("PYTHONDONTWRITEBYTECODE", Some("1".into()));
    }
}

/// Whether `text` names `file` itself, not a longer name that contains
/// it (`bc.bin` in `abc.bin`).
pub fn names_file(text: &str, file: &str) -> bool {
    let in_name = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_');
    !file.is_empty()
        && text.match_indices(file).any(|(at, _)| {
            let (before, after) = (&text[..at], &text[at + file.len()..]);
            !before.ends_with(|c| in_name(c) || c == '.') && !after.starts_with(in_name)
        })
}

/// How to get an external unpacker on `os` (as `std::env::consts::OS`),
/// in English. Windows: the firmware crate finds one only on `PATH`,
/// which a running app doesn't see change.
pub fn unpacker_hint(os: &str) -> &'static str {
    match os {
        "macos" => "To install one: brew install unar",
        "windows" => "Install 7-Zip (winget install 7zip.7zip), add its folder to PATH and start the app again.",
        _ => "To install one: sudo apt install unar, or sudo dnf install unar",
    }
}

/// A download size: "27.8 MB", "334 kB".
pub fn human_size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / f64::from(1u32 << 20))
    } else {
        format!("{} kB", bytes.div_ceil(1024))
    }
}

/// What a [`Job`] does.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum JobKind {
    Download,
    Import,
}

/// A firmware download or import: the worker thread reports into it as
/// the fetch's [`fetch::Progress`], and the dialog reads it each frame
/// and cancels it.
pub(crate) struct Job {
    pub kind: JobKind,
    /// Bytes to download; 0 for an import.
    total: u64,
    cancel: AtomicBool,
    state: Mutex<JobState>,
    /// Wakes the UI.
    repaint: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct JobState {
    log: Vec<String>,
    /// The download in progress, its bytes so far, and the bytes of the
    /// downloads before it.
    name: String,
    done: u64,
    before: u64,
    result: Option<Result<fetch::Installed, fetch::Error>>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job").field("kind", &self.kind).field("finished", &self.finished()).finish_non_exhaustive()
    }
}

impl Job {
    pub fn new(kind: JobKind, total: u64, repaint: impl Fn() + Send + Sync + 'static) -> Self {
        Self { kind, total, cancel: AtomicBool::new(false), state: Mutex::default(), repaint: Box::new(repaint) }
    }

    /// Asks the worker to stop. A cancelled fetch installs nothing.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn finish(&self, result: Result<fetch::Installed, fetch::Error>) {
        self.state.lock().result = Some(result);
        (self.repaint)();
    }

    pub fn finished(&self) -> bool {
        self.state.lock().result.is_some()
    }

    /// The downloaded bytes and their total, once a download reported.
    pub fn downloaded(&self) -> Option<(u64, u64)> {
        let s = self.state.lock();
        let done = s.before + s.done;
        (self.total > 0 && !s.name.is_empty()).then(|| (done.min(self.total), self.total))
    }

    pub fn log(&self) -> String {
        self.state.lock().log.join("\n")
    }

    /// Reads the result, when there is one.
    pub fn with_result<T>(&self, f: impl FnOnce(&Result<fetch::Installed, fetch::Error>) -> T) -> Option<T> {
        self.state.lock().result.as_ref().map(f)
    }
}

impl fetch::Progress for Job {
    fn message(&self, msg: &str) {
        self.state.lock().log.push(msg.to_owned());
        (self.repaint)();
    }

    fn download(&self, name: &str, done: u64, total: u64) {
        let mut s = self.state.lock();
        if s.name != name {
            // The next file.
            s.before += s.done;
            s.name = name.to_owned();
        }
        s.done = done.min(total);
        drop(s);
        (self.repaint)();
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fetch::Progress;

    #[test]
    fn whole_file_names() {
        let error = "firmware file not found: abc.bin (install it)";
        assert!(names_file(error, "abc.bin"));
        assert!(!names_file(error, "bc.bin"), "part of a longer name");
        assert!(!names_file(error, "abc.bi"));
        assert!(!names_file(error, ""));
        assert!(names_file("firmware not found: dir/a-b.fw.", "a-b.fw"), "after a path, before a full stop");
        assert!(!names_file("x.a-b.fw", "a-b.fw"));
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(29_160_040), "27.8 MB");
        assert_eq!(human_size(341_160), "334 kB");
        assert_eq!(human_size(1), "1 kB");
    }

    /// Bytes add up across a package's downloads, and Cancel reaches the
    /// worker.
    #[test]
    fn job_progress() {
        let job = Job::new(JobKind::Download, 300, || {});
        assert_eq!(job.downloaded(), None);
        job.download("a.fw", 50, 100);
        assert_eq!(job.downloaded(), Some((50, 300)));
        job.download("a.fw", 100, 100);
        job.download("b.fw", 20, 200);
        assert_eq!(job.downloaded(), Some((120, 300)));
        job.message("Extracting");
        assert_eq!(job.log(), "Extracting");
        assert!(!job.cancelled() && !job.finished());
        job.cancel();
        assert!(job.cancelled());
        job.finish(Err(fetch::Error::Cancelled));
        assert_eq!(job.with_result(|r| r.is_err()), Some(true));
    }

    /// The real thing, over the network, with a `Job` as its progress:
    /// `cargo test --lib -- --ignored downloads_for_real`.
    #[test]
    #[ignore = "downloads 54 kB from sigrok.org"]
    fn downloads_for_real() {
        let pkg = fetch::package("fx2lafw").unwrap();
        let (cache, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let job = Job::new(JobKind::Download, pkg.download_size(), || {});
        let options = fetch::Options { cache_dir: cache.path().to_owned(), progress: &job };
        let installed = fetch::fetch(pkg, dest.path(), &options).unwrap();
        assert_eq!(installed.files.len(), pkg.files.len());
        assert!(dest.path().join("fx2lafw-saleae-logic.fw").is_file());
        assert_eq!(job.downloaded(), Some((pkg.download_size(), pkg.download_size())));
        eprintln!("{}", job.log());
    }
}

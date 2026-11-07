//! Native file dialogs, behind a trait so tests can answer
//! them without a window.

use std::path::{Path, PathBuf};

/// A file-type filter: a label and its extensions without dots.
pub type Filter = (String, Vec<String>);

pub trait FileDialogs: Send + Sync {
    /// Blocks until the user picks a folder or cancels. Called off the UI thread.
    fn pick_folder(&self, start: Option<&Path>) -> Option<PathBuf>;

    /// Where to save a file. Called off the UI thread.
    fn save_file(&self, title: &str, start: Option<&Path>, name: &str, filters: &[Filter]) -> Option<PathBuf>;

    /// Files to open (several allowed). Called off the UI thread.
    fn open_files(&self, title: &str, start: Option<&Path>, filters: &[Filter]) -> Vec<PathBuf>;
}

/// `rfd`'s async dialogs, driven to completion on the calling thread.
pub struct NativeDialogs;

fn with_filters(mut d: rfd::AsyncFileDialog, filters: &[Filter]) -> rfd::AsyncFileDialog {
    for (label, exts) in filters {
        d = d.add_filter(label, exts);
    }
    d
}

impl FileDialogs for NativeDialogs {
    fn pick_folder(&self, start: Option<&Path>) -> Option<PathBuf> {
        let mut d = rfd::AsyncFileDialog::new().set_title("Choose folder for export");
        if let Some(s) = start {
            d = d.set_directory(s);
        }
        pollster::block_on(d.pick_folder()).map(|h| h.path().to_path_buf())
    }

    fn save_file(&self, title: &str, start: Option<&Path>, name: &str, filters: &[Filter]) -> Option<PathBuf> {
        let mut d = with_filters(rfd::AsyncFileDialog::new().set_title(title).set_file_name(name), filters);
        if let Some(s) = start {
            d = d.set_directory(s);
        }
        pollster::block_on(d.save_file()).map(|h| h.path().to_path_buf())
    }

    fn open_files(&self, title: &str, start: Option<&Path>, filters: &[Filter]) -> Vec<PathBuf> {
        let mut d = with_filters(rfd::AsyncFileDialog::new().set_title(title), filters);
        if let Some(s) = start {
            d = d.set_directory(s);
        }
        pollster::block_on(d.pick_files()).unwrap_or_default().iter().map(|h| h.path().to_path_buf()).collect()
    }
}

/// Answers every dialog from a fixed folder (tests): folders
/// are that folder, saves go into it under the suggested name, and opens
/// return nothing.
pub struct FixedDialogs(pub Option<PathBuf>);

impl FileDialogs for FixedDialogs {
    fn pick_folder(&self, _start: Option<&Path>) -> Option<PathBuf> {
        self.0.clone()
    }

    fn save_file(&self, _title: &str, _start: Option<&Path>, name: &str, _filters: &[Filter]) -> Option<PathBuf> {
        self.0.as_ref().map(|d| d.join(name))
    }

    fn open_files(&self, _title: &str, _start: Option<&Path>, _filters: &[Filter]) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Test dialogs with scripted answers: the next files to open and the
/// next save path (falling back to the folder and suggested name).
#[derive(Default)]
pub struct ScriptedDialogs {
    pub folder: Option<PathBuf>,
    pub open: parking_lot::Mutex<Vec<PathBuf>>,
    pub save: parking_lot::Mutex<Option<PathBuf>>,
    /// The next save dialog is cancelled.
    pub cancel_save: std::sync::atomic::AtomicBool,
}

impl FileDialogs for ScriptedDialogs {
    fn pick_folder(&self, _start: Option<&Path>) -> Option<PathBuf> {
        self.folder.clone()
    }

    fn save_file(&self, _title: &str, _start: Option<&Path>, name: &str, _filters: &[Filter]) -> Option<PathBuf> {
        if self.cancel_save.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        self.save.lock().take().or_else(|| self.folder.as_ref().map(|d| d.join(name)))
    }

    fn open_files(&self, _title: &str, _start: Option<&Path>, _filters: &[Filter]) -> Vec<PathBuf> {
        std::mem::take(&mut *self.open.lock())
    }
}

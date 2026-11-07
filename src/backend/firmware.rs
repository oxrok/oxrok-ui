//! Running a sigrok-util firmware extraction script.

use crate::backend::BackendError;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

/// The interpreter a script's first line names: `python3` or `sh`.
pub fn interpreter(script: &Path) -> &'static str {
    let first = std::fs::read_to_string(script).ok().and_then(|s| s.lines().next().map(str::to_owned));
    if first.is_some_and(|l| l.contains("python")) { "python3" } else { "sh" }
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    std::fs::read_dir(dir)
        .map(|it| it.flatten().map(|e| (e.path(), e.metadata().and_then(|m| m.modified()).ok())).collect())
        .unwrap_or_default()
}

/// Runs `script input` in `out_dir`, streaming stdout and stderr lines to
/// `progress`; returns the files it created or rewrote.
pub fn run_script(
    script: &Path,
    input: &Path,
    out_dir: &Path,
    progress: &mut dyn FnMut(String),
) -> Result<Vec<PathBuf>, BackendError> {
    std::fs::create_dir_all(out_dir).map_err(|e| BackendError::Io(e.to_string()))?;
    let before = snapshot(out_dir);
    let mut child = Command::new(interpreter(script))
        .arg(script)
        .arg(input)
        .current_dir(out_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| BackendError::Io(format!("{}: {e}", interpreter(script))))?;
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    let readers: Vec<_> = [
        child.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|e| Box::new(e) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|r| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(r).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        })
    })
    .collect();
    drop(tx);
    let mut last = Vec::new();
    for line in rx {
        last.push(line.clone());
        if last.len() > 5 {
            last.remove(0);
        }
        progress(line);
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait().map_err(|e| BackendError::Io(e.to_string()))?;
    if !status.success() {
        return Err(BackendError::Device(format!("the script failed ({status})\n{}", last.join("\n"))));
    }
    let after = snapshot(out_dir);
    Ok(after.into_iter().filter(|a| !before.contains(a)).map(|a| a.0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shell "extractor": writes a file into the working directory.
    #[test]
    fn runs_a_script_and_lists_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("x.sh");
        std::fs::write(&script, "#!/bin/sh\necho saving out.fw\ncp \"$1\" out.fw\n").unwrap();
        let input = dir.path().join("vendor.bin");
        std::fs::write(&input, b"abc").unwrap();
        let out = dir.path().join("fw");
        let mut lines = Vec::new();
        let files = run_script(&script, &input, &out, &mut |l| lines.push(l)).unwrap();
        assert_eq!(lines, ["saving out.fw"]);
        assert_eq!(files, [out.join("out.fw")]);
        assert_eq!(interpreter(&script), "sh");
        // A failing script reports its last lines.
        std::fs::write(&script, "#!/bin/sh\necho checksum mismatch >&2\nexit 1\n").unwrap();
        let e = run_script(&script, &input, &out, &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("checksum mismatch"), "{e}");
    }
}

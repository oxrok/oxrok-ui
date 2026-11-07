//! The build date and commit for the About dialog.

use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

fn main() {
    let date = run("date", &["-u", "+%Y-%m-%d"]).unwrap_or_else(|| "unknown date".into());
    let commit = run("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown commit".into());
    println!("cargo:rustc-env=LA_BUILD_DATE={date}");
    println!("cargo:rustc-env=LA_BUILD_COMMIT={commit}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-env-changed=LA_PROJECT_URL");
}

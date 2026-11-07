//! Probe profile storage: built-ins shipped with the app plus
//! user files in `<config>/logic-analyzer/probe-profiles/`.

use crate::backend::DeviceInfo;
use crate::core::probe::{DeviceKey, Match, ProbeProfile, select};
use std::path::{Path, PathBuf};

/// Built-in profiles (read-only).
pub const BUILTIN: &[(&str, &str)] =
    &[("intronix-logicport.json", include_str!("../../profiles/intronix-logicport.json"))];

/// Where the active profile of a session comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProfileSource {
    BuiltIn,
    User,
    /// A user copy that shadows a built-in with the same match.
    UserOverride,
}

#[derive(Default)]
pub struct ProfileStore {
    builtins: Vec<ProbeProfile>,
    user: Vec<(PathBuf, ProbeProfile)>,
    dir: Option<PathBuf>,
    /// Files that failed to load: `(file, error)`.
    pub errors: Vec<(String, String)>,
}

pub fn default_dir() -> Option<PathBuf> {
    crate::app::settings::default_path().and_then(|p| p.parent().map(|d| d.join("probe-profiles")))
}

pub fn device_key(d: &DeviceInfo) -> DeviceKey<'_> {
    DeviceKey { driver: &d.driver, vendor: &d.vendor, model: &d.model, channels: d.channels.len() }
}

/// `driver[-vendor][-model]`, lowercase, safe as a file name.
fn file_name(m: &Match) -> String {
    let parts = [Some(m.driver.as_str()), m.vendor.as_deref(), m.model.as_deref()];
    let slug: String = parts
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    format!("{slug}.json")
}

impl ProfileStore {
    /// Loads the built-ins (plus `extra_builtins`, for tests) and the user directory.
    pub fn load(dir: Option<PathBuf>, extra_builtins: &[String]) -> Self {
        let mut s = Self { dir, ..Self::default() };
        let texts = BUILTIN
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .chain(extra_builtins.iter().map(|t| ("<test>".into(), t.clone())));
        for (name, text) in texts {
            match ProbeProfile::parse(&text) {
                Ok(p) => s.builtins.push(p),
                Err(e) => s.errors.push((name, e.to_string())),
            }
        }
        s.reload_user();
        s
    }

    pub fn reload_user(&mut self) {
        self.user.clear();
        self.errors.retain(|(n, _)| !n.ends_with(".json") || BUILTIN.iter().any(|(b, _)| b == n));
        let Some(dir) = &self.dir else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<PathBuf> =
            entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| ProbeProfile::parse(&t).map_err(|e| e.to_string()))
            {
                Ok(p) => self.user.push((path, p)),
                Err(e) => self.errors.push((name, e)),
            }
        }
    }

    /// The best profile for `device` and where it comes from.
    pub fn select(&self, device: &DeviceInfo) -> Option<(ProbeProfile, ProfileSource)> {
        let key = device_key(device);
        let all = self.builtins.iter().map(|p| (p, false)).chain(self.user.iter().map(|(_, p)| (p, true)));
        let (p, user) = select(all, &key)?;
        let source = match user {
            false => ProfileSource::BuiltIn,
            true if self.builtin_with(&p.matcher).is_some() => ProfileSource::UserOverride,
            true => ProfileSource::User,
        };
        Some((p.clone(), source))
    }

    pub fn builtin_with(&self, m: &Match) -> Option<&ProbeProfile> {
        self.builtins.iter().find(|p| p.matcher == *m)
    }

    fn user_path(&self, m: &Match) -> Option<PathBuf> {
        self.user.iter().find(|(_, p)| p.matcher == *m).map(|(path, _)| path.clone())
    }

    /// Writes `p` as a user profile (replacing the user file with the same
    /// match), atomically. Editing a built-in thus creates a user copy.
    pub fn save(&mut self, p: &ProbeProfile) -> std::io::Result<PathBuf> {
        let dir = self.dir.clone().ok_or_else(|| std::io::Error::other("no profile directory"))?;
        std::fs::create_dir_all(&dir)?;
        let path = self.user_path(&p.matcher).unwrap_or_else(|| dir.join(file_name(&p.matcher)));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, p.to_json())?;
        std::fs::rename(&tmp, &path)?;
        self.reload_user();
        Ok(path)
    }

    /// Deletes the user profile with this match (*Reset to built-in* / *Delete*).
    pub fn delete_user(&mut self, m: &Match) -> std::io::Result<()> {
        if let Some(path) = self.user_path(m) {
            std::fs::remove_file(path)?;
        }
        self.reload_user();
        Ok(())
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use super::*;
    use crate::backend::{Backend, MockBackend};

    fn logicport() -> DeviceInfo {
        let mut d = MockBackend::seeded(1).scan_devices().remove(1);
        d.driver = "intronix-logicport".into();
        d
    }

    #[test]
    fn builtin_logicport_profile() {
        let s = ProfileStore::load(None, &[]);
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        let (p, src) = s.select(&logicport()).unwrap();
        assert_eq!(src, ProfileSource::BuiltIn);
        assert_eq!(p.groups.len(), 5);
        let d9 = p.channel("D9").unwrap();
        assert_eq!((d9.wire.as_ref().unwrap().spec.as_str(), p.marker(d9).unwrap().spec.as_str()), ("brown", "blue"));
        let clk2 = p.channel("CLK2").unwrap();
        assert_eq!(
            (clk2.wire.as_ref().unwrap().spec.as_str(), p.marker(clk2).unwrap().spec.as_str()),
            ("white", "green")
        );
    }

    #[test]
    fn user_copy_overrides_and_resets() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ProfileStore::load(Some(dir.path().to_path_buf()), &[]);
        let (mut p, _) = s.select(&logicport()).unwrap();
        p.channels[0].wire = Some(p.color("#123456").unwrap());
        let path = s.save(&p).unwrap();
        assert_eq!(path.file_name().unwrap(), "intronix-logicport.json");
        let (q, src) = s.select(&logicport()).unwrap();
        assert_eq!(src, ProfileSource::UserOverride);
        assert_eq!(q.channels[0].wire.as_ref().unwrap().rgb, [0x12, 0x34, 0x56]);
        s.delete_user(&q.matcher).unwrap();
        assert_eq!(s.select(&logicport()).unwrap().1, ProfileSource::BuiltIn);
    }

    #[test]
    fn bad_user_files_are_reported_and_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ nope").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let s = ProfileStore::load(Some(dir.path().to_path_buf()), &[]);
        assert_eq!(s.errors.len(), 1);
        assert_eq!(s.errors[0].0, "broken.json");
    }
}

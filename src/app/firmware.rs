//! Firmware families, bundled resources, and what an
//! extraction needs.

use std::path::{Path, PathBuf};

/// One row of the table.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Family {
    /// Missing file names starting with this belong to the family.
    pub prefix: &'static str,
    pub title: &'static str,
    /// Under the bundled `sigrok-util/firmware/`; `None`: manual only.
    pub script: Option<&'static str>,
    /// What the script takes (step 1).
    pub input: &'static str,
    pub needs: &'static [&'static str],
    /// The device's sigrok wiki page.
    pub wiki: &'static str,
}

pub const FAMILIES: [Family; 8] = [
    Family {
        prefix: "kingst-la",
        title: "Kingst LA",
        script: Some("kingst-la/sigrok-fwextract-kingst-la2016"),
        input: "The KingstVIS executable",
        needs: &["python3"],
        wiki: "https://sigrok.org/wiki/Kingst_LA2016",
    },
    Family {
        prefix: "saleae-logic16",
        title: "Saleae Logic16",
        script: Some("saleae-logic16/sigrok-fwextract-saleae-logic16"),
        input: "The Logic 1.2.x executable",
        needs: &["python3"],
        wiki: "https://sigrok.org/wiki/Saleae_Logic16",
    },
    Family {
        prefix: "hantek-",
        title: "Hantek DSO",
        script: Some("hantek-dso/sigrok-fwextract-hantek-dso"),
        input: "The Hantek Windows driver (.sys)",
        needs: &["python3"],
        wiki: "https://sigrok.org/wiki/Hantek_DSO-2090",
    },
    Family {
        prefix: "sysclk-lwla1016",
        title: "Sysclk LWLA1016",
        script: Some("sysclk-lwla/sigrok-fwextract-sysclk-lwla1016"),
        input: "The LWLA1016 installation folder",
        needs: &["sh", "sha256sum", "dd"],
        wiki: "https://sigrok.org/wiki/Sysclk_LWLA1016",
    },
    Family {
        prefix: "sysclk-lwla1034",
        title: "Sysclk LWLA1034",
        script: Some("sysclk-lwla/sigrok-fwextract-sysclk-lwla1034"),
        input: "lwla1034_EN_setup.exe from the CD-ROM",
        needs: &["sh", "sha256sum", "dd"],
        wiki: "https://sigrok.org/wiki/Sysclk_LWLA1034",
    },
    Family {
        prefix: "lecroy-logicstudio",
        title: "LeCroy LogicStudio",
        script: Some("lecroy-logicstudio/sigrok-fwextract-lecroy-logicstudio"),
        input: "The LogicStudio firmware file",
        needs: &["python3"],
        wiki: "https://sigrok.org/wiki/LeCroy_LogicStudio",
    },
    Family {
        prefix: "dreamsourcelab-dslogic",
        title: "DSLogic",
        script: None,
        input: "The DSView firmware files",
        needs: &[],
        wiki: "https://sigrok.org/wiki/DreamSourceLab_DSLogic",
    },
    Family {
        prefix: "fx2lafw-",
        title: "fx2lafw (open firmware)",
        script: None,
        input: "The sigrok-firmware-fx2lafw package",
        needs: &[],
        wiki: "https://sigrok.org/wiki/Fx2lafw",
    },
];

/// The family of a missing file; unknown files get a manual-only entry.
pub fn family_of(file: &str) -> Family {
    FAMILIES.iter().copied().find(|f| file.starts_with(f.prefix)).unwrap_or(Family {
        prefix: "",
        title: "this device",
        script: None,
        input: "The vendor's software",
        needs: &[],
        wiki: "https://sigrok.org/wiki/Supported_hardware",
    })
}

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
    candidates
        .into_iter()
        .find(|d| d.join("sigrok-util").is_dir() || d.join("sigrok-firmware").is_dir() || d.join("decoders").is_dir())
}

/// The bundled script for `family`, if it is there.
pub fn script_path(family: &Family) -> Option<PathBuf> {
    let p = resource_dir()?.join("sigrok-util/firmware").join(family.script?);
    p.is_file().then_some(p)
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

/// The tools of `needs` that aren't on `PATH`.
pub fn missing_tools(needs: &[&str]) -> Vec<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    needs
        .iter()
        .filter(|t| !std::env::split_paths(&path).any(|d| d.join(t).is_file()))
        .map(|t| (*t).to_owned())
        .collect()
}

/// An install hint for a missing tool.
pub fn install_hint(tool: &str) -> String {
    let mac = cfg!(target_os = "macos");
    match tool {
        "sha256sum" if mac => "brew install coreutils".into(),
        "python3" if mac => "brew install python".into(),
        t if mac => format!("brew install {t}"),
        t => format!("sudo apt install {t}"),
    }
}

/// The user firmware dir among libsigrok's search paths: the one under
/// the home folder, else `~/.local/share/sigrok-firmware`.
pub fn user_firmware_dir(paths: &[(PathBuf, bool)]) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    paths
        .iter()
        .map(|p| p.0.clone())
        .find(|p| home.as_ref().is_some_and(|h| p.starts_with(h)))
        .or_else(|| paths.first().map(|p| p.0.clone()))
        .unwrap_or_else(|| home.unwrap_or_default().join(".local/share/sigrok-firmware"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_by_file() {
        assert_eq!(family_of("kingst-la2016a1-fpga.bitstream").title, "Kingst LA");
        assert_eq!(family_of("sysclk-lwla1034-int.rbf").title, "Sysclk LWLA1034");
        assert_eq!(family_of("fx2lafw-saleae-logic.fw").script, None);
        assert_eq!(family_of("odd.bin").title, "this device");
        assert!(missing_tools(&["surely-not-a-tool-xyz"]) == ["surely-not-a-tool-xyz"]);
        // `sh` is a Unix shell; it isn't on Windows, where the firmware
        // scripts (and this check) don't apply.
        #[cfg(not(target_os = "windows"))]
        assert!(missing_tools(&["sh"]).is_empty());
        let paths = [(PathBuf::from("/usr/share/sigrok-firmware"), true)];
        assert_eq!(user_firmware_dir(&paths).file_name().unwrap(), "sigrok-firmware");
    }
}

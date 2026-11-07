//! Probe profiles: the physical wire and marker
//! colors of a device's probe leads, and how they are bundled.
//!
//! Profiles are data: every channel's colors are given explicitly (a
//! group's `wires` list is only a positional shorthand, never repeated).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

/// built-in color names, as `#RRGGBB`. Wire hues follow the app's
/// channel palette so profiles and the fallback look alike.
pub const NAMED_COLORS: [(&str, [u8; 3]); 13] = [
    ("black", [0x00, 0x00, 0x00]),
    ("brown", [0xC7, 0x95, 0x79]),
    ("red", [0xFF, 0x6D, 0x7F]),
    ("orange", [0xFF, 0xB4, 0x5B]),
    ("yellow", [0xE8, 0xD8, 0x36]),
    ("green", [0x58, 0xC6, 0x67]),
    ("blue", [0x53, 0xA9, 0xFD]),
    ("violet", [0xAF, 0x92, 0xFB]),
    ("gray", [0x9E, 0x9E, 0x9E]),
    ("grey", [0x9E, 0x9E, 0x9E]),
    ("white", [0xF2, 0xF2, 0xF2]),
    ("pink", [0xF4, 0x8F, 0xB1]),
    ("cyan", [0x4D, 0xD0, 0xE1]),
];

/// The fallback palette's names by `index % 8`, for seeding a
/// new profile (*Create profile*).
pub const FALLBACK_NAMES: [&str; 8] = ["gray", "brown", "red", "orange", "yellow", "green", "blue", "violet"];

/// A color as written in the profile (`"brown"`, `"#AABBCC"` or a profile
/// name) and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Color {
    pub spec: String,
    pub rgb: [u8; 3],
}

impl Color {
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.rgb[0], self.rgb[1], self.rgb[2])
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn named_color(name: &str) -> Option<[u8; 3]> {
    let n = name.to_ascii_lowercase();
    NAMED_COLORS.iter().find(|(k, _)| *k == n).map(|(_, c)| *c)
}

/// WCAG contrast ratio between two colors.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let lum = |c: [u8; 3]| {
        let ch = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
    };
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Below this ratio against the background, draw with a neutral
/// color and outline swatches.
pub const MIN_CONTRAST: f64 = 1.5;

// ---- file format ----

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Match {
    pub driver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum ChannelEntry {
    Name(String),
    Full {
        channel: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wire: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        marker: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct GroupFile {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    marker: Option<String>,
    channels: Vec<ChannelEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wires: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ProfileFile {
    schema_version: u32,
    name: String,
    #[serde(rename = "match")]
    matcher: Match,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    colors: BTreeMap<String, String>,
    groups: Vec<GroupFile>,
}

// ---- resolved model ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub marker: Option<Color>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeChannel {
    /// Backend channel name (e.g. `D3`, `CLK1`).
    pub name: String,
    pub group: usize,
    pub wire: Option<Color>,
    /// The channel's own marker; `None` falls back to the group's.
    pub marker: Option<Color>,
    /// Default row name.
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeProfile {
    pub name: String,
    pub matcher: Match,
    /// The profile's own color names.
    pub colors: BTreeMap<String, [u8; 3]>,
    pub groups: Vec<Group>,
    /// In group order, then listing order within the group.
    pub channels: Vec<ProbeChannel>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("unsupported schema_version {0}")]
    Schema(u32),
    #[error("unknown color {0:?}")]
    UnknownColor(String),
    #[error("channel {0:?} is listed twice")]
    Duplicate(String),
    #[error("group {group:?}: `wires` has {wires} entries for {channels} channels")]
    WiresLength { group: String, wires: usize, channels: usize },
    #[error("`match.driver` is empty")]
    NoDriver,
}

/// What a device offers, for matching.
#[derive(Copy, Clone, Debug)]
pub struct DeviceKey<'a> {
    pub driver: &'a str,
    pub vendor: &'a str,
    pub model: &'a str,
    pub channels: usize,
}

impl ProbeProfile {
    /// Resolves a color spec: `#RRGGBB`, a profile name, or a built-in name.
    pub fn color(&self, spec: &str) -> Result<Color, ProfileError> {
        let rgb = parse_hex(spec)
            .or_else(|| self.colors.get(spec).copied())
            .or_else(|| named_color(spec))
            .ok_or_else(|| ProfileError::UnknownColor(spec.to_owned()))?;
        Ok(Color { spec: spec.to_owned(), rgb })
    }

    pub fn parse(json: &str) -> Result<Self, ProfileError> {
        let f: ProfileFile = serde_json::from_str(json).map_err(|e| ProfileError::Json(e.to_string()))?;
        if f.schema_version != SCHEMA_VERSION {
            return Err(ProfileError::Schema(f.schema_version));
        }
        if f.matcher.driver.trim().is_empty() {
            return Err(ProfileError::NoDriver);
        }
        let mut p = ProbeProfile {
            name: f.name,
            matcher: f.matcher,
            colors: BTreeMap::new(),
            groups: Vec::new(),
            channels: Vec::new(),
        };
        for (k, v) in &f.colors {
            let rgb = parse_hex(v).ok_or_else(|| ProfileError::UnknownColor(v.clone()))?;
            p.colors.insert(k.clone(), rgb);
        }
        let mut seen = std::collections::HashSet::new();
        for g in f.groups {
            if let Some(w) = &g.wires
                && w.len() != g.channels.len()
            {
                return Err(ProfileError::WiresLength { group: g.name, wires: w.len(), channels: g.channels.len() });
            }
            let marker = g.marker.as_deref().map(|m| p.color(m)).transpose()?;
            let gi = p.groups.len();
            p.groups.push(Group { name: g.name, marker });
            for (i, e) in g.channels.into_iter().enumerate() {
                let positional = g.wires.as_ref().map(|w| w[i].clone());
                let (name, wire, marker, label) = match e {
                    ChannelEntry::Name(n) => (n, positional, None, None),
                    ChannelEntry::Full { channel, wire, marker, label } => {
                        (channel, wire.or(positional), marker, label)
                    }
                };
                if !seen.insert(name.clone()) {
                    return Err(ProfileError::Duplicate(name));
                }
                let wire = wire.as_deref().map(|w| p.color(w)).transpose()?;
                let marker = marker.as_deref().map(|m| p.color(m)).transpose()?;
                p.channels.push(ProbeChannel { name, group: gi, wire, marker, label });
            }
        }
        Ok(p)
    }

    /// Canonical JSON: one object per channel, no positional shorthand.
    pub fn to_json(&self) -> String {
        let file = ProfileFile {
            schema_version: SCHEMA_VERSION,
            name: self.name.clone(),
            matcher: self.matcher.clone(),
            colors: self.colors.iter().map(|(k, v)| (k.clone(), Color { spec: k.clone(), rgb: *v }.hex())).collect(),
            groups: self
                .groups
                .iter()
                .enumerate()
                .map(|(gi, g)| GroupFile {
                    name: g.name.clone(),
                    marker: g.marker.as_ref().map(|c| c.spec.clone()),
                    channels: self
                        .channels
                        .iter()
                        .filter(|c| c.group == gi)
                        .map(|c| ChannelEntry::Full {
                            channel: c.name.clone(),
                            wire: c.wire.as_ref().map(|w| w.spec.clone()),
                            marker: c.marker.as_ref().map(|m| m.spec.clone()),
                            label: c.label.clone(),
                        })
                        .collect(),
                    wires: None,
                })
                .collect(),
        };
        serde_json::to_string_pretty(&file).expect("serializable")
    }

    /// `Some(specificity)` when every given field matches.
    pub fn matches(&self, d: &DeviceKey<'_>) -> Option<usize> {
        let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        let m = &self.matcher;
        if !eq(&m.driver, d.driver) {
            return None;
        }
        let mut n = 1;
        for (want, have) in [(&m.vendor, d.vendor), (&m.model, d.model)] {
            if let Some(w) = want {
                if !eq(w, have) {
                    return None;
                }
                n += 1;
            }
        }
        if let Some(c) = m.channels {
            if c != d.channels {
                return None;
            }
            n += 1;
        }
        Some(n)
    }

    pub fn channel(&self, name: &str) -> Option<&ProbeChannel> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// The channel's marker: its own, else its group's.
    pub fn marker<'a>(&'a self, c: &'a ProbeChannel) -> Option<&'a Color> {
        c.marker.as_ref().or(self.groups.get(c.group).and_then(|g| g.marker.as_ref()))
    }

    /// *Create profile*: fallback colors, one group per 8 channels,
    /// no markers. `channels` are `(backend name, index)` pairs.
    pub fn seed(name: &str, matcher: Match, channels: &[(String, u16)]) -> Self {
        let mut p = ProbeProfile {
            name: name.to_owned(),
            matcher,
            colors: BTreeMap::new(),
            groups: Vec::new(),
            channels: Vec::new(),
        };
        for (k, chunk) in channels.chunks(8).enumerate() {
            let label = match (chunk.first(), chunk.last()) {
                (Some(a), Some(b)) if chunk.len() > 1 => format!("{}–{}", a.0, b.0),
                (Some(a), _) => a.0.clone(),
                _ => format!("Group {}", k + 1),
            };
            p.groups.push(Group { name: label, marker: None });
            for (n, idx) in chunk {
                let spec = FALLBACK_NAMES[*idx as usize % 8];
                let wire = Some(Color { spec: spec.to_owned(), rgb: named_color(spec).unwrap() });
                p.channels.push(ProbeChannel { name: n.clone(), group: k, wire, marker: None, label: None });
            }
        }
        p
    }
}

/// Picks the best profile for a device: highest specificity, user before
/// built-in on ties. `profiles` are `(profile, is_user)`.
pub fn select<'a>(
    profiles: impl IntoIterator<Item = (&'a ProbeProfile, bool)>,
    d: &DeviceKey<'_>,
) -> Option<(&'a ProbeProfile, bool)> {
    profiles
        .into_iter()
        .filter_map(|(p, user)| p.matches(d).map(|s| ((s, user), p, user)))
        .max_by_key(|(k, _, _)| *k)
        .map(|(_, p, u)| (p, u))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGICPORT: &str = r##"{
      "schema_version": 1,
      "name": "Intronix LogicPort",
      "match": { "driver": "intronix-logicport" },
      "colors": { "probe-white": "#ECECEC" },
      "groups": [
        { "name": "D0–D7", "marker": "white",
          "channels": ["D0","D1","D2","D3","D4","D5","D6","D7"],
          "wires": ["black","brown","red","orange","yellow","green","blue","violet"] },
        { "name": "Clocks",
          "channels": [ { "channel": "CLK1", "wire": "probe-white", "marker": "blue", "label": "Clock 1" },
                        { "channel": "CLK2", "wire": "#FFFFFF", "marker": "green" } ] }
      ]
    }"##;

    fn key(driver: &str, model: &str, channels: usize) -> DeviceKey<'static> {
        let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
        DeviceKey { driver: leak(driver), vendor: "Intronix", model: leak(model), channels }
    }

    #[test]
    fn parses_shorthand_and_objects() {
        let p = ProbeProfile::parse(LOGICPORT).unwrap();
        assert_eq!(p.groups.len(), 2);
        let d1 = p.channel("D1").unwrap();
        assert_eq!(d1.wire.as_ref().unwrap().spec, "brown");
        assert_eq!(p.marker(d1).unwrap().spec, "white", "group marker");
        let clk1 = p.channel("CLK1").unwrap();
        assert_eq!(clk1.wire.as_ref().unwrap().rgb, [0xEC, 0xEC, 0xEC], "profile color name");
        assert_eq!(p.marker(clk1).unwrap().spec, "blue", "own marker beats the group's");
        assert_eq!(clk1.label.as_deref(), Some("Clock 1"));
        assert_eq!(p.channel("CLK2").unwrap().wire.as_ref().unwrap().rgb, [0xFF; 3]);
        assert!(p.channel("D9").is_none());
    }

    #[test]
    fn canonical_json_round_trips() {
        let p = ProbeProfile::parse(LOGICPORT).unwrap();
        let json = p.to_json();
        assert!(!json.contains("\"wires\""), "no shorthand in saved files");
        assert_eq!(ProbeProfile::parse(&json).unwrap(), p);
    }

    #[test]
    fn validation_errors() {
        let bad = |json: &str| ProbeProfile::parse(json).unwrap_err();
        let base =
            |groups: &str| format!(r#"{{"schema_version":1,"name":"x","match":{{"driver":"d"}},"groups":[{groups}]}}"#);
        assert_eq!(
            bad(&base(r#"{"name":"g","channels":["D0"],"wires":["mauve"]}"#)),
            ProfileError::UnknownColor("mauve".into())
        );
        assert_eq!(bad(&base(r#"{"name":"g","channels":["D0","D0"]}"#)), ProfileError::Duplicate("D0".into()));
        assert!(matches!(
            bad(&base(r#"{"name":"g","channels":["D0","D1"],"wires":["red"]}"#)),
            ProfileError::WiresLength { wires: 1, channels: 2, .. }
        ));
        assert!(matches!(bad("{"), ProfileError::Json(_)));
        assert_eq!(
            bad(r#"{"schema_version":2,"name":"x","match":{"driver":"d"},"groups":[]}"#),
            ProfileError::Schema(2)
        );
        assert_eq!(
            bad(r#"{"schema_version":1,"name":"x","match":{"driver":" "},"groups":[]}"#),
            ProfileError::NoDriver
        );
    }

    #[test]
    fn matching_and_selection() {
        let generic = ProbeProfile::parse(LOGICPORT).unwrap();
        let mut specific = generic.clone();
        specific.matcher.model = Some("LogicPort".into());
        let mut user_generic = generic.clone();
        user_generic.name = "mine".into();

        let lp = key("intronix-logicport", "logicport", 34);
        assert_eq!(generic.matches(&lp), Some(1));
        assert_eq!(specific.matches(&lp), Some(2), "case-insensitive");
        assert_eq!(generic.matches(&key("fx2lafw", "x", 8)), None);
        assert_eq!(specific.matches(&key("intronix-logicport", "Other", 34)), None);

        // Most specific wins; at equal specificity the user profile wins.
        let all = [(&generic, false), (&specific, false), (&user_generic, true)];
        assert_eq!(select(all, &lp).unwrap().0.matcher.model.as_deref(), Some("LogicPort"));
        let all = [(&generic, false), (&user_generic, true)];
        assert_eq!(select(all, &lp).unwrap(), (&user_generic, true));
    }

    #[test]
    fn seed_uses_fallback_and_groups_of_8() {
        let chans: Vec<(String, u16)> = (0..10).map(|i| (format!("D{i}"), i)).collect();
        let p = ProbeProfile::seed("new", Match { driver: "mock".into(), ..Match::default() }, &chans);
        assert_eq!(p.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["D0–D7", "D8–D9"]);
        assert_eq!(p.channel("D9").unwrap().wire.as_ref().unwrap().spec, "brown");
        assert_eq!(ProbeProfile::parse(&p.to_json()).unwrap(), p);
    }

    #[test]
    fn contrast() {
        let dark_bg = [0x14, 0x14, 0x15];
        assert!(contrast_ratio(named_color("black").unwrap(), dark_bg) < MIN_CONTRAST);
        assert!(contrast_ratio(named_color("brown").unwrap(), dark_bg) > MIN_CONTRAST);
        assert!(contrast_ratio(named_color("white").unwrap(), [0xF5; 3]) < MIN_CONTRAST);
        assert!((contrast_ratio([0; 3], [255; 3]) - 21.0).abs() < 1e-9);
    }
}

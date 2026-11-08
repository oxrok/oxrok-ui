//! Device-level capture settings shared by the backend, the settings file
//! and capture files: the logic threshold and generic device
//! options.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdChoice {
    /// One of the device's options, by its backend id.
    Preset(String),
    /// A voltage.
    Custom(f64),
}

/// Where a device option lives: the device, or one of its channel groups.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub enum OptionScope {
    Device,
    Group(String),
}

impl OptionScope {
    /// The segment label: "Device" or the group's name.
    pub fn label(&self) -> &str {
        match self {
            OptionScope::Device => crate::l10n::tr("Device"),
            OptionScope::Group(g) => g,
        }
    }
}

/// Stored as a JSON map key: `device` or `group:‹name›`.
impl From<OptionScope> for String {
    fn from(s: OptionScope) -> String {
        match s {
            OptionScope::Device => "device".into(),
            OptionScope::Group(g) => format!("group:{g}"),
        }
    }
}

impl From<String> for OptionScope {
    fn from(s: String) -> Self {
        match s.strip_prefix("group:") {
            Some(g) => OptionScope::Group(g.to_owned()),
            None => OptionScope::Device,
        }
    }
}

/// A driver setting's value, by libsigrok data type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfValue {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    /// `p/q`, e.g. a timebase of 1/2 s.
    Rational(u64, u64),
    /// A (low, high) pair of floats, e.g. a voltage threshold.
    Pair(f64, f64),
    /// A (low, high) pair of integers.
    UPair(u64, u64),
}

impl ConfValue {
    /// The numeric value, for range inputs and unit formatting.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            ConfValue::Int(v) => Some(v as f64),
            ConfValue::UInt(v) => Some(v as f64),
            ConfValue::Float(v) => Some(v),
            ConfValue::Rational(p, q) if q != 0 => Some(p as f64 / q as f64),
            _ => None,
        }
    }

    /// `v` in this value's numeric type, rounded for integers.
    pub fn with_f64(&self, v: f64) -> ConfValue {
        match self {
            ConfValue::Int(_) => ConfValue::Int(v.round() as i64),
            ConfValue::UInt(_) => ConfValue::UInt(v.round().max(0.0) as u64),
            _ => ConfValue::Float(v),
        }
    }

    /// As sigrok-cli reads it in `-c key=value`.
    pub fn cli(&self) -> String {
        match self {
            ConfValue::Bool(b) => b.to_string(),
            ConfValue::Int(v) => v.to_string(),
            ConfValue::UInt(v) => v.to_string(),
            ConfValue::Float(v) => v.to_string(),
            ConfValue::Str(s) => s.clone(),
            ConfValue::Rational(p, q) => format!("{p}/{q}"),
            ConfValue::Pair(a, b) => format!("{a}-{b}"),
            ConfValue::UPair(a, b) => format!("{a}-{b}"),
        }
    }
}

/// Per scope, the options whose value differs from the
/// driver's default, by libsigrok key id (e.g. `external_clock`).
pub type DeviceOptions = BTreeMap<OptionScope, BTreeMap<String, ConfValue>>;

/// A key id in title case, for rows without driver metadata
/// (`external_clock` → "External clock").
pub fn title_case(id: &str) -> String {
    let words = id.replace(['_', '-'], " ");
    let mut c = words.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_keys_round_trip() {
        let mut m: DeviceOptions = BTreeMap::new();
        m.entry(OptionScope::Device).or_default().insert("external_clock".into(), ConfValue::Bool(true));
        m.entry(OptionScope::Group("Analog".into()))
            .or_default()
            .insert("coupling".into(), ConfValue::Str("AC".into()));
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#"{"device":{"external_clock":{"bool":true}},"group:Analog":{"coupling":{"str":"AC"}}}"#);
        assert_eq!(serde_json::from_str::<DeviceOptions>(&json).unwrap(), m);
    }

    #[test]
    fn values() {
        assert_eq!(ConfValue::UInt(256).with_f64(300.4), ConfValue::UInt(300));
        assert_eq!(ConfValue::Rational(1, 2).as_f64(), Some(0.5));
        assert_eq!(ConfValue::Pair(1.5, 1.5).cli(), "1.5-1.5");
        assert_eq!(title_case("external_clock"), "External clock");
        assert_eq!(serde_json::to_string(&ThresholdChoice::Custom(1.25)).unwrap(), r#"{"custom":1.25}"#);
    }
}

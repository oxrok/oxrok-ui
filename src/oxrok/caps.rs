//! `crate::backend` capabilities, options and thresholds, read from an oxrok
//! device ().
//!
//! The key ids are sigrok's, so the mapping is the same as the one the
//! libsigrok backend made from `sr_config_list`/`sr_config_get`.

use crate::backend::{
    ConfValue, DeviceCaps, DeviceOption, DeviceOptions, HwTriggerCaps, NumRange, OptionGroup, OptionKind, OptionScope,
    RateCoupling, Rates, Rejected, ThresholdCaps, ThresholdOption, TriggerMatch, Unit,
};
use crate::core::device::title_case;
use crate::core::{ChannelId, ChannelKind};
use oxrok::core::{Device, Key, OptionDesc, Value, ValueKind, ValueList};

/// More voltages than this, evenly spaced, make a range.
const LONG_LIST: usize = 16;

/// The keys the capture and settings panels own, or that are not capture
/// settings at all. Ids, so driver-specific keys pass.
const EXCLUDED: &[&str] = &[
    "samplerate",
    "captureratio",
    "rle",
    "triggermatch",
    "voltage_threshold",
    "logic_threshold",
    "logic_threshold_custom",
    "logic_channels",
    "analog_channels",
    "enabled",
    "sessionfile",
    "capturefile",
    "capture_unitsize",
    "power_off",
    "limit_time",
    "limit_samples",
    "limit_frames",
    "continuous",
];

fn excluded(key: &Key) -> bool {
    EXCLUDED.contains(&key.id())
}

/// The options of `group` with their key, when the device has them.
pub(crate) fn options_of(dev: &dyn Device, group: Option<&str>) -> Vec<OptionDesc> {
    dev.options(group).unwrap_or_default()
}

/// An option descriptor of `key`, when the device lists it.
#[allow(dead_code)]
fn access(dev: &dyn Device, group: Option<&str>, key: &Key) -> Option<OptionDesc> {
    options_of(dev, group).into_iter().find(|o| &o.key == key)
}

/// An oxrok value as a stored setting.
fn conf(v: &Value) -> Option<ConfValue> {
    Some(match v {
        Value::Bool(b) => ConfValue::Bool(*b),
        Value::U64(v) => ConfValue::UInt(*v),
        Value::I32(v) => ConfValue::Int(i64::from(*v)),
        Value::F64(v) => ConfValue::Float(*v),
        Value::Str(s) => ConfValue::Str(s.clone()),
        Value::Rational(p, q) => ConfValue::Rational(*p, *q),
        Value::RangeU64(a, b) => ConfValue::UPair(*a, *b),
        Value::RangeF64(a, b) => ConfValue::Pair(*a, *b),
        Value::KeyValue(..) | Value::Mq(..) => return None,
    })
}

/// A stored setting as an oxrok value, in the type of `like` when the
/// device already reported one.
fn value(v: &ConfValue, like: Option<&Value>) -> Value {
    let num = v.as_f64();
    match like {
        Some(Value::Bool(_)) => Value::Bool(matches!(v, ConfValue::Bool(true)) || num.is_some_and(|n| n != 0.0)),
        Some(Value::U64(_)) => Value::U64(num.unwrap_or(0.0).round().max(0.0) as u64),
        Some(Value::I32(_)) => Value::I32(num.unwrap_or(0.0).round() as i32),
        Some(Value::F64(_)) => Value::F64(num.unwrap_or(0.0)),
        Some(Value::Str(_)) => Value::Str(v.cli()),
        Some(Value::Rational(..)) => match v {
            ConfValue::Rational(p, q) => Value::Rational(*p, *q),
            _ => Value::Rational(num.unwrap_or(0.0).round().max(0.0) as u64, 1),
        },
        Some(Value::RangeU64(..)) => match v {
            ConfValue::UPair(a, b) => Value::RangeU64(*a, *b),
            _ => {
                Value::RangeU64(num.unwrap_or(0.0).round().max(0.0) as u64, num.unwrap_or(0.0).round().max(0.0) as u64)
            }
        },
        Some(Value::RangeF64(..)) => match v {
            ConfValue::Pair(a, b) => Value::RangeF64(*a, *b),
            _ => Value::RangeF64(num.unwrap_or(0.0), num.unwrap_or(0.0)),
        },
        _ => match v {
            ConfValue::Bool(b) => Value::Bool(*b),
            ConfValue::Int(i) => Value::I32(*i as i32),
            ConfValue::UInt(u) => Value::U64(*u),
            ConfValue::Float(f) => Value::F64(*f),
            ConfValue::Str(s) => Value::Str(s.clone()),
            ConfValue::Rational(p, q) => Value::Rational(*p, *q),
            ConfValue::Pair(a, b) => Value::RangeF64(*a, *b),
            ConfValue::UPair(a, b) => Value::RangeU64(*a, *b),
        },
    }
}

fn same(a: &ConfValue, b: &ConfValue) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= f64::EPSILON * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    }
}

/// How a setting is edited.
fn kind_from_list(list: &ValueList) -> OptionKind {
    let floats = |v: &[f64]| OptionKind::Choice(v.iter().map(|f| ConfValue::Float(*f)).collect());
    let ints = |v: &[u64]| OptionKind::Choice(v.iter().map(|i| ConfValue::UInt(*i)).collect());
    match list {
        ValueList::U64(v) if v.is_empty() => OptionKind::Free,
        ValueList::U64(v) => ints(v),
        ValueList::F64(v) if v.is_empty() => OptionKind::Free,
        ValueList::F64(v) => floats(v),
        ValueList::Str(v) if v.is_empty() => OptionKind::Free,
        ValueList::Str(v) => OptionKind::Choice(v.iter().map(|s| ConfValue::Str(s.clone())).collect()),
        ValueList::Rational(v) if v.is_empty() => OptionKind::Free,
        ValueList::Rational(v) => OptionKind::Choice(v.iter().map(|(p, q)| ConfValue::Rational(*p, *q)).collect()),
        ValueList::RangeStepU64 { min, max, step } => {
            OptionKind::Range(NumRange { min: *min as f64, max: *max as f64, step: *step as f64 })
        }
        ValueList::RangeStepF64 { min, max, step } => OptionKind::Range(NumRange { min: *min, max: *max, step: *step }),
        ValueList::RangeF64(v) if v.is_empty() => OptionKind::Free,
        ValueList::RangeF64(v) => OptionKind::Choice(v.iter().map(|(a, b)| ConfValue::Pair(*a, *b)).collect()),
        ValueList::RangeU64(v) if v.is_empty() => OptionKind::Free,
        ValueList::RangeU64(v) => OptionKind::Choice(v.iter().map(|(a, b)| ConfValue::UPair(*a, *b)).collect()),
        ValueList::MinMaxU64(..) | ValueList::TriggerMatches(_) | ValueList::Mq(_) => OptionKind::Free,
    }
}

/// The value type of a key, for options that cannot list their values.
fn bool_key(key: &Key) -> bool {
    matches!(key.kind(), ValueKind::Bool)
}

/// Every shown option of every scope, with current values.
/// `defaults` gives the first-seen values (filled in on the first call).
pub(crate) fn option_groups(
    dev: &dyn Device,
    defaults: &mut Option<std::collections::BTreeMap<(OptionScope, String), ConfValue>>,
) -> Vec<OptionGroup> {
    let mut groups = Vec::new();
    let mut scopes = vec![(OptionScope::Device, None)];
    scopes.extend(dev.channel_groups().iter().map(|g| (OptionScope::Group(g.name.clone()), Some(g.name.as_str()))));
    for (scope, group) in scopes {
        let mut options = Vec::new();
        for desc in options_of(dev, group) {
            let key = &desc.key;
            if excluded(key) {
                continue;
            }
            let (can_get, can_set) = (desc.access.get, desc.access.set);
            if !can_get && !can_set {
                continue;
            }
            if matches!(key.kind(), ValueKind::Mq | ValueKind::KeyValue) {
                continue;
            }
            let value = if can_get { dev.get(key, group).ok().as_ref().and_then(conf) } else { None };
            if value.is_none() && !can_set {
                continue;
            }
            let kind = if bool_key(key) {
                OptionKind::Bool
            } else if desc.access.list {
                dev.list(key, group).map_or(OptionKind::Free, |l| kind_from_list(&l))
            } else {
                OptionKind::Free
            };
            let id = key.id().to_owned();
            let name = {
                let n = key.name();
                if n.is_empty() { title_case(&id) } else { n.to_owned() }
            };
            let description = match key {
                Key::Custom(c) => c.description.to_owned(),
                _ => String::new(),
            };
            options.push(DeviceOption {
                id,
                name,
                description,
                kind,
                unit: Unit::for_key(key.id()),
                value,
                default: None,
                writable: can_set,
            });
        }
        if !options.is_empty() {
            groups.push(OptionGroup { scope, options });
        }
    }
    let defaults = defaults.get_or_insert_with(|| {
        groups
            .iter()
            .flat_map(|g| g.options.iter().filter_map(|o| Some(((g.scope.clone(), o.id.clone()), o.value.clone()?))))
            .collect()
    });
    for g in &mut groups {
        for o in &mut g.options {
            o.default = defaults.get(&(g.scope.clone(), o.id.clone())).cloned();
        }
    }
    groups
}

/// Sets every wanted value that differs, then reads every option
/// back. A refused key keeps the device's value.
pub(crate) fn apply_options(dev: &mut dyn Device, wanted: &DeviceOptions) -> (DeviceOptions, Vec<Rejected>) {
    let mut read = DeviceOptions::new();
    let mut rejected = Vec::new();
    let mut scopes: Vec<(OptionScope, Option<String>)> = vec![(OptionScope::Device, None)];
    scopes.extend(dev.channel_groups().iter().map(|g| (OptionScope::Group(g.name.clone()), Some(g.name.clone()))));
    for (scope, group) in scopes {
        let group = group.as_deref();
        let want = wanted.get(&scope);
        for desc in options_of(dev, group) {
            let key = &desc.key;
            if excluded(key) {
                continue;
            }
            let id = key.id();
            let before = if desc.access.get { dev.get(key, group).ok().and_then(|v| conf(&v)) } else { None };
            if desc.access.set
                && let Some(v) = want.and_then(|w| w.get(id))
            {
                let changed = before.as_ref() != Some(v) && !before.as_ref().is_some_and(|b| same(b, v));
                if changed {
                    let like = dev.get(key, group).ok();
                    if dev.set(key, &value(v, like.as_ref()), group).is_err() {
                        rejected.push(Rejected {
                            scope: scope.clone(),
                            id: id.to_owned(),
                            requested: v.clone(),
                            kept: before.clone(),
                        });
                    }
                }
            }
            if desc.access.get
                && let Some(v) = dev.get(key, group).ok().as_ref().and_then(conf)
            {
                read.entry(scope.clone()).or_default().insert(id.to_owned(), v);
            }
        }
    }
    (read, rejected)
}

/// The sample rates a device offers.
pub(crate) fn samplerates(dev: &dyn Device) -> Rates {
    match dev.list(&Key::Samplerate, None) {
        Ok(ValueList::U64(v)) => Rates::List(v),
        Ok(ValueList::RangeStepU64 { min, max, step }) => Rates::Range { min, max, step },
        Ok(ValueList::MinMaxU64(min, max)) => Rates::Range { min, max, step: 1 },
        _ => Rates::List(Vec::new()),
    }
}

/// The trigger matches the device lists.
pub(crate) fn trigger_caps(dev: &dyn Device, capture_ratio: bool) -> Option<HwTriggerCaps> {
    let matches: Vec<TriggerMatch> = match dev.list(&Key::TriggerMatch, None) {
        Ok(ValueList::TriggerMatches(v)) => v
            .into_iter()
            .filter_map(|m| match m {
                oxrok::core::TriggerMatch::Zero => Some(TriggerMatch::Zero),
                oxrok::core::TriggerMatch::One => Some(TriggerMatch::One),
                oxrok::core::TriggerMatch::Rising => Some(TriggerMatch::Rising),
                oxrok::core::TriggerMatch::Falling => Some(TriggerMatch::Falling),
                oxrok::core::TriggerMatch::Edge => Some(TriggerMatch::Edge),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let f = dev.trigger_features();
    (!matches.is_empty()).then_some(HwTriggerCaps {
        matches,
        capture_ratio,
        pulse: f.duration && f.pulse,
        max_stages: f.max_stages,
        count: f.count,
        consecutive: f.count && f.consecutive,
        duration: f.duration,
        range: f.range,
        or: f.or,
        and: f.and,
    })
}

/// Everything the settings panel needs, except the
/// options and thresholds, which go through [`option_groups`] and
/// [`thresholds`].
pub(crate) fn capabilities(dev: &dyn Device, channels: &[(ChannelId, usize)]) -> DeviceCaps {
    let descs = options_of(dev, None);
    let listed = |id: &str| descs.iter().any(|o| o.key.id() == id);
    let setting = |id: &str| descs.iter().any(|o| o.key.id() == id && o.access.set);
    let rates = samplerates(dev);
    let read_state = !matches!(&rates, Rates::List(l) if l.is_empty()) && setting("limit_samples");
    let analog_ranges =
        channels.iter().filter(|(id, _)| id.kind == ChannelKind::Analog).map(|(id, _)| (*id, None)).collect();
    DeviceCaps {
        samplerates: rates,
        rates_coupled: RateCoupling::Shared,
        max_limit_samples: None,
        // Listed at all means the device can stream.
        supports_continuous: listed("continuous"),
        supports_limit_msec: setting("limit_time"),
        rle: setting("rle"),
        thresholds: None,
        threshold: None,
        options: Vec::new(),
        hw_trigger: trigger_caps(dev, setting("captureratio")),
        analog_ranges,
        // A 1-sample capture needs rates and a sample limit.
        supports_state_read: read_state,
    }
}

/// `x.x V`.
fn volts(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0');
    let s = if s.ends_with('.') { format!("{s}0") } else { s.to_owned() };
    format!("{s} V")
}

fn pair_id(lo: f64, hi: f64) -> String {
    format!("{lo}:{hi}")
}

fn parse_pair(id: &str) -> Option<(f64, f64)> {
    let (a, b) = id.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// A `USER…` entry selects the custom level on drivers that have one.
fn is_user_entry(s: &str) -> bool {
    s.to_ascii_uppercase().starts_with("USER")
}

/// A long, evenly spaced list of single voltages as a range.
fn as_range(pairs: &[(f64, f64)]) -> Option<NumRange> {
    if pairs.len() <= LONG_LIST || pairs.iter().any(|(a, b)| a != b) {
        return None;
    }
    let step = pairs[1].0 - pairs[0].0;
    let even = step > 0.0 && pairs.windows(2).all(|w| ((w[1].0 - w[0].0) - step).abs() < step * 0.01);
    // Drivers accumulate the step, so the ends drift (5.999999999999974).
    let round = |v: f64| (v * 1e6).round() / 1e6;
    even.then(|| NumRange { min: round(pairs[0].0), max: round(pairs[pairs.len() - 1].0), step: round(step) })
}

/// The voltage pairs of `VoltageThreshold`.
fn voltage_pairs(dev: &dyn Device) -> Vec<(f64, f64)> {
    match dev.list(&Key::VoltageThreshold, None) {
        Ok(ValueList::RangeStepF64 { min, max, step }) => {
            if step <= 0.0 || max < min {
                return vec![(min, min)];
            }
            let n = ((max - min) / step).round() as u64;
            (0..=n).map(|i| (min + i as f64 * step, min + i as f64 * step)).collect()
        }
        Ok(ValueList::F64(v)) => v.into_iter().map(|x| (x, x)).collect(),
        Ok(ValueList::RangeF64(v)) => v,
        _ => Vec::new(),
    }
}

/// What the device offers and its current choice.
pub(crate) fn thresholds(dev: &dyn Device) -> (Option<ThresholdCaps>, Option<crate::core::device::ThresholdChoice>) {
    use crate::core::device::ThresholdChoice;
    let descs = options_of(dev, None);
    let cap = |id: &str| descs.iter().find(|o| o.key.id() == id);
    if let Some(d) = cap("logic_threshold")
        && d.access.list
    {
        let names = match dev.list(&Key::LogicThreshold, None) {
            Ok(ValueList::Str(v)) => v,
            _ => Vec::new(),
        };
        let custom = cap("logic_threshold_custom").filter(|d| d.access.set).map(|_| {
            match dev.list(&Key::LogicThresholdCustom, None) {
                Ok(ValueList::RangeStepF64 { min, max, step }) => NumRange { min, max, step },
                _ => NumRange { min: -10.0, max: 10.0, step: 0.01 },
            }
        });
        let options = names
            .iter()
            .filter(|n| !is_user_entry(n))
            .map(|n| ThresholdOption { id: n.clone(), label: n.clone() })
            .collect::<Vec<_>>();
        let current = match dev.get(&Key::LogicThreshold, None) {
            Ok(Value::Str(s)) if is_user_entry(&s) => {
                dev.get(&Key::LogicThresholdCustom, None).ok().and_then(|v| v.as_f64()).map(ThresholdChoice::Custom)
            }
            Ok(Value::Str(s)) => Some(ThresholdChoice::Preset(s)),
            _ => None,
        };
        if !options.is_empty() || custom.is_some() {
            return (Some(ThresholdCaps { options, custom }), current);
        }
    }
    if let Some(d) = cap("voltage_threshold")
        && d.access.list
    {
        let pairs = voltage_pairs(dev);
        if pairs.is_empty() {
            return (None, None);
        }
        let now = match dev.get(&Key::VoltageThreshold, None) {
            Ok(Value::RangeF64(a, b)) => Some((a, b)),
            _ => None,
        };
        if let Some(range) = as_range(&pairs) {
            let caps = ThresholdCaps { options: Vec::new(), custom: Some(range) };
            return (Some(caps), now.map(|(a, b)| ThresholdChoice::Custom((a + b) / 2.0)));
        }
        let options = pairs
            .iter()
            .map(|&(a, b)| ThresholdOption {
                id: pair_id(a, b),
                label: if a == b { volts(a) } else { format!("{}–{}", volts(a).trim_end_matches(" V"), volts(b)) },
            })
            .collect();
        let current = now.map(|(a, b)| ThresholdChoice::Preset(pair_id(a, b)));
        return (Some(ThresholdCaps { options, custom: None }), current);
    }
    (None, None)
}

/// Apply: the choice through whichever key the device has.
pub(crate) fn apply_threshold(
    dev: &mut dyn Device,
    choice: &crate::core::device::ThresholdChoice,
) -> Result<(), crate::backend::BackendError> {
    use crate::core::device::ThresholdChoice;
    let descs = options_of(dev, None);
    let settable = |id: &str| descs.iter().any(|o| o.key.id() == id && o.access.set);
    let err = |e: oxrok::core::Error| crate::backend::BackendError::Device(e.to_string());
    match choice {
        ThresholdChoice::Preset(id) => match parse_pair(id) {
            Some((a, b)) => dev.set(&Key::VoltageThreshold, &Value::RangeF64(a, b), None).map_err(err),
            None => dev.set(&Key::LogicThreshold, &Value::Str(id.clone()), None).map_err(err),
        },
        ThresholdChoice::Custom(v) => {
            if settable("logic_threshold") {
                let user = match dev.list(&Key::LogicThreshold, None) {
                    Ok(ValueList::Str(names)) => names.into_iter().find(|n| is_user_entry(n)),
                    _ => None,
                };
                if let Some(u) = user {
                    dev.set(&Key::LogicThreshold, &Value::Str(u), None).map_err(err)?;
                }
                dev.set(&Key::LogicThresholdCustom, &Value::F64(*v), None).map_err(err)
            } else {
                dev.set(&Key::VoltageThreshold, &Value::RangeF64(*v, *v), None).map_err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_voltage_lists_are_ranges() {
        let pairs: Vec<(f64, f64)> = (0..=240).map(|i| -6.0 + i as f64 * 0.05).map(|v| (v, v)).collect();
        let r = as_range(&pairs).unwrap();
        assert_eq!((r.min, r.max, r.step), (-6.0, 6.0, 0.05));
        assert!(as_range(&pairs[..5]).is_none());
        assert_eq!(volts(1.5), "1.5 V");
        assert_eq!(volts(1.25), "1.25 V");
        assert_eq!(volts(3.0), "3.0 V");
        assert_eq!(parse_pair(&pair_id(0.7, 1.4)), Some((0.7, 1.4)));
    }

    #[test]
    fn values_follow_the_devices_type() {
        assert_eq!(value(&ConfValue::Float(1.4), Some(&Value::U64(0))), Value::U64(1));
        assert_eq!(value(&ConfValue::UInt(3), Some(&Value::Str(String::new()))), Value::Str("3".into()));
        assert_eq!(value(&ConfValue::Pair(1.0, 2.0), None), Value::RangeF64(1.0, 2.0));
    }
}

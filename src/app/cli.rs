//! The `oxrok` command that reproduces a session's capture settings

use crate::app::session::Session;
use crate::backend::{ConfValue, DeviceCaps, ThresholdChoice};
use crate::core::capture::CaptureMode;
use crate::core::trigger::TriggerEvent;

/// Shown broken at this width, copied as one line.
pub const WRAP: usize = 80;

#[derive(Clone, Debug, PartialEq)]
pub struct CliCommand {
    /// Shell words, already quoted.
    words: Vec<String>,
    /// A trailing `# …` note.
    comment: Option<String>,
}

impl CliCommand {
    /// The copied form.
    pub fn one_line(&self) -> String {
        let mut s = self.words.join(" ");
        if let Some(c) = &self.comment {
            s.push_str(" # ");
            s.push_str(c);
        }
        s
    }

    /// The displayed form: options kept with their values, lines broken
    /// with ` \` before `width` columns and continued with two spaces.
    pub fn wrapped(&self, width: usize) -> String {
        // Pair each flag with its value so a break never separates them.
        let mut parts: Vec<String> = Vec::new();
        let mut i = 0;
        while i < self.words.len() {
            let w = &self.words[i];
            if w.starts_with('-') && w != "--continuous" && i + 1 < self.words.len() {
                parts.push(format!("{w} {}", self.words[i + 1]));
                i += 2;
            } else {
                parts.push(w.clone());
                i += 1;
            }
        }
        if let Some(c) = &self.comment {
            parts.push(format!("# {c}"));
        }
        let mut lines: Vec<String> = vec![String::new()];
        for p in parts {
            let line = lines.last_mut().unwrap();
            if line.trim().is_empty() {
                line.push_str(&p);
            } else if line.len() + 1 + p.len() + 2 <= width {
                line.push(' ');
                line.push_str(&p);
            } else {
                line.push_str(" \\");
                lines.push(format!("  {p}"));
            }
        }
        lines.join("\n")
    }
}

/// Quotes a word for a POSIX shell when it needs it.
pub fn quote(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-.,:=/+@%".contains(c));
    if plain {
        s.to_owned()
    } else {
        let escaped: String =
            s.chars().flat_map(|c| if "\"\\$`".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
        format!("\"{escaped}\"")
    }
}

/// The size suffixes oxrok shares with sigrok-cli: `1m`, `100k`, `24m`, `1g`.
pub fn rate(hz: u64) -> String {
    for (div, suffix) in [(1_000_000_000, "g"), (1_000_000, "m"), (1_000, "k")] {
        if hz >= div && hz.is_multiple_of(div) {
            return format!("{}{suffix}", hz / div);
        }
    }
    hz.to_string()
}

/// `key=value` for the threshold, by the key the device uses.
fn threshold(t: &ThresholdChoice, caps: Option<&DeviceCaps>) -> String {
    let range_only = caps.and_then(|c| c.thresholds.as_ref()).is_some_and(|t| t.options.is_empty());
    match t {
        ThresholdChoice::Preset(id) => match id.split_once(':') {
            Some((a, b)) => format!("voltage_threshold={a}-{b}"),
            None => format!("logic_threshold={id}"),
        },
        ThresholdChoice::Custom(v) if range_only => format!("voltage_threshold={v}-{v}"),
        ThresholdChoice::Custom(v) => format!("logic_threshold_custom={v}"),
    }
}

impl Session {
    /// A session opened from a file uses its capture snapshot.
    pub fn cli_command(&self) -> CliCommand {
        let mut st = self.settings.clone();
        if self.is_read_only()
            && let Some(snap) = &self.snapshot
        {
            st.mode = snap.mode;
            st.sample_rate = snap.sample_rate;
            st.enabled = snap.enabled.clone();
            st.timer_seconds = snap.timer_seconds;
            if let Some(t) = &snap.trigger {
                st.trigger = t.clone();
            }
            st.logic_threshold = snap.logic_threshold.clone();
            st.device_options = snap.device_options.clone();
        }
        let caps = self.caps.as_ref();
        let mut words = vec!["oxrok".to_owned(), "-d".into()];
        let d = &self.device;
        words.push(quote(&match &d.conn {
            Some(c) => format!("{}:conn={c}", d.driver),
            None if !d.is_demo && !d.serial_or_conn.is_empty() => format!("{}:serial={}", d.driver, d.serial_or_conn),
            None => d.driver.clone(),
        }));
        // Enabled channels in the device's order; renamed rows add `=name`.
        let channels: Vec<String> = d
            .channels
            .iter()
            .filter(|c| st.is_enabled(c.id))
            .map(|c| match self.row(c.id) {
                Some(r) if r.name != r.ident => format!("{}={}", c.device_name, r.name),
                _ => c.device_name.clone(),
            })
            .collect();
        // -t refers to channels by the name -C gave them: the alias if renamed.
        let cli_name = |id| {
            let c = d.channels.iter().find(|c| c.id == id)?;
            Some(match self.row(id) {
                Some(r) if r.name != r.ident => r.name.clone(),
                _ => c.device_name.clone(),
            })
        };
        if !channels.is_empty() {
            words.push("-C".into());
            words.push(quote(&channels.join(",")));
        }
        let mut device = vec![format!("samplerate={}", rate(st.sample_rate))];
        // The threshold, unless it is the one the device already had.
        if let Some(t) = &st.logic_threshold
            && caps.is_none_or(|c| c.threshold.as_ref() != Some(t))
        {
            device.push(threshold(t, caps));
        }
        let triggered = st.mode == CaptureMode::Triggered;
        let buffered = caps.is_some_and(|c| !c.supports_continuous);
        if triggered && buffered {
            let (pre, post) = (st.trigger.pre_trigger_seconds, st.trigger.post_trigger_seconds);
            let ratio = if pre + post > 0.0 { (pre / (pre + post) * 100.0).round() as u32 } else { 0 };
            device.push(format!("captureratio={ratio}"));
        }
        let mut groups = Vec::new();
        for (scope, values) in &st.device_options {
            let kv = values.iter().map(|(k, v): (&String, &ConfValue)| format!("{k}={}", v.cli()));
            match scope {
                crate::backend::OptionScope::Device => device.extend(kv),
                crate::backend::OptionScope::Group(g) => groups.push((g.clone(), kv.collect::<Vec<_>>().join(":"))),
            }
        }
        words.push("-c".into());
        words.push(quote(&device.join(":")));
        for (g, kv) in groups {
            words.extend(["-g".into(), quote(&g), "-c".into(), quote(&kv)]);
        }
        let ms = |s: f64| ((s * 1000.0).round() as u64).to_string();
        let mut comment = None;
        match st.mode {
            CaptureMode::Continuous => words.push("--continuous".into()),
            CaptureMode::Timed => words.extend(["--time".into(), ms(st.timer_seconds)]),
            CaptureMode::Triggered => {
                let t = &st.trigger;
                let mut conds = Vec::new();
                let name = cli_name;
                if let Some(ch) = t.channel.and_then(name) {
                    match t.event {
                        TriggerEvent::Rising => conds.push(format!("{ch}=r")),
                        TriggerEvent::Falling => conds.push(format!("{ch}=f")),
                        _ => comment = Some("pulse trigger: evaluated in the app".to_owned()),
                    }
                }
                conds.extend(t.linked.iter().filter_map(|&(c, high)| Some(format!("{}={}", name(c)?, high as u8))));
                if !conds.is_empty() {
                    words.extend(["-t".into(), quote(&conds.join(","))]);
                }
                words.extend(["--time".into(), ms(t.post_trigger_seconds)]);
            }
        }
        words.extend(["-o".into(), quote(&format!("{}.sr", self.name))]);
        CliCommand { words, comment }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_rates() {
        assert_eq!(quote("D0,D1"), "D0,D1");
        assert_eq!(quote("Session 0.sr"), "\"Session 0.sr\"");
        assert_eq!(quote("a\"b$"), "\"a\\\"b\\$\"");
        assert_eq!(rate(1_000_000), "1m");
        assert_eq!(rate(100_000), "100k");
        assert_eq!(rate(1_562_500), "1562500");
        assert_eq!(rate(2_000_000_000), "2g");
    }

    #[test]
    fn wraps_at_80_columns() {
        let words: Vec<String> =
            "oxrok -d fx2lafw:conn=3.12 -C D0,D1,D2 -c samplerate=8m -t D0=r --time 1000 -o cap.sr"
                .split(' ')
                .map(str::to_owned)
                .collect();
        let c = CliCommand { words, comment: None };
        let w = c.wrapped(WRAP);
        assert_eq!(w, "oxrok -d fx2lafw:conn=3.12 -C D0,D1,D2 -c samplerate=8m -t D0=r --time 1000 \\\n  -o cap.sr");
        assert!(w.lines().all(|l| l.len() <= WRAP));
        assert_eq!(c.one_line().lines().count(), 1);
    }
}

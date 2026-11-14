//! The `oxrok` command that reproduces a session's capture settings

use crate::app::session::Session;
use crate::backend::{ConfValue, DeviceCaps, ThresholdChoice};
use crate::core::ChannelId;
use crate::core::capture::CaptureMode;

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

/// `t` in the `-t` grammar: `,` between the conditions of a stage; `;`,
/// `|` or `&` between stages; `*N` and `*Nc` counts; `>a`, `<b` and
/// `>a<b` durations in samples, both ends strict; `First-Last=lo..hi`
/// value ranges. `name` is what `-C` calls a channel, `position` its
/// place among the device's channels. The error says what the grammar
/// has no words for.
pub(crate) fn trigger_spec(
    t: &crate::backend::HwTrigger,
    name: impl Fn(ChannelId) -> Option<String>,
    position: impl Fn(ChannelId) -> Option<usize>,
) -> Result<String, String> {
    use crate::backend::{HwCombine, HwDuration, TriggerMatch};
    let name = |id: ChannelId| {
        let n = name(id).ok_or_else(|| format!("no channel {}", id.short_name()))?;
        // These would read as part of the grammar.
        if n.is_empty() || n.contains(['-', ',', '=', ';', '|', '&', '*', '<', '>']) {
            // Escaped: the error ends up in the command's `#` comment,
            // where a line break in the name would end the comment.
            return Err(format!("the channel name {n:?}"));
        }
        Ok(n)
    };
    let mut stages = Vec::new();
    for s in &t.stages {
        let modifier = match (s.count, s.duration) {
            (Some((n, consecutive)), _) => format!("*{n}{}", if consecutive { "c" } else { "" }),
            (None, Some(HwDuration::Longer(n))) => format!(">{n}"),
            (None, Some(HwDuration::Shorter(n))) => format!("<{n}"),
            (None, Some(HwDuration::Between(min, max))) => {
                format!(">{}<{}", min.saturating_sub(1), max.saturating_add(1))
            }
            (None, None) => String::new(),
        };
        // The modifier is the stage's; it reads best on its edge.
        let is_edge = |m: &TriggerMatch| !matches!(m, TriggerMatch::Zero | TriggerMatch::One);
        let carrier = s.matches.iter().position(|(_, m)| is_edge(m)).unwrap_or(0);
        let mut tokens = Vec::new();
        for (k, (c, m)) in s.matches.iter().enumerate() {
            let code = match m {
                TriggerMatch::Zero => '0',
                TriggerMatch::One => '1',
                TriggerMatch::Rising => 'r',
                TriggerMatch::Falling => 'f',
                TriggerMatch::Edge => 'e',
            };
            tokens.push(format!("{}={code}{}", name(*c)?, if k == carrier { modifier.as_str() } else { "" }));
        }
        if tokens.is_empty() && !modifier.is_empty() {
            return Err("a count or duration on a value range alone".into());
        }
        if let Some(r) = &s.range {
            // By the device's order, first to last, the first the least
            // significant.
            let at = r.channels.iter().map(|c| position(*c)).collect::<Option<Vec<_>>>();
            let contiguous = at.is_some_and(|p| p.windows(2).all(|w| w[1] == w[0] + 1));
            let (Some(first), Some(last), true) = (r.channels.first(), r.channels.last(), contiguous) else {
                return Err("a value range on channels that are not neighbours".into());
            };
            let values = if r.low == r.high { r.low.to_string() } else { format!("{}..{}", r.low, r.high) };
            tokens.push(format!("{}-{}={}{values}", name(*first)?, name(*last)?, if r.inside { "" } else { "!" }));
        }
        stages.push(tokens.join(","));
    }
    let sep = match t.combine {
        HwCombine::Sequence => ";",
        HwCombine::Or => "|",
        HwCombine::And => "&",
    };
    Ok(stages.join(sep))
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
        // The device's own trigger, where the app arms it too.
        let lowered = (st.mode == CaptureMode::Triggered)
            .then(|| crate::app::trigger::lower(&st.trigger, st.sample_rate, 0).ok())
            .flatten();
        let device_trigger = crate::app::plan_trigger(&st.trigger, caps, 0, st.sample_rate)
            .ok()
            .filter(|e| lowered.is_some() && e.device().is_some());
        // A device that waits for the trigger records one stretch around
        // it: the app's sample limit, and the share of it before the
        // trigger. `--time` would be a limit on the whole capture.
        let waits = st.mode == CaptureMode::Triggered && crate::app::waits_in_device(&st.trigger, caps, st.sample_rate);
        let (limit, ratio) = crate::app::buffered_trigger_plan(
            st.trigger.pre_trigger_seconds,
            st.trigger.post_trigger_seconds,
            st.sample_rate,
        );
        if waits {
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
                if let Some(mut hw) = lowered {
                    // Widths the device's trigger does not take are
                    // the app's to find.
                    if device_trigger.is_none() {
                        for s in &mut hw.stages {
                            if s.duration.take().is_some() {
                                comment = Some("pulse trigger: evaluated in the app".to_owned());
                            }
                        }
                    }
                    // The trigger is armed after the time to keep before
                    // it; the command's fires on its first match.
                    if device_trigger.as_ref().and_then(|e| e.device()).is_some_and(|hw| hw.holdoff > 0) {
                        comment.get_or_insert_with(|| "the app arms the trigger after the pre-trigger time".to_owned());
                    }
                    let position = |id| d.channels.iter().position(|c| c.id == id);
                    match trigger_spec(&hw, cli_name, position) {
                        Ok(spec) => words.extend(["-t".into(), quote(&spec)]),
                        Err(why) => comment = Some(format!("trigger not written: {why}")),
                    }
                }
                if waits {
                    words.extend(["--samples".into(), limit.to_string()]);
                } else {
                    words.extend(["--time".into(), ms(t.post_trigger_seconds)]);
                }
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
    /// The trigger in the `-t` grammar, and oxrok's own parser reading
    /// each line back as the trigger that was written.
    #[test]
    fn trigger_grammar() {
        use crate::app::trigger::lower;
        use crate::core::trigger::{Combine, DigitalTrigger, Hold, Match, Stage, StageCount, ValueRange};
        const D: fn(u16) -> ChannelId = ChannelId::digital;
        let mut registry = ::oxrok::core::Registry::new();
        ::oxrok::register_all(&mut registry);
        let demo = registry.find("demo").unwrap().scan(&[]).unwrap().remove(0);
        let name = |c: ChannelId| Some(c.short_name());
        let position = |c: ChannelId| Some(c.index as usize);
        let rate = 1_000_000;

        let edge = {
            let mut t = DigitalTrigger::on(D(0), Match::Rising);
            t.set(0, D(1), Some(Match::High));
            t.set(0, D(2), Some(Match::Low));
            t
        };
        let pulse = {
            let mut t = DigitalTrigger::pulse(D(3), true, 8e-6, 16e-6);
            t.set(0, D(5), Some(Match::High));
            t
        };
        let counted = {
            let mut t = DigitalTrigger::on(D(0), Match::Rising);
            t.set(1, D(0), Some(Match::High));
            t.set(1, D(1), Some(Match::Falling));
            t.stages[1].count = Some(StageCount { n: 3, consecutive: true });
            t
        };
        let held = {
            let mut t = DigitalTrigger::on(D(0), Match::High);
            t.set(0, D(1), Some(Match::Low));
            t.stages[0].hold = Some(Hold::Longer(50e-6));
            t
        };
        let range = |inside, low, high| ValueRange { channels: vec![D(4), D(5), D(6), D(7)], low, high, inside };
        let ranged = {
            let mut t = DigitalTrigger::on(D(0), Match::Edge);
            t.stages[0].range = Some(range(true, 3, 9));
            t.stages[1] = Stage { range: Some(range(false, 5, 5)), ..Stage::default() };
            t
        };
        let either = |combine| {
            let mut t = DigitalTrigger::on(D(0), Match::Edge);
            t.set(1, D(1), Some(Match::Rising));
            DigitalTrigger { combine, ..t }
        };
        let long = {
            let mut t = DigitalTrigger::on(D(2), Match::Falling);
            t.stages[0].hold = Some(Hold::Longer(10e-6));
            t.set(1, D(2), Some(Match::Rising));
            t.stages[1].hold = Some(Hold::Shorter(4e-6));
            t
        };
        for (t, want) in [
            (edge, "D0=r,D1=1,D2=0"),
            (pulse, "D3=f>7<17,D5=1"),
            (counted, "D0=r;D0=1,D1=f*3c"),
            (held, "D0=1>50,D1=0"),
            (ranged, "D0=e,D4-D7=3..9;D4-D7=!5"),
            (either(Combine::Or), "D0=e|D1=r"),
            (either(Combine::And), "D0=e&D1=r"),
            (long, "D2=f>10;D2=r<4"),
        ] {
            let hw = lower(&t, rate, 0).unwrap();
            assert_eq!(trigger_spec(&hw, name, position).as_deref(), Ok(want), "{t:?}");
            let read = ::oxrok::core::trigger_syntax::parse(demo.as_ref(), want).unwrap();
            assert_eq!(read, crate::oxrok::trigger::to_oxrok(&hw, position).unwrap(), "{want}");
        }

        // What the grammar has no words for.
        let spec = |t: &DigitalTrigger| trigger_spec(&lower(t, rate, 0).unwrap(), name, position);
        let mut alone = DigitalTrigger::default();
        alone.stages[0] = Stage { range: Some(range(true, 1, 2)), hold: Some(Hold::Longer(5e-6)), ..Stage::default() };
        assert!(spec(&alone).unwrap_err().contains("value range alone"));
        let mut apart = DigitalTrigger::default();
        apart.stages[0].range = Some(ValueRange { channels: vec![D(1), D(3)], low: 0, high: 3, inside: true });
        assert!(spec(&apart).unwrap_err().contains("not neighbours"));
        let hw = lower(&DigitalTrigger::on(D(0), Match::Rising), rate, 0).unwrap();
        let renamed = |_| Some("CLK-A".to_owned());
        assert!(trigger_spec(&hw, renamed, position).unwrap_err().contains("CLK-A"));
        // A line break in a name stays inside the comment that carries
        // the error.
        let broken = |_| Some("A\n-B".to_owned());
        let why = trigger_spec(&hw, broken, position).unwrap_err();
        assert_eq!(why, r#"the channel name "A\n-B""#);
        let c = CliCommand { words: vec!["oxrok".into()], comment: Some(format!("trigger not written: {why}")) };
        assert_eq!(c.one_line().lines().count(), 1);
    }
}

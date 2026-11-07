//! Protocol analyzers in a session (12): the model, the link from a
//! capture to the decoder thread, lanes and validation.

use crate::app::capture::SharedCapture;
use crate::core::ChannelId;
use crate::decode::runner::{LogicSource, Output, TaskSpec};
use crate::decode::{DecoderInfo, OptionValue, Radix, all_rows};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The Analyzer Trigger card's settings, saved with a capture.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AnalyzerTrigger {
    pub enabled: bool,
    pub analyzer: Option<u32>,
    pub query: String,
}

/// The scan over the recording capture's frames.
#[derive(Clone, Debug, Default)]
pub struct TriggerScan {
    /// The capture being recorded is watched (until its decoding ends).
    pub armed: bool,
    /// The analyzer's generation when armed: only frames of a later one
    /// (this capture's) count.
    pub armed_gen: u64,
    pub generation: u64,
    /// Frames already looked at, and the last trigger event's sample.
    pub scanned: usize,
    pub last: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Analyzer {
    pub id: u32,
    pub decoder: String,
    pub name: String,
    pub color: [u8; 3],
    /// Role → channel, in the decoder's channel order (required first).
    pub channels: Vec<(String, ChannelId)>,
    pub options: BTreeMap<String, OptionValue>,
    pub radix: Radix,
    pub show_in_table: bool,
    pub to_terminal: bool,
    /// Annotation row ids shown as lanes.
    pub rows_visible: Vec<String>,
    /// Decoder results, shared with the decoder thread.
    pub out: Arc<Output>,
}

impl Analyzer {
    /// What the decoder thread runs.: decoders with a `format`
    /// option (e.g. UART) render values per the radix themselves.
    pub fn task(&self, info: &DecoderInfo) -> TaskSpec {
        let mut options = self.options.clone();
        if info.options.iter().any(|o| o.id == "format")
            && let Some(f) = self.radix.sigrok_format()
        {
            options.insert("format".into(), OptionValue::Str(f.into()));
        }
        TaskSpec { decoder: self.decoder.clone(), options, channels: self.channels.clone(), radix: self.radix }
    }

    /// The channel a row's lane sits on. Rows named after a role
    /// (`rx-data`, `mosi-data`) go on that role's channel, the rest on the
    /// primary channel (the first required, else the first assigned).
    pub fn lane_channel(&self, info: &DecoderInfo, row_id: &str) -> Option<ChannelId> {
        // A row of a role with no channel (UART TX when only RX is set) has no lane.
        let named = |role: &str| row_id.starts_with(&format!("{role}-"));
        let roles = info.channels.iter().chain(&info.opt_channels);
        if roles.clone().any(|c| named(&c.id)) && !self.channels.iter().any(|(r, _)| named(r)) {
            return None;
        }
        let by_role = self.channels.iter().find(|(role, _)| named(role));
        let primary = info
            .channels
            .first()
            .and_then(|c| self.channels.iter().find(|(r, _)| *r == c.id))
            .or_else(|| self.channels.first());
        by_role.or(primary).map(|(_, ch)| *ch)
    }

    /// The visible lanes as `(row index, channel)`, in row order.
    pub fn lanes(&self, info: &DecoderInfo) -> Vec<(usize, ChannelId)> {
        all_rows(info)
            .iter()
            .enumerate()
            .filter(|(_, (id, _))| self.rows_visible.contains(id))
            .filter_map(|(k, (id, _))| self.lane_channel(info, id).map(|c| (k, c)))
            .collect()
    }
}

/// The first problem with a dialog's channel choices.
pub fn validate_channels(info: &DecoderInfo, chosen: &[(String, Option<ChannelId>)]) -> Result<(), &'static str> {
    let get = |role: &str| chosen.iter().find(|(r, _)| r == role).and_then(|c| c.1);
    let required_ok = info.channels.iter().all(|c| get(&c.id).is_some());
    // A decoder with only optional channels needs at least one (UART: RX or TX).
    let any = chosen.iter().any(|c| c.1.is_some());
    if !required_ok || !any {
        return Err("Invalid channel(s)");
    }
    let mut used: Vec<ChannelId> = chosen.iter().filter_map(|c| c.1).collect();
    used.sort();
    if used.windows(2).any(|w| w[0] == w[1]) {
        return Err("Each channel can only be used once");
    }
    Ok(())
}

/// A capture as the decoder thread sees it.
pub struct CaptureSource {
    pub capture: SharedCapture,
    /// Set when the capture has ended.
    pub done: Arc<AtomicBool>,
}

impl LogicSource for CaptureSource {
    fn samplerate(&self) -> Option<u64> {
        self.capture.read().sample_rate()
    }

    fn range(&self) -> Option<(u64, u64)> {
        let d = self.capture.read();
        // Digital channels only: analyzers never read analog data.
        let ranges: Vec<(u64, u64)> =
            d.ingest.iter().flat_map(|i| i.channels().iter().filter_map(|c| c.data_range())).collect();
        let begin = ranges.iter().map(|r| r.0).max()?;
        let end = ranges.iter().map(|r| r.1).min()?;
        (end > begin).then_some((begin, end))
    }

    fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    fn pack(&self, channels: &[ChannelId], s0: u64, s1: u64) -> Option<(Vec<u8>, usize)> {
        let d = self.capture.read();
        let chans: Option<Vec<_>> = channels.iter().map(|c| d.channel(*c)).collect();
        crate::decode::pack::pack(&chans?, s0, s1)
    }
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use super::*;
    use crate::decode::DecoderEngine;
    use crate::decode::mock::MockEngine;

    #[test]
    fn lanes_and_validation() {
        let e = MockEngine::new();
        let uart = e.decoders().iter().find(|d| d.id == "uart").unwrap();
        let a = Analyzer {
            id: 0,
            decoder: "uart".into(),
            name: "Async Serial".into(),
            color: [0; 3],
            channels: vec![("rx".into(), ChannelId::digital(1)), ("tx".into(), ChannelId::digital(2))],
            options: BTreeMap::new(),
            radix: Radix::Ascii,
            show_in_table: true,
            to_terminal: true,
            rows_visible: crate::decode::default_visible_rows(uart),
            out: Default::default(),
        };
        let lanes: Vec<ChannelId> = a.lanes(uart).into_iter().map(|l| l.1).collect();
        assert_eq!(lanes, [ChannelId::digital(1), ChannelId::digital(2)], "rx-data on RX, tx-data on TX");
        assert_eq!(a.task(uart).options.get("format"), Some(&OptionValue::Str("ascii".into())));
        // RX only: no TX lane.
        let rx_only = Analyzer { channels: vec![("rx".into(), ChannelId::digital(1))], ..a.clone() };
        assert_eq!(rx_only.lanes(uart).len(), 1);
        let i2c = e.decoders().iter().find(|d| d.id == "i2c").unwrap();
        let none = [("scl".to_string(), None), ("sda".to_string(), Some(ChannelId::digital(1)))];
        assert_eq!(validate_channels(i2c, &none), Err("Invalid channel(s)"));
        let dup = [("scl".to_string(), Some(ChannelId::digital(1))), ("sda".to_string(), Some(ChannelId::digital(1)))];
        assert_eq!(validate_channels(i2c, &dup), Err("Each channel can only be used once"));
        assert_eq!(
            validate_channels(uart, &[("rx".to_string(), None), ("tx".to_string(), None)]),
            Err("Invalid channel(s)")
        );
        assert_eq!(validate_channels(uart, &[("rx".to_string(), Some(ChannelId::digital(1)))]), Ok(()));
    }
}

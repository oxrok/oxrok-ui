//! The decoder thread: every analyzer of every session, round-robin
//! in chunks, on one thread (the Python GIL serializes decoding anyway).
//! Each session is a *group* with its own capture.

use crate::core::ChannelId;
use crate::decode::frames::{Frame, FrameStore};
use crate::decode::{
    DecodeSession, DecodeSpec, DecoderEngine, DecoderInfo, OptionValue, Radix, RadixSupport, class_rows,
    is_error_class, radix_support,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Mutex, RwLock};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Samples per chunk: small enough to round-robin, large enough to keep
/// the per-call overhead low.
pub const CHUNK: u64 = 1 << 20;

/// Where an analyzer's samples come from: the session's capture.
pub trait LogicSource: Send + Sync {
    fn samplerate(&self) -> Option<u64>;
    /// The committed sample interval `[begin, end)`.
    fn range(&self) -> Option<(u64, u64)>;
    /// No more data will come.
    fn finished(&self) -> bool;
    /// Packed units of `channels` over `[s0, s1)`.
    fn pack(&self, channels: &[ChannelId], s0: u64, s1: u64) -> Option<(Vec<u8>, usize)>;
}

/// Status.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Running,
    Complete,
    Error(String),
}

/// What the UI reads of one analyzer.
#[derive(Debug, Default)]
pub struct Output {
    pub frames: RwLock<FrameStore>,
    pub status: Mutex<Status>,
    /// 0–1.
    pub progress: Mutex<f32>,
    /// Counts restarts: a new value means `frames` were cleared.
    pub generation: std::sync::atomic::AtomicU64,
}

/// One analyzer to run.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskSpec {
    pub decoder: String,
    pub options: BTreeMap<String, OptionValue>,
    /// Role → channel; bit `k` of the packed unit is the k-th entry.
    pub channels: Vec<(String, ChannelId)>,
    /// Applied to raw values when the decoder has no `format`
    /// option.
    pub radix: Radix,
}

/// The session a task or source belongs to.
pub type Group = u64;

enum Cmd {
    Set { group: Group, id: u32, spec: TaskSpec, out: Arc<Output> },
    Remove(Group, u32),
    Source(Group, Option<Arc<dyn LogicSource>>),
    RemoveGroup(Group),
    Pause(Group, bool),
    Shutdown,
}

/// The decoder list, once the engine has loaded (or failed to).
pub type Decoders = Arc<RwLock<Option<Result<Arc<Vec<DecoderInfo>>, String>>>>;

pub struct DecodeService {
    tx: Sender<Cmd>,
    decoders: Decoders,
    handle: Option<JoinHandle<()>>,
}

impl DecodeService {
    /// Starts the decoder thread; `engine` runs there.
    pub fn spawn(engine: impl FnOnce() -> Result<Box<dyn DecoderEngine>, String> + Send + 'static) -> Self {
        let (tx, rx) = unbounded();
        let decoders: Decoders = Arc::new(RwLock::new(None));
        let d = decoders.clone();
        let handle = std::thread::Builder::new()
            .name("decoders".into())
            .spawn(move || match engine() {
                Ok(engine) => {
                    *d.write() = Some(Ok(Arc::new(engine.decoders().to_vec())));
                    Runner { engine, tasks: Vec::new(), sources: HashMap::new(), paused: HashSet::new(), rr: 0 }
                        .run(rx);
                }
                Err(e) => *d.write() = Some(Err(e)),
            })
            .expect("decoder thread");
        Self { tx, decoders, handle: Some(handle) }
    }

    /// `None` until the engine has loaded.
    pub fn decoders(&self) -> Option<Result<Arc<Vec<DecoderInfo>>, String>> {
        self.decoders.read().clone()
    }

    /// Adds analyzer `id` of `group`, or replaces it and restarts its decoding.
    pub fn set(&self, group: Group, id: u32, spec: TaskSpec, out: Arc<Output>) {
        let _ = self.tx.send(Cmd::Set { group, id, spec, out });
    }

    pub fn remove(&self, group: Group, id: u32) {
        let _ = self.tx.send(Cmd::Remove(group, id));
    }

    /// The capture `group` decodes; a new one restarts its analyzers.
    pub fn set_source(&self, group: Group, source: Option<Arc<dyn LogicSource>>) {
        let _ = self.tx.send(Cmd::Source(group, source));
    }

    /// `group`'s analyzers stop where they are, or continue.
    pub fn pause(&self, group: Group, paused: bool) {
        let _ = self.tx.send(Cmd::Pause(group, paused));
    }

    /// The session closed: its analyzers and capture go.
    pub fn remove_group(&self, group: Group) {
        let _ = self.tx.send(Cmd::RemoveGroup(group));
    }
}

impl Drop for DecodeService {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Task {
    group: Group,
    id: u32,
    spec: TaskSpec,
    out: Arc<Output>,
    session: Option<Box<dyn DecodeSession>>,
    next: u64,
    done: bool,
}

impl Task {
    fn reset(&mut self) {
        self.session = None;
        self.done = false;
        *self.out.frames.write() = FrameStore::default();
        *self.out.status.lock() = Status::Idle;
        *self.out.progress.lock() = 0.0;
        self.out.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct Runner {
    engine: Box<dyn DecoderEngine>,
    tasks: Vec<Task>,
    sources: HashMap<Group, Arc<dyn LogicSource>>,
    paused: HashSet<Group>,
    rr: usize,
}

impl Runner {
    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            // Block only when there is nothing to decode.
            let cmd =
                if self.has_work() { rx.try_recv().ok() } else { rx.recv_timeout(Duration::from_millis(20)).ok() };
            if let Some(cmd) = cmd {
                match cmd {
                    Cmd::Shutdown => return,
                    Cmd::Set { group, id, spec, out } => {
                        self.tasks.retain(|t| (t.group, t.id) != (group, id));
                        let mut t = Task { group, id, spec, out, session: None, next: 0, done: false };
                        t.reset();
                        self.tasks.push(t);
                    }
                    Cmd::Remove(group, id) => self.tasks.retain(|t| (t.group, t.id) != (group, id)),
                    Cmd::Source(group, s) => {
                        match s {
                            Some(s) => self.sources.insert(group, s),
                            None => self.sources.remove(&group),
                        };
                        self.tasks.iter_mut().filter(|t| t.group == group).for_each(Task::reset);
                    }
                    Cmd::Pause(group, true) => {
                        self.paused.insert(group);
                    }
                    Cmd::Pause(group, false) => {
                        self.paused.remove(&group);
                    }
                    Cmd::RemoveGroup(group) => {
                        self.paused.remove(&group);
                        self.sources.remove(&group);
                        self.tasks.retain(|t| t.group != group);
                    }
                }
                continue;
            }
            self.step();
        }
    }

    /// Whether task `t` can make progress now.
    fn runnable(&self, t: &Task) -> bool {
        if self.paused.contains(&t.group) {
            return false;
        }
        let Some(src) = self.sources.get(&t.group) else { return false };
        let Some((_, end)) = src.range() else { return false };
        src.samplerate().is_some() && !t.done && (t.session.is_none() || t.next < end || src.finished())
    }

    fn has_work(&self) -> bool {
        self.tasks.iter().any(|t| self.runnable(t))
    }

    /// One chunk of one analyzer (round-robin over every session's).
    fn step(&mut self) {
        let n = self.tasks.len();
        for k in 0..n {
            let i = (self.rr + k) % n;
            if !self.runnable(&self.tasks[i]) {
                continue;
            }
            self.rr = i + 1;
            let src = self.sources[&self.tasks[i].group].clone();
            let (Some(rate), Some((begin, end))) = (src.samplerate(), src.range()) else { return };
            self.advance(i, &*src, rate, begin, end);
            return;
        }
    }

    fn advance(&mut self, i: usize, src: &dyn LogicSource, rate: u64, begin: u64, end: u64) {
        let info = self.engine.decoders().iter().find(|d| d.id == self.tasks[i].spec.decoder).cloned();
        let t = &mut self.tasks[i];
        let Some(info) = info else {
            fail(t, format!("Unknown decoder {}", t.spec.decoder));
            return;
        };
        if t.session.is_none() {
            let spec = DecodeSpec {
                decoder: t.spec.decoder.clone(),
                samplerate: rate,
                options: t.spec.options.clone(),
                channels: t.spec.channels.iter().enumerate().map(|(k, (role, _))| (role.clone(), k)).collect(),
            };
            match self.engine.start(&spec) {
                Ok(s) => {
                    t.session = Some(s);
                    t.next = begin;
                    *t.out.status.lock() = Status::Running;
                }
                Err(e) => {
                    fail(t, e);
                    return;
                }
            }
        }
        // Data trimmed away under us: continue from the new start.
        t.next = t.next.max(begin);
        let to = (t.next + CHUNK).min(end);
        let chans: Vec<ChannelId> = t.spec.channels.iter().map(|c| c.1).collect();
        let anns = if to > t.next {
            let Some((buf, us)) = src.pack(&chans, t.next, to) else {
                fail(t, "A channel of this analyzer has no data.".into());
                return;
            };
            let r = t.session.as_mut().unwrap().send(t.next, &buf, us);
            t.next = to;
            r
        } else if src.finished() {
            // After the capture ends, feed to the end, then finish.
            let r = t.session.as_mut().unwrap().finish();
            t.done = true;
            r
        } else {
            return;
        };
        match anns {
            Ok(anns) => {
                let rows = class_rows(&info);
                let ours = radix_support(&info) == RadixSupport::RawValues && t.spec.radix != Radix::Native;
                let mut store = t.out.frames.write();
                for a in anns {
                    let row = rows.get(a.class).copied().unwrap_or(0);
                    let error = is_error_class(&info, a.class);
                    let texts = match a.value {
                        Some((v, bits)) if ours => crate::decode::format::apply_radix(&a.texts, v, bits, t.spec.radix),
                        _ => a.texts,
                    };
                    store.push(Frame { start: a.start, end: a.end, class: a.class, row, texts: texts.into(), error });
                }
            }
            Err(e) => {
                fail(t, e);
                return;
            }
        }
        let span = (end - begin).max(1) as f32;
        *t.out.progress.lock() = if t.done { 1.0 } else { (t.next - begin) as f32 / span };
        if t.done {
            *t.out.status.lock() = Status::Complete;
        }
    }
}

/// Frames so far are kept.
fn fail(t: &mut Task, e: String) {
    t.session = None;
    t.done = true;
    *t.out.status.lock() = Status::Error(e);
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use super::*;
    use crate::decode::mock::MockEngine;
    use crate::store::DigitalChannel;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Instant;

    /// A UART line in a DigitalChannel, revealed gradually like a capture.
    struct Src {
        ch: DigitalChannel,
        end: AtomicU64,
        finished: AtomicBool,
        total: u64,
    }

    impl LogicSource for Src {
        fn samplerate(&self) -> Option<u64> {
            Some(1_152_000)
        }
        fn range(&self) -> Option<(u64, u64)> {
            Some((0, self.end.load(Ordering::SeqCst)))
        }
        fn finished(&self) -> bool {
            self.finished.load(Ordering::SeqCst)
        }
        fn pack(&self, channels: &[ChannelId], s0: u64, s1: u64) -> Option<(Vec<u8>, usize)> {
            assert_eq!(channels, [ChannelId::digital(1)]);
            crate::decode::pack::pack(&[&self.ch], s0, s1)
        }
    }

    fn uart_channel(text: &[u8], spb: u64) -> (DigitalChannel, u64) {
        let mut ch = DigitalChannel::new();
        ch.start(0, true);
        let mut level = true;
        let mut s = spb * 3;
        for &b in text {
            let bits = std::iter::once(false).chain((0..8).map(|i| (b >> i) & 1 == 1)).chain(std::iter::once(true));
            for bit in bits {
                if bit != level {
                    ch.push_transition(s);
                    level = bit;
                }
                s += spb;
            }
        }
        s += spb * 3;
        ch.extend_to(s);
        (ch, s)
    }

    fn wait(cond: impl Fn() -> bool) {
        let t0 = Instant::now();
        while !cond() {
            assert!(t0.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn decodes_live_then_completes_and_restarts() {
        let svc = DecodeService::spawn(|| Ok(Box::new(MockEngine::new()) as Box<dyn DecoderEngine>));
        wait(|| svc.decoders().is_some());
        assert_eq!(svc.decoders().unwrap().unwrap().len(), 3);
        let (ch, total) = uart_channel(b"Hello", 10);
        let src = Arc::new(Src { ch, end: AtomicU64::new(total / 2), finished: AtomicBool::new(false), total });
        let out = Arc::new(Output::default());
        let spec = TaskSpec {
            radix: crate::decode::Radix::Native,
            decoder: "uart".into(),
            options: BTreeMap::from([("format".into(), OptionValue::Str("ascii".into()))]),
            channels: vec![("rx".into(), ChannelId::digital(1))],
        };
        svc.set_source(7, Some(src.clone()));
        svc.set(7, 0, spec.clone(), out.clone());
        let data = |out: &Output| {
            let f = out.frames.read();
            f.order()
                .iter()
                .filter_map(|&i| f.get(i))
                .filter(|f| f.class == 0)
                .map(|f| f.longest().to_owned())
                .collect::<Vec<_>>()
        };
        // Half the data: the first bytes decode, still running.
        wait(|| data(&out).len() >= 2);
        assert_eq!(*out.status.lock(), Status::Running);
        // The rest arrives and the capture ends.
        src.end.store(src.total, Ordering::SeqCst);
        src.finished.store(true, Ordering::SeqCst);
        wait(|| *out.status.lock() == Status::Complete);
        assert_eq!(data(&out), ["H", "e", "l", "l", "o"]);
        assert_eq!(*out.progress.lock(), 1.0);
        // Replacing the analyzer restarts it from scratch.
        let hex = TaskSpec { options: BTreeMap::new(), ..spec };
        svc.set(7, 0, hex, out.clone());
        wait(|| *out.status.lock() == Status::Complete && data(&out).first().is_some_and(|d| d == "48"));
        assert_eq!(data(&out), ["48", "65", "6C", "6C", "6F"]);
    }

    #[test]
    fn unknown_decoder_is_an_error() {
        let svc = DecodeService::spawn(|| Ok(Box::new(MockEngine::new()) as Box<dyn DecoderEngine>));
        let (ch, total) = uart_channel(b"x", 10);
        let src = Arc::new(Src { ch, end: AtomicU64::new(total), finished: AtomicBool::new(true), total });
        svc.set_source(0, Some(src));
        let out = Arc::new(Output::default());
        svc.set(
            0,
            3,
            TaskSpec {
                decoder: "nope".into(),
                options: BTreeMap::new(),
                channels: vec![],
                radix: crate::decode::Radix::Native,
            },
            out.clone(),
        );
        wait(|| matches!(*out.status.lock(), Status::Error(_)));
    }

    /// Sessions decode side by side; a new capture in one
    /// doesn't restart the other's analyzers.
    #[test]
    fn groups_are_independent() {
        let svc = DecodeService::spawn(|| Ok(Box::new(MockEngine::new()) as Box<dyn DecoderEngine>));
        let spec = TaskSpec {
            radix: crate::decode::Radix::Native,
            decoder: "uart".into(),
            options: BTreeMap::from([("format".into(), OptionValue::Str("ascii".into()))]),
            channels: vec![("rx".into(), ChannelId::digital(1))],
        };
        let src = |text: &[u8]| {
            let (ch, total) = uart_channel(text, 10);
            Arc::new(Src { ch, end: AtomicU64::new(total), finished: AtomicBool::new(true), total })
        };
        let (a, b) = (Arc::new(Output::default()), Arc::new(Output::default()));
        svc.set_source(1, Some(src(b"ab")));
        svc.set_source(2, Some(src(b"xyz")));
        svc.set(1, 0, spec.clone(), a.clone());
        svc.set(2, 0, spec.clone(), b.clone());
        let done = |o: &Output| *o.status.lock() == Status::Complete;
        wait(|| done(&a) && done(&b));
        let count = |o: &Output| {
            o.frames.read().order().iter().filter(|&&i| o.frames.read().get(i).unwrap().class == 0).count()
        };
        assert_eq!((count(&a), count(&b)), (2, 3));
        // A new capture in group 2 leaves group 1's results alone.
        svc.set_source(2, Some(src(b"q")));
        wait(|| done(&b) && count(&b) == 1);
        assert!(done(&a));
        assert_eq!(count(&a), 2);
        svc.remove_group(1);
        svc.set_source(1, Some(src(b"zz")));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(count(&a), 2, "removed tasks don't run again");
    }

    /// A paused group decodes nothing until resumed.
    #[test]
    fn pause_and_resume() {
        let svc = DecodeService::spawn(|| Ok(Box::new(MockEngine::new()) as Box<dyn DecoderEngine>));
        let spec = TaskSpec {
            radix: crate::decode::Radix::Native,
            decoder: "uart".into(),
            options: BTreeMap::new(),
            channels: vec![("rx".into(), ChannelId::digital(1))],
        };
        let (ch, total) = uart_channel(b"hi", 10);
        svc.pause(4, true);
        svc.set_source(
            4,
            Some(Arc::new(Src { ch, end: AtomicU64::new(total), finished: AtomicBool::new(true), total })),
        );
        let out = Arc::new(Output::default());
        svc.set(4, 0, spec, out.clone());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(*out.status.lock(), Status::Idle);
        assert!(out.frames.read().order().is_empty());
        svc.pause(4, false);
        wait(|| *out.status.lock() == Status::Complete);
    }

    /// Finished channels, packed in the order asked for.
    struct Multi(Vec<(ChannelId, DigitalChannel)>, u64);

    impl LogicSource for Multi {
        fn samplerate(&self) -> Option<u64> {
            Some(1_000_000)
        }
        fn range(&self) -> Option<(u64, u64)> {
            Some((0, self.1))
        }
        fn finished(&self) -> bool {
            true
        }
        fn pack(&self, channels: &[ChannelId], s0: u64, s1: u64) -> Option<(Vec<u8>, usize)> {
            let chs: Option<Vec<&DigitalChannel>> =
                channels.iter().map(|c| self.0.iter().find(|x| x.0 == *c).map(|x| &x.1)).collect();
            crate::decode::pack::pack(&chs?, s0, s1)
        }
    }

    /// SPI values in the radix, from their raw values;
    /// Native keeps the decoder's text.
    #[test]
    fn raw_values_take_the_radix() {
        // CLK on D0, MOSI on D1: 0x3A, MSB first, mode 0.
        let (mut clk, mut mosi) = (DigitalChannel::new(), DigitalChannel::new());
        clk.start(0, false);
        mosi.start(0, false);
        let (mut c, mut m) = (false, false);
        let set = |ch: &mut DigitalChannel, level: &mut bool, v: bool, s: u64| {
            if *level != v {
                ch.push_transition(s);
                *level = v;
            }
        };
        let mut s = 4;
        for k in (0..8).rev() {
            set(&mut mosi, &mut m, (0x3A >> k) & 1 == 1, s);
            set(&mut clk, &mut c, true, s + 2);
            set(&mut clk, &mut c, false, s + 4);
            s += 4;
        }
        clk.extend_to(s + 4);
        mosi.extend_to(s + 4);
        let src: Arc<dyn LogicSource> =
            Arc::new(Multi(vec![(ChannelId::digital(0), clk), (ChannelId::digital(1), mosi)], s + 4));
        let svc = DecodeService::spawn(|| Ok(Box::new(MockEngine::new()) as Box<dyn DecoderEngine>));
        svc.set_source(1, Some(src));
        for (radix, want) in [(Radix::Native, "3A"), (Radix::Dec, "58"), (Radix::Ascii, "':'"), (Radix::Hex, "0x3A")] {
            let out = Arc::new(Output::default());
            let spec = TaskSpec {
                decoder: "spi".into(),
                options: BTreeMap::new(),
                channels: vec![("clk".into(), ChannelId::digital(0)), ("mosi".into(), ChannelId::digital(1))],
                radix,
            };
            svc.set(1, 0, spec, out.clone());
            wait(|| *out.status.lock() == Status::Complete);
            let f = out.frames.read();
            let data: Vec<&str> =
                f.order().iter().filter_map(|&i| f.get(i)).filter(|f| f.class == 1).map(|f| f.longest()).collect();
            assert_eq!(data, [want], "{radix:?}");
        }
    }
}

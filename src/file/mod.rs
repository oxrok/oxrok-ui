//! The `.oxrok` capture container.
//!
//! An uncompressed tar archive (GNU headers, so entries can pass 8 GiB)
//! holding, in this order:
//! - `metadata.json`: `schema_version`, `app_version` and the session
//!   state, which the app defines and migrates. Always first, so it is
//!   read without scanning the rest;
//! - `channels/D<i>.bin`: a digital channel, zstd compressed: begin u64,
//!   end u64, initial level u8, transition count u64, then the transitions
//!   as little-endian u64 deltas (the first from `begin`);
//! - `channels/A<i>.bin`: an analog channel, zstd compressed: begin u64,
//!   sample count u64, then the samples as little-endian f32.
//!
//! Files are written to a temporary file next to the target and renamed
//! into place, so a failed save never leaves half a file.

use crate::core::{ChannelId, ChannelKind};
use crate::store::{AnalogChannel, DigitalChannel};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The schema this build writes. Older files are migrated by the app;
/// newer ones are refused.
pub const SCHEMA_VERSION: u32 = 1;
pub const EXTENSION: &str = "oxrok";
const META: &str = "metadata.json";
const BLOCK: u64 = 512;
const ZSTD_LEVEL: i32 = 3;
/// Values copied per read or write call while streaming channel data.
const BATCH: usize = 1 << 16;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("not a capture file: {0}")]
    Format(String),
    #[error("This capture was saved by a newer version of the app")]
    Newer(u32),
    #[error("cancelled")]
    Cancelled,
}

impl From<serde_json::Error> for FileError {
    fn from(e: serde_json::Error) -> Self {
        FileError::Format(format!("metadata.json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, FileError>;

/// Progress from 0 to 1; returning `false` cancels.
pub type Progress<'a> = &'a mut dyn FnMut(f32) -> bool;

/// What a load returns: the raw `metadata.json` bytes (migrate before use)
/// and the channel data. Kept as bytes: the app's metadata holds `i128`
/// times, which `serde_json::Value` can't represent.
pub struct Loaded {
    pub meta: Vec<u8>,
    pub digital: Vec<(ChannelId, DigitalChannel)>,
    pub analog: Vec<(ChannelId, AnalogChannel)>,
}

/// The `schema_version` of a `metadata.json`, 0 when missing.
pub fn schema_version(meta: &[u8]) -> Result<u32> {
    #[derive(serde::Deserialize)]
    struct Version {
        #[serde(default)]
        schema_version: u32,
    }
    Ok(serde_json::from_slice::<Version>(meta)?.schema_version)
}

fn entry_name(id: ChannelId) -> String {
    format!("channels/{}.bin", id.short_name())
}

fn parse_entry_name(name: &str) -> Option<ChannelId> {
    let stem = name.strip_prefix("channels/")?.strip_suffix(".bin")?;
    let (kind, index) = match stem.split_at_checked(1)? {
        ("D", i) => (ChannelKind::Digital, i),
        ("A", i) => (ChannelKind::Analog, i),
        _ => return None,
    };
    Some(ChannelId { kind, index: index.parse().ok()? })
}

/// Writes tar entries whose size isn't known up front: a blank header,
/// the data, then the header filled in.
struct TarWriter {
    out: BufWriter<File>,
    mtime: u64,
}

impl TarWriter {
    fn create(path: &Path) -> Result<Self> {
        let mtime = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Ok(Self { out: BufWriter::new(File::create(path)?), mtime })
    }

    fn entry(&mut self, name: &str, body: impl FnOnce(&mut BufWriter<File>) -> Result<()>) -> Result<()> {
        let start = self.out.stream_position()?;
        self.out.write_all(&[0; BLOCK as usize])?;
        body(&mut self.out)?;
        let size = self.out.stream_position()? - start - BLOCK;
        self.out.write_all(&vec![0; ((BLOCK - size % BLOCK) % BLOCK) as usize])?;
        let mut h = tar::Header::new_gnu();
        h.set_path(name)?;
        h.set_size(size);
        h.set_mode(0o644);
        h.set_mtime(self.mtime);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        self.out.seek(SeekFrom::Start(start))?;
        self.out.write_all(h.as_bytes())?;
        self.out.seek(SeekFrom::End(0))?;
        Ok(())
    }

    /// The end-of-archive marker: two zero blocks.
    fn finish(mut self) -> Result<()> {
        self.out.write_all(&[0; 2 * BLOCK as usize])?;
        self.out.flush()?;
        Ok(())
    }
}

/// A malformed archive is a format error, not an I/O one.
fn malformed(e: io::Error) -> FileError {
    FileError::Format(e.to_string())
}

/// Opens a capture's archive and its length.
fn open_archive(path: &Path) -> Result<(tar::Archive<BufReader<File>>, u64)> {
    let f = File::open(path)?;
    let len = f.metadata()?.len();
    Ok((tar::Archive::new(BufReader::new(f)), len))
}

/// The first entry, which must be `metadata.json`.
fn read_meta<'a, R: 'a + Read>(entries: &mut impl Iterator<Item = io::Result<tar::Entry<'a, R>>>) -> Result<Vec<u8>> {
    let missing = || FileError::Format("metadata.json is missing".into());
    let mut e = entries.next().ok_or_else(missing)?.map_err(malformed)?;
    if e.path().map_err(malformed)?.as_os_str() != META {
        return Err(missing());
    }
    let mut meta = Vec::new();
    e.read_to_end(&mut meta).map_err(malformed)?;
    check_version(&meta)?;
    Ok(meta)
}

/// A temporary path next to `path`, renamed onto it on success.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".saving");
    path.with_file_name(name)
}

fn finish_atomically(tmp: &Path, path: &Path, r: Result<()>) -> Result<()> {
    match r {
        Ok(()) => std::fs::rename(tmp, path).map_err(FileError::from),
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(e)
        }
    }
}

/// Writes a capture: `meta` (its `schema_version` is set here) and the
/// channels' stored data.
pub fn save(
    path: &Path,
    meta: &[u8],
    digital: &[(ChannelId, &DigitalChannel)],
    analog: &[(ChannelId, &AnalogChannel)],
    progress: Progress,
) -> Result<()> {
    let tmp = temp_path(path);
    let r = (|| {
        check_version(meta)?;
        let mut tar = TarWriter::create(&tmp)?;
        tar.entry(META, |w| Ok(w.write_all(meta)?))?;

        // Progress weights: transitions for digital, samples for analog.
        let weight = |n: u64| n.max(1) as f64;
        let digital_weight = |c: &DigitalChannel| c.data_range().map_or(0, |(a, b)| c.transitions_in(a, b));
        let total: f64 = digital.iter().map(|(_, c)| weight(digital_weight(c))).sum::<f64>()
            + analog.iter().map(|(_, c)| weight(c.data_range().map_or(0, |(a, b)| b - a))).sum::<f64>();
        let mut done = 0.0;
        for (id, ch) in digital {
            tar.entry(&entry_name(*id), |w| {
                let mut z = zstd::Encoder::new(w, ZSTD_LEVEL)?;
                write_digital(&mut z, ch, &mut |k| progress(((done + k as f64) / total) as f32))?;
                z.finish()?;
                Ok(())
            })?;
            done += weight(digital_weight(ch));
        }
        for (id, ch) in analog {
            tar.entry(&entry_name(*id), |w| {
                let mut z = zstd::Encoder::new(w, ZSTD_LEVEL)?;
                write_analog(&mut z, ch, &mut |k| progress(((done + k as f64) / total) as f32))?;
                z.finish()?;
                Ok(())
            })?;
            done += weight(ch.data_range().map_or(0, |(a, b)| b - a));
        }
        tar.finish()?;
        progress(1.0);
        Ok(())
    })();
    finish_atomically(&tmp, path, r)
}

fn write_digital(w: &mut impl Write, ch: &DigitalChannel, progress: &mut dyn FnMut(u64) -> bool) -> Result<()> {
    let (begin, end) = ch.data_range().unwrap_or((0, 0));
    let level = ch.level_at(begin).unwrap_or(false);
    // Transitions after the first sample (one at `begin` is in `level`).
    let count = if end > begin { ch.transitions_in(begin + 1, end) } else { 0 };
    w.write_all(&begin.to_le_bytes())?;
    w.write_all(&end.to_le_bytes())?;
    w.write_all(&[level as u8])?;
    w.write_all(&count.to_le_bytes())?;
    let mut prev = begin;
    let mut buf = Vec::with_capacity(BATCH * 8);
    let mut written = 0u64;
    for t in ch.transitions_from(begin + 1).take(count as usize) {
        buf.extend_from_slice(&(t - prev).to_le_bytes());
        prev = t;
        if buf.len() >= BATCH * 8 {
            w.write_all(&buf)?;
            written += (buf.len() / 8) as u64;
            buf.clear();
            if !progress(written) {
                return Err(FileError::Cancelled);
            }
        }
    }
    w.write_all(&buf)?;
    Ok(())
}

fn write_analog(w: &mut impl Write, ch: &AnalogChannel, progress: &mut dyn FnMut(u64) -> bool) -> Result<()> {
    let (begin, end) = ch.data_range().unwrap_or((0, 0));
    w.write_all(&begin.to_le_bytes())?;
    w.write_all(&(end - begin).to_le_bytes())?;
    let mut buf = Vec::with_capacity(BATCH * 4);
    let mut written = 0u64;
    for (_, v) in ch.samples(begin, end) {
        buf.extend_from_slice(&v.to_le_bytes());
        if buf.len() >= BATCH * 4 {
            w.write_all(&buf)?;
            written += (buf.len() / 4) as u64;
            buf.clear();
            if !progress(written) {
                return Err(FileError::Cancelled);
            }
        }
    }
    w.write_all(&buf)?;
    Ok(())
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_digital(r: &mut impl Read) -> Result<DigitalChannel> {
    let begin = read_u64(r)?;
    let end = read_u64(r)?;
    let mut level = [0u8];
    r.read_exact(&mut level)?;
    let count = read_u64(r)?;
    let mut ch = DigitalChannel::new();
    if end <= begin {
        return Ok(ch);
    }
    ch.start(begin, level[0] != 0);
    let mut buf = vec![0u8; BATCH * 8];
    let mut left = count;
    let mut prev = begin;
    while left > 0 {
        let n = left.min(BATCH as u64) as usize;
        r.read_exact(&mut buf[..n * 8])?;
        for &d in buf[..n * 8].as_chunks::<8>().0 {
            let delta = u64::from_le_bytes(d);
            prev = prev
                .checked_add(delta)
                .filter(|&t| t < end && delta > 0)
                .ok_or_else(|| FileError::Format("a digital channel's transitions are out of order".into()))?;
            ch.push_transition(prev);
        }
        left -= n as u64;
    }
    ch.extend_to(end);
    Ok(ch)
}

fn read_analog(r: &mut impl Read) -> Result<AnalogChannel> {
    let begin = read_u64(r)?;
    let count = read_u64(r)?;
    let mut ch = AnalogChannel::new();
    let mut buf = vec![0u8; BATCH * 4];
    let mut values = Vec::with_capacity(BATCH);
    let mut at = begin;
    let mut left = count;
    while left > 0 {
        let n = left.min(BATCH as u64) as usize;
        r.read_exact(&mut buf[..n * 4])?;
        values.clear();
        values.extend(buf[..n * 4].as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)));
        ch.push(at, &values).map_err(|_| FileError::Format("analog samples out of order".into()))?;
        at += n as u64;
        left -= n as u64;
    }
    Ok(ch)
}

/// Refuses metadata of a schema newer than this build's.
fn check_version(meta: &[u8]) -> Result<u32> {
    let v = schema_version(meta)?;
    if v > SCHEMA_VERSION {
        return Err(FileError::Newer(v));
    }
    Ok(v)
}

/// Reads only `metadata.json` (e.g. to list a file's contents).
pub fn load_meta(path: &Path) -> Result<Vec<u8>> {
    let (mut archive, _) = open_archive(path)?;
    read_meta(&mut archive.entries_with_seek().map_err(malformed)?)
}

/// Reads a whole capture. Progress follows the bytes read.
pub fn load(path: &Path, progress: Progress) -> Result<Loaded> {
    let (mut archive, len) = open_archive(path)?;
    let mut entries = archive.entries_with_seek().map_err(malformed)?;
    let meta = read_meta(&mut entries)?;
    let (mut digital, mut analog) = (Vec::new(), Vec::new());
    for e in entries {
        let e = e.map_err(malformed)?;
        let Some(id) = e.path().ok().and_then(|p| parse_entry_name(&p.to_string_lossy())) else { continue };
        if !progress(e.raw_file_position() as f32 / len.max(1) as f32) {
            return Err(FileError::Cancelled);
        }
        let mut z = zstd::Decoder::new(e)?;
        match id.kind {
            ChannelKind::Digital => digital.push((id, read_digital(&mut z)?)),
            ChannelKind::Analog => analog.push((id, read_analog(&mut z)?)),
            // Never written: a bus has no samples.
            ChannelKind::Bus => {}
        }
    }
    digital.sort_by_key(|(id, _)| *id);
    analog.sort_by_key(|(id, _)| *id);
    progress(1.0);
    Ok(Loaded { meta, digital, analog })
}

/// Replaces `metadata.json` and keeps the channel data as it is
/// the channel entries are copied without recompressing.
pub fn update_meta(path: &Path, meta: &[u8]) -> Result<()> {
    let tmp = temp_path(path);
    let r = (|| {
        let (mut src, _) = open_archive(path)?;
        let mut entries = src.entries_with_seek().map_err(malformed)?;
        read_meta(&mut entries)?;
        check_version(meta)?;
        let mut tar = TarWriter::create(&tmp)?;
        tar.entry(META, |w| Ok(w.write_all(meta)?))?;
        for e in entries {
            let mut e = e.map_err(malformed)?;
            let name = e.path().map_err(malformed)?.to_string_lossy().into_owned();
            tar.entry(&name, |w| {
                io::copy(&mut e, w)?;
                Ok(())
            })?;
        }
        tar.finish()
    })();
    finish_atomically(&tmp, path, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digital(begin: u64, level: bool, transitions: &[u64], end: u64) -> DigitalChannel {
        let mut c = DigitalChannel::new();
        c.start(begin, level);
        for &t in transitions {
            c.push_transition(t);
        }
        c.extend_to(end);
        c
    }

    fn transitions(c: &DigitalChannel) -> Vec<u64> {
        let (b, _) = c.data_range().unwrap();
        c.transitions_from(b + 1).collect()
    }

    #[test]
    fn round_trip_keeps_channels_and_meta() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.oxrok");
        let d0 = digital(10, true, &[12, 15, 400_000], 1_000_000);
        let d3 = digital(10, false, &[], 1_000_000);
        // Many transitions: more than one batch and one store chunk.
        let many: Vec<u64> = (0..200_000).map(|i| 20 + i * 3).collect();
        let d5 = digital(10, false, &many, 1_000_000);
        let mut a1 = AnalogChannel::new();
        let values: Vec<f32> = (0..100_000).map(|i| (i as f32 * 0.01).sin()).collect();
        a1.push(10, &values).unwrap();
        let meta = br#"{"schema_version": 1, "app_version": "test", "name": "Session 0"}"#;
        let mut seen = Vec::new();
        save(
            &path,
            meta,
            &[(ChannelId::digital(0), &d0), (ChannelId::digital(3), &d3), (ChannelId::digital(5), &d5)],
            &[(ChannelId::analog(1), &a1)],
            &mut |p| {
                seen.push(p);
                true
            },
        )
        .unwrap();
        assert_eq!(seen.last(), Some(&1.0));
        let l = load(&path, &mut |_| true).unwrap();
        assert_eq!(schema_version(&l.meta).unwrap(), SCHEMA_VERSION);
        let m: serde_json::Value = serde_json::from_slice(&l.meta).unwrap();
        assert_eq!(m["name"], "Session 0");
        let ids: Vec<ChannelId> = l.digital.iter().map(|c| c.0).collect();
        assert_eq!(ids, [ChannelId::digital(0), ChannelId::digital(3), ChannelId::digital(5)]);
        let back = &l.digital[0].1;
        assert_eq!(back.data_range(), Some((10, 1_000_000)));
        assert_eq!(back.level_at(10), Some(true));
        assert_eq!(transitions(back), [12, 15, 400_000]);
        assert_eq!(l.digital[1].1.transition_count(), 0);
        assert_eq!(l.digital[1].1.level_at(999_999), Some(false));
        assert_eq!(transitions(&l.digital[2].1), many);
        let a = &l.analog[0].1;
        assert_eq!(a.data_range(), Some((10, 100_010)));
        let got: Vec<f32> = a.samples(10, 100_010).map(|(_, v)| v).collect();
        assert_eq!(got, values);
        // No temporary file left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_transition_at_the_first_sample_is_part_of_the_level() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.oxrok");
        // Trimmed data can start exactly on a transition.
        let mut c = digital(0, false, &[5, 9, 20], 30);
        c.trim_before(9);
        let (b, _) = c.data_range().unwrap();
        save(&path, b"{}", &[(ChannelId::digital(0), &c)], &[], &mut |_| true).unwrap();
        let l = load(&path, &mut |_| true).unwrap();
        let back = &l.digital[0].1;
        assert_eq!(back.data_range(), c.data_range());
        for s in b..30 {
            assert_eq!(back.level_at(s), c.level_at(s), "sample {s}");
        }
    }

    #[test]
    fn newer_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.oxrok");
        save(&path, b"{}", &[], &[], &mut |_| true).unwrap();
        update_meta(&path, br#"{"x": 1}"#).unwrap();
        // Write a future schema by hand.
        let mut tar = TarWriter::create(&path).unwrap();
        tar.entry(META, |w| Ok(w.write_all(br#"{"schema_version": 99}"#)?)).unwrap();
        tar.finish().unwrap();
        assert!(matches!(load(&path, &mut |_| true), Err(FileError::Newer(99))));
        assert!(matches!(update_meta(&path, br#"{}"#), Err(FileError::Newer(99))));
        // Writing a newer schema than this build knows is refused too.
        assert!(matches!(save(&path, br#"{"schema_version": 2}"#, &[], &[], &mut |_| true), Err(FileError::Newer(2))));
        assert_eq!(FileError::Newer(99).to_string(), "This capture was saved by a newer version of the app");
    }

    #[test]
    fn update_meta_keeps_the_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.oxrok");
        let d0 = digital(0, false, &[3, 4, 100], 200);
        save(&path, br#"{"notes": "a"}"#, &[(ChannelId::digital(0), &d0)], &[], &mut |_| true).unwrap();
        update_meta(&path, br#"{"notes": "b", "big": 170141183460469231731687303715884105727}"#).unwrap();
        let l = load(&path, &mut |_| true).unwrap();
        assert!(String::from_utf8(l.meta).unwrap().contains(r#""notes": "b""#));
        assert_eq!(transitions(&l.digital[0].1), [3, 4, 100]);
    }

    #[test]
    fn cancelling_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.oxrok");
        let many: Vec<u64> = (1..300_000).collect();
        let d0 = digital(0, false, &many, 400_000);
        let r = save(&path, b"{}", &[(ChannelId::digital(0), &d0)], &[], &mut |p| p < 0.1);
        assert!(matches!(r, Err(FileError::Cancelled)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn garbage_is_a_format_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.oxrok");
        std::fs::write(&path, b"not a tar").unwrap();
        assert!(matches!(load(&path, &mut |_| true), Err(FileError::Format(_))));
        // Any other archive, e.g. a ZIP.
        std::fs::write(&path, b"PK\x03\x04 and the rest").unwrap();
        assert!(matches!(load(&path, &mut |_| true), Err(FileError::Format(_))));
        std::fs::write(&path, b"").unwrap();
        assert!(matches!(load(&path, &mut |_| true), Err(FileError::Format(_))));
    }

    /// Standard tools read it; `metadata.json` comes first.
    #[test]
    fn a_plain_tar_with_metadata_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.oxrok");
        let d0 = digital(0, false, &[3, 4, 100], 200);
        save(&path, br#"{"notes": "a"}"#, &[(ChannelId::digital(0), &d0)], &[], &mut |_| true).unwrap();
        let mut a = tar::Archive::new(File::open(&path).unwrap());
        let names: Vec<String> =
            a.entries().unwrap().map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["metadata.json", "channels/D0.bin"]);
        assert_eq!(std::fs::metadata(&path).unwrap().len() % BLOCK, 0);
    }
}

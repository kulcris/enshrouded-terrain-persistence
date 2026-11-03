//! Crash-safe terrain snapshot journal for 8x8x8 voxel bricks.
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

pub const BRICK_VOXELS: usize = 512;
pub const DIRECTORY: &str = "terrain-delta";
pub type Brick = [u16; BRICK_VOXELS];
const MAGIC: &[u8; 8] = b"EVJNL001";
const SIZE: usize = 8 + 12 + BRICK_VOXELS * 2 + 4;
const BUILDING: u16 = 0x0080;
const OMIT: u16 = 0xffff;

/// Minimum voxel coordinates of an 8-cube brick; every component must be 8-aligned.
pub type BrickKey = [u32; 3];

#[derive(Debug)]
pub struct Journal {
    dir: PathBuf,
    bricks: BTreeMap<BrickKey, Brick>,
}

impl Journal {
    /// Opens (and validates) a journal directory. Any malformed record fails open.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let dir = path.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut bricks = BTreeMap::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".brick") {
                continue;
            }
            let metadata = entry.metadata()?;
            if !metadata.is_file() || metadata.len() != SIZE as u64 {
                return Err(invalid("invalid journal brick length"));
            }
            let bytes = fs::read(entry.path())?;
            let (key, brick) = decode(&bytes)?;
            if entry.file_name() != std::ffi::OsString::from(file_name(key))
                || bricks.insert(key, brick).is_some()
            {
                return Err(invalid("journal key/file mismatch or duplicate"));
            }
        }
        Ok(Self { dir, bricks })
    }

    /// Persists a complete snapshot; building entries remove earlier overrides.
    pub fn record_snapshot(&mut self, key: BrickKey, snapshot: &Brick) -> io::Result<()> {
        check_key(key)?;
        let mut stored = *snapshot;
        for word in &mut stored {
            if is_building(*word) {
                *word = OMIT;
            }
        }
        if self.bricks.get(&key) == Some(&stored) {
            return Ok(());
        }
        let path = self.dir.join(file_name(key));
        if stored.iter().all(|&v| v == OMIT) {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            self.bricks.remove(&key);
            return Ok(());
        }
        let bytes = encode(key, &stored);
        let tmp = self.dir.join(format!("{}.tmp", file_name(key)));
        let result: io::Result<()> = (|| {
            use std::io::Write;
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result?;
        self.bricks.insert(key, stored);
        Ok(())
    }

    /// Persists only the supplied air/terrain changes, retaining earlier edits
    /// at every position marked OMIT. Building words are invalid delta input.
    pub fn record_delta(&mut self, key: BrickKey, delta: &Brick) -> io::Result<()> {
        check_key(key)?;
        for &word in delta {
            if word != OMIT && is_building(word) {
                return Err(invalid("journal delta contains building word"));
            }
        }
        if delta.iter().all(|&word| word == OMIT) {
            return Ok(());
        }
        let mut merged = self
            .bricks
            .get(&key)
            .copied()
            .unwrap_or([OMIT; BRICK_VOXELS]);
        for (stored, &word) in merged.iter_mut().zip(delta) {
            if word != OMIT {
                *stored = word;
            }
        }
        self.record_snapshot(key, &merged)
    }

    pub fn keys(&self) -> Vec<BrickKey> {
        self.bricks.keys().copied().collect()
    }

    /// Applies saved terrain and air to current terrain/air, preserving buildings.
    pub fn merge(&self, key: BrickKey, current: &Brick) -> Option<Brick> {
        let saved = self.bricks.get(&key)?;
        let mut merged = *current;
        for i in 0..BRICK_VOXELS {
            if saved[i] != OMIT && !is_building(current[i]) {
                merged[i] = saved[i];
            }
        }
        Some(merged)
    }
}

fn is_building(word: u16) -> bool {
    (word & 0xff) >= BUILDING
}
fn invalid(s: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
fn check_key(key: BrickKey) -> io::Result<()> {
    if key.iter().any(|v| v % 8 != 0) {
        Err(invalid("brick key is not 8-aligned"))
    } else {
        Ok(())
    }
}
fn file_name(k: BrickKey) -> String {
    format!("{}-{}-{}.brick", k[0], k[1], k[2])
}
fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x01000193)
    })
}
fn encode(key: BrickKey, brick: &Brick) -> Vec<u8> {
    let mut b = Vec::with_capacity(SIZE);
    b.extend_from_slice(MAGIC);
    for c in key {
        b.extend_from_slice(&c.to_le_bytes());
    }
    for w in brick {
        b.extend_from_slice(&w.to_le_bytes());
    }
    let sum = checksum(&b);
    b.extend_from_slice(&sum.to_le_bytes());
    b
}
fn decode(b: &[u8]) -> io::Result<(BrickKey, Brick)> {
    if b.len() != SIZE || &b[..8] != MAGIC {
        return Err(invalid("invalid journal brick header"));
    }
    let end = SIZE - 4;
    let sum = u32::from_le_bytes(b[end..].try_into().unwrap());
    if checksum(&b[..end]) != sum {
        return Err(invalid("journal checksum mismatch"));
    }
    let key = [
        u32::from_le_bytes(b[8..12].try_into().unwrap()),
        u32::from_le_bytes(b[12..16].try_into().unwrap()),
        u32::from_le_bytes(b[16..20].try_into().unwrap()),
    ];
    check_key(key)?;
    let mut brick = [OMIT; BRICK_VOXELS];
    for (i, slot) in brick.iter_mut().enumerate() {
        *slot = u16::from_le_bytes(b[20 + i * 2..22 + i * 2].try_into().unwrap());
        if *slot != OMIT && is_building(*slot) {
            return Err(invalid("journal contains building override"));
        }
    }
    Ok((key, brick))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "voxel-journal-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn word(m: u8, fill: u8) -> u16 {
        u16::from_le_bytes([m, fill])
    }
    #[test]
    fn recovers_and_repeated_snapshots_replace_old_values() {
        let p = dir();
        let k = [8, 16, 24];
        let mut j = Journal::open(&p).unwrap();
        let mut a = [word(1, 100); 512];
        a[3] = word(0, 0);
        j.record_snapshot(k, &a).unwrap();
        let mut b = a;
        b[0] = word(4, 200);
        j.record_snapshot(k, &b).unwrap();
        drop(j);
        let j = Journal::open(&p).unwrap();
        assert_eq!(j.keys(), vec![k]);
        let got = j.merge(k, &[word(2, 9); 512]).unwrap();
        assert_eq!(got[0], b[0]);
        assert_eq!(got[3], b[3]);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn buildings_remove_old_override_and_survive_merge() {
        let p = dir();
        let k = [0, 0, 0];
        let mut j = Journal::open(&p).unwrap();
        let a = [word(2, 1); 512];
        j.record_snapshot(k, &a).unwrap();
        let mut b = a;
        b[7] = word(128, 4);
        j.record_snapshot(k, &b).unwrap();
        let mut live = [word(5, 2); 512];
        live[7] = word(130, 7);
        let got = j.merge(k, &live).unwrap();
        assert_eq!(got[7], live[7]);
        assert_eq!(got[6], a[6]);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn corruption_is_reported_and_not_rewritten() {
        let p = dir();
        let k = [0, 8, 0];
        let mut j = Journal::open(&p).unwrap();
        j.record_snapshot(k, &[word(1, 2); 512]).unwrap();
        drop(j);
        let f = p.join(file_name(k));
        let mut bytes = fs::read(&f).unwrap();
        bytes[22] ^= 1;
        fs::write(&f, &bytes).unwrap();
        assert_eq!(
            Journal::open(&p).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(f).unwrap(), bytes);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn sparse_deltas_accumulate_repeat_and_survive_reload() {
        let p = dir();
        let k = [8, 16, 24];
        let mut j = Journal::open(&p).unwrap();
        let mut first = [OMIT; BRICK_VOXELS];
        first[2] = word(0, 0);
        first[9] = word(3, 20);
        j.record_delta(k, &first).unwrap();
        let mut second = [OMIT; BRICK_VOXELS];
        second[400] = word(4, 30);
        second[9] = word(5, 40);
        j.record_delta(k, &second).unwrap();
        // Repeating a delta is idempotent and untouched positions remain intact.
        j.record_delta(k, &second).unwrap();
        drop(j);
        let j = Journal::open(&p).unwrap();
        let got = j.merge(k, &[word(1, 7); BRICK_VOXELS]).unwrap();
        assert_eq!(got[2], first[2]);
        assert_eq!(got[9], second[9]);
        assert_eq!(got[400], second[400]);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn sparse_delta_preserves_natural_terrain_and_empty_delta_creates_nothing() {
        let p = dir();
        let k = [0, 0, 0];
        let mut j = Journal::open(&p).unwrap();
        j.record_delta(k, &[OMIT; BRICK_VOXELS]).unwrap();
        assert!(j.keys().is_empty());
        assert!(fs::read_dir(&p).unwrap().next().is_none());
        let mut delta = [OMIT; BRICK_VOXELS];
        delta[4] = word(2, 9);
        j.record_delta(k, &delta).unwrap();
        let mut current = [word(7, 11); BRICK_VOXELS];
        current[4] = word(1, 3);
        let got = j.merge(k, &current).unwrap();
        assert_eq!(got[4], delta[4]);
        assert_eq!(got[5], current[5]);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn malformed_building_delta_does_not_change_existing_record() {
        let p = dir();
        let k = [0, 8, 0];
        let mut j = Journal::open(&p).unwrap();
        let mut initial = [OMIT; BRICK_VOXELS];
        initial[1] = word(3, 6);
        j.record_delta(k, &initial).unwrap();
        let path = p.join(file_name(k));
        let before = fs::read(&path).unwrap();
        let mut bad = [OMIT; BRICK_VOXELS];
        bad[1] = word(4, 7);
        bad[2] = word(128, 0);
        assert_eq!(
            j.record_delta(k, &bad).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(j.bricks[&k], initial);
        fs::remove_dir_all(p).unwrap();
    }
}

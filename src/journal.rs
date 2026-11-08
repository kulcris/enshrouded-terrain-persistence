//! Durable sparse terrain overrides backed by the world's shared redb store.
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use std::{fs, io, path::Path};

use crate::{
    codec,
    store::{self, META, Store},
};

pub const BRICK_VOXELS: usize = 512;
pub const DIRECTORY: &str = "terrain-delta";
pub type Brick = [u16; BRICK_VOXELS];
pub type BrickKey = [u32; 3];

const TERRAIN: TableDefinition<u128, &[u8]> = TableDefinition::new("terrain");
const MAGIC: &[u8; 8] = b"EVJNL001";
const LEGACY_SIZE: usize = 8 + 12 + BRICK_VOXELS * 2 + 4;
const BUILDING: u16 = 0x0080;
const OMIT: u16 = 0xffff;
const IMPORT_MARKER: &str = "terrain_imported";

#[derive(Clone, Debug)]
pub struct Journal {
    store: Store,
}

impl Journal {
    /// Opens a world-local database and imports legacy `.brick` records once.
    /// Legacy files remain untouched as rollback copies.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let dir = path.as_ref();
        fs::create_dir_all(dir)?;
        let db_path = dir.join("records.redb");
        let db_existed = db_path.exists();
        let old = if db_existed {
            None
        } else {
            Some(legacy_records(dir)?)
        };
        let store = Store::open(&db_path)?;
        let marker = {
            let tx = store.db.begin_read().map_err(store::err)?;
            let table = tx.open_table(META).map_err(store::err)?;
            table
                .get(IMPORT_MARKER)
                .map_err(store::err)?
                .map(|v| v.value())
        };
        if marker.is_some_and(|v| v != 1) {
            return Err(invalid("invalid terrain import marker"));
        }

        if marker.is_none() {
            let tx = store.db.begin_write().map_err(store::err)?;
            {
                let mut terrain = tx.open_table(TERRAIN).map_err(store::err)?;
                let has_rows = terrain
                    .iter()
                    .map_err(store::err)?
                    .next()
                    .transpose()
                    .map_err(store::err)?
                    .is_some();
                let legacy = match old {
                    Some(records) => records,
                    None => legacy_records(dir)?,
                };
                if db_existed && has_rows && !legacy.is_empty() {
                    return Err(invalid(
                        "legacy terrain conflicts with existing database rows",
                    ));
                }
                // A database created immediately before a crash can be empty
                // and unmarked. Importing then is safe and makes startup resumable.
                if !db_existed || !has_rows {
                    for (key, words) in legacy {
                        let encoded = codec::encode(&words);
                        terrain
                            .insert(store::key(key), encoded.as_slice())
                            .map_err(store::err)?;
                    }
                } else if !legacy.is_empty() {
                    return Err(invalid(
                        "legacy terrain conflicts with existing database rows",
                    ));
                }
            }
            {
                let mut meta = tx.open_table(META).map_err(store::err)?;
                meta.insert(IMPORT_MARKER, 1).map_err(store::err)?;
            }
            tx.commit().map_err(store::err)?;
        }
        let journal = Self { store };
        journal.validate_all()?;
        Ok(journal)
    }

    /// Persists a complete image. Building cells clear prior terrain overrides.
    #[cfg(test)]
    pub fn record_snapshot(&mut self, key: BrickKey, snapshot: &Brick) -> io::Result<()> {
        check_key(key)?;
        let mut words = *snapshot;
        for word in &mut words {
            if is_building(*word) {
                *word = OMIT;
            }
        }
        let tx = self.store.db.begin_write().map_err(store::err)?;
        {
            let mut table = tx.open_table(TERRAIN).map_err(store::err)?;
            if words.iter().all(|&w| w == OMIT) {
                table.remove(store::key(key)).map_err(store::err)?;
            } else {
                let encoded = codec::encode(&words);
                table
                    .insert(store::key(key), encoded.as_slice())
                    .map_err(store::err)?;
            }
        }
        tx.commit().map_err(store::err)
    }

    /// Applies terrain/air entries from a sparse delta. OMIT leaves a value
    /// unchanged; building words are rejected (snapshots use record_snapshot).
    pub fn record_delta(&mut self, key: BrickKey, delta: &Brick) -> io::Result<()> {
        self.record_deltas(&[(key, *delta)])
    }

    /// Atomically applies sparse deltas in one database transaction.
    pub fn record_deltas(&mut self, deltas: &[(BrickKey, Brick)]) -> io::Result<()> {
        for (key, delta) in deltas {
            check_key(*key)?;
            if delta.iter().any(|&word| word != OMIT && is_building(word)) {
                return Err(invalid("journal delta contains building word"));
            }
        }
        if deltas
            .iter()
            .all(|(_, delta)| delta.iter().all(|&word| word == OMIT))
        {
            return Ok(());
        }
        let tx = self.store.db.begin_write().map_err(store::err)?;
        {
            let mut table = tx.open_table(TERRAIN).map_err(store::err)?;
            for (key, delta) in deltas {
                if delta.iter().all(|&word| word == OMIT) {
                    continue;
                }
                let id = store::key(*key);
                let mut words = match table.get(id).map_err(store::err)? {
                    Some(value) => decode_record(value.value())?,
                    None => [OMIT; BRICK_VOXELS],
                };
                for (stored, &word) in words.iter_mut().zip(delta) {
                    if word != OMIT {
                        *stored = word;
                    }
                }
                if words.iter().all(|&word| word == OMIT) {
                    table.remove(id).map_err(store::err)?;
                } else {
                    let encoded = codec::encode(&words);
                    table.insert(id, encoded.as_slice()).map_err(store::err)?;
                }
            }
        }
        tx.commit().map_err(store::err)
    }

    pub fn keys(&self) -> io::Result<Vec<BrickKey>> {
        let tx = self.store.db.begin_read().map_err(store::err)?;
        let table = tx.open_table(TERRAIN).map_err(store::err)?;
        let mut result = Vec::new();
        for entry in table.iter().map_err(store::err)? {
            let (k, value) = entry.map_err(store::err)?;
            let key = store::unkey(k.value());
            check_key(key)?;
            let _ = value;
            result.push(key);
        }
        Ok(result)
    }

    /// Merges stored terrain/air over current non-building voxels.
    pub fn merge(&self, key: BrickKey, current: &Brick) -> io::Result<Option<Brick>> {
        check_key(key)?;
        let tx = self.store.db.begin_read().map_err(store::err)?;
        let table = tx.open_table(TERRAIN).map_err(store::err)?;
        let Some(value) = table.get(store::key(key)).map_err(store::err)? else {
            return Ok(None);
        };
        let saved = decode_record(value.value())?;
        validate_words(&saved)?;
        let mut merged = *current;
        for i in 0..BRICK_VOXELS {
            if saved[i] != OMIT && !is_building(current[i]) {
                merged[i] = saved[i];
            }
        }
        Ok(Some(merged))
    }

    pub fn store(&self) -> Store {
        self.store.clone()
    }

    fn validate_all(&self) -> io::Result<()> {
        let tx = self.store.db.begin_read().map_err(store::err)?;
        let table = tx.open_table(TERRAIN).map_err(store::err)?;
        for entry in table.iter().map_err(store::err)? {
            let (k, value) = entry.map_err(store::err)?;
            check_key(store::unkey(k.value()))?;
            validate_words(&decode_record(value.value())?)?;
        }
        Ok(())
    }
}

/// Reads and validates the old fixed-size `.brick` format without modifying it.
pub fn legacy_records(path: &Path) -> io::Result<Vec<(BrickKey, Brick)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".tmp") {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("brick") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            return Err(invalid("legacy brick is not a file"));
        }
        let metadata = entry.metadata()?;
        if metadata.len() != LEGACY_SIZE as u64 {
            return Err(invalid("invalid legacy journal brick length"));
        }
        // The expected size is checked before reading, and read is capped as a
        // second bound in case the file changes between metadata and open.
        let file = fs::File::open(entry.path())?;
        use io::Read;
        let mut bytes = Vec::with_capacity(LEGACY_SIZE);
        file.take((LEGACY_SIZE + 1) as u64)
            .read_to_end(&mut bytes)?;
        let (key, words) = decode_legacy(&bytes)?;
        if name != std::ffi::OsString::from(legacy_file_name(key)) {
            return Err(invalid("legacy key/file mismatch or duplicate"));
        }
        records.push((key, words));
    }
    Ok(records)
}

fn decode_record(bytes: &[u8]) -> io::Result<Brick> {
    let words = codec::decode(bytes)?;
    validate_words(&words)?;
    Ok(words)
}
fn validate_words(words: &Brick) -> io::Result<()> {
    if words.iter().any(|&word| word != OMIT && is_building(word)) {
        return Err(invalid("journal contains building override"));
    }
    Ok(())
}
fn is_building(word: u16) -> bool {
    (word & 0xff) >= BUILDING
}
fn check_key(key: BrickKey) -> io::Result<()> {
    if key.iter().any(|v| v % 8 != 0) {
        Err(invalid("brick key is not 8-aligned"))
    } else {
        Ok(())
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn legacy_file_name(k: BrickKey) -> String {
    format!("{}-{}-{}.brick", k[0], k[1], k[2])
}
fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x01000193)
    })
}
fn decode_legacy(bytes: &[u8]) -> io::Result<(BrickKey, Brick)> {
    if bytes.len() != LEGACY_SIZE || &bytes[..8] != MAGIC {
        return Err(invalid("invalid legacy journal brick header"));
    }
    let end = LEGACY_SIZE - 4;
    let sum = u32::from_le_bytes(bytes[end..].try_into().unwrap());
    if checksum(&bytes[..end]) != sum {
        return Err(invalid("legacy journal checksum mismatch"));
    }
    let key = [
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    ];
    check_key(key)?;
    let mut words = [OMIT; BRICK_VOXELS];
    for (i, word) in words.iter_mut().enumerate() {
        *word = u16::from_le_bytes(bytes[20 + i * 2..22 + i * 2].try_into().unwrap());
    }
    validate_words(&words)?;
    Ok((key, words))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn dir() -> PathBuf {
        loop {
            let path = std::env::temp_dir().join(format!(
                "terrain-journal-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("could not create test directory: {e}"),
            }
        }
    }
    fn word(m: u8, fill: u8) -> u16 {
        u16::from_le_bytes([m, fill])
    }

    #[test]
    fn reloads_repeated_edits_and_preserves_buildings() {
        let p = dir();
        let k = [8, 16, 24];
        let mut j = Journal::open(&p).unwrap();
        let mut first = [word(2, 200); 512];
        first[3] = word(0, 0);
        j.record_snapshot(k, &first).unwrap();
        let mut second = [OMIT; 512];
        second[0] = word(7, 90);
        second[3] = word(4, 80);
        j.record_delta(k, &second).unwrap();
        drop(j);
        let j = Journal::open(&p).unwrap();
        let mut current = [word(1, 50); 512];
        current[0] = word(128, 0);
        let merged = j.merge(k, &current).unwrap().unwrap();
        assert_eq!(merged[0], current[0]);
        assert_eq!(merged[3], second[3]);
        assert_eq!(j.keys().unwrap(), vec![k]);
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn snapshot_buildings_clear_existing_override() {
        let p = dir();
        let k = [0, 0, 0];
        let mut j = Journal::open(&p).unwrap();
        let terrain = [word(2, 20); 512];
        j.record_snapshot(k, &terrain).unwrap();
        let mut next = terrain;
        next[4] = word(128, 0);
        j.record_snapshot(k, &next).unwrap();
        let current = [word(1, 5); 512];
        let merged = j.merge(k, &current).unwrap().unwrap();
        assert_eq!(merged[4], current[4]);
        assert_eq!(merged[5], terrain[5]);
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn legacy_is_validated_and_imported_once_without_removing_backup() {
        let p = dir();
        let k = [16, 0, 8];
        let mut words = [OMIT; 512];
        words[10] = word(4, 40);
        fs::write(p.join(legacy_file_name(k)), encode_legacy(k, &words)).unwrap();
        let j = Journal::open(&p).unwrap();
        assert_eq!(j.keys().unwrap(), vec![k]);
        drop(j);
        assert!(p.join(legacy_file_name(k)).exists());
        // The marked store wins; a later conflicting legacy file is not merged.
        let conflict = [32, 0, 0];
        fs::write(
            p.join(legacy_file_name(conflict)),
            encode_legacy(conflict, &words),
        )
        .unwrap();
        let j = Journal::open(&p).unwrap();
        assert_eq!(j.keys().unwrap(), vec![k]);
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn malformed_legacy_fails_without_rewriting_source() {
        let p = dir();
        let k = [0, 8, 0];
        let bytes = vec![0u8; LEGACY_SIZE];
        let file = p.join(legacy_file_name(k));
        fs::write(&file, &bytes).unwrap();
        assert_eq!(
            legacy_records(&p).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&file).unwrap(), bytes);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn invalid_batch_is_atomic() {
        let p = dir();
        let mut j = Journal::open(&p).unwrap();
        let good = ([0, 0, 0], [word(2, 3); 512]);
        let bad = ([1, 0, 0], [word(4, 5); 512]);
        assert_eq!(
            j.record_deltas(&[good, bad]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(j.keys().unwrap().is_empty());
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn sparse_encoding_stays_compact_and_round_trips() {
        let p = dir();
        let mut j = Journal::open(&p).unwrap();
        let started = std::time::Instant::now();
        for i in 0..10_000u32 {
            if i % 250 == 0 {
                // Each transaction models a bounded runtime batch.
                let mut batch = Vec::with_capacity(250);
                for n in i..i + 250 {
                    let key = [n * 8, 0, 0];
                    let mut delta = [OMIT; 512];
                    delta[n as usize % 512] = word(3, 20);
                    batch.push((key, delta));
                }
                j.record_deltas(&batch).unwrap();
            }
        }
        assert_eq!(j.keys().unwrap().len(), 10_000);
        drop(j);
        let j = Journal::open(&p).unwrap();
        assert_eq!(j.keys().unwrap().len(), 10_000);
        let size = fs::metadata(p.join("records.redb")).unwrap().len();
        eprintln!(
            "10,000 sparse terrain bricks: {size} bytes in {:?}",
            started.elapsed()
        );
        drop(j);
        fs::remove_dir_all(p).unwrap();
    }
}

#[cfg(test)]
fn encode_legacy(key: BrickKey, words: &Brick) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(LEGACY_SIZE);
    bytes.extend_from_slice(MAGIC);
    for c in key {
        bytes.extend_from_slice(&c.to_le_bytes());
    }
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    let sum = checksum(&bytes);
    bytes.extend_from_slice(&sum.to_le_bytes());
    bytes
}

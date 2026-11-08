//! Indexed, compact ore timers sharing the world's terrain database.
use crate::{
    codec,
    hooks::{Brick, Image},
    store::{self, META, Store},
};
use redb::{ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::Deserialize;
use std::{collections::BTreeSet, fs, io, path::Path};
const ORES: TableDefinition<u128, &[u8]> = TableDefinition::new("ore_timers");
const DUE: TableDefinition<(u64, u128), u8> = TableDefinition::new("ore_deadlines");
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    key: Brick,
    due: u64,
    original: Vec<u16>,
}
pub struct Ores {
    store: Store,
}
impl Ores {
    #[cfg(test)]
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref();
        let store = Store::open(dir.join("records.redb"))?;
        Self::open_shared(dir, store)
    }
    pub fn open_shared(dir: impl AsRef<Path>, store: Store) -> io::Result<Self> {
        let tx = store.db.begin_write().map_err(store::err)?;
        {
            let mut metadata = tx.open_table(META).map_err(store::err)?;
            let mut timers = tx.open_table(ORES).map_err(store::err)?;
            let mut deadlines = tx.open_table(DUE).map_err(store::err)?;
            if metadata.get("ores_imported").map_err(store::err)?.is_none() {
                let legacy = legacy_records(dir.as_ref())?;
                if timers.len().map_err(store::err)? != 0 {
                    return Err(invalid("ore timers exist without import marker"));
                }
                for record in legacy {
                    let image: Image = record
                        .original
                        .try_into()
                        .map_err(|_| invalid("invalid ore image size"))?;
                    let k = store::key(record.key);
                    let value = pack(record.due, &image);
                    timers.insert(k, value.as_slice()).map_err(store::err)?;
                    deadlines.insert((record.due, k), 0).map_err(store::err)?;
                }
                metadata.insert("ores_imported", 1).map_err(store::err)?;
            }
        }
        tx.commit().map_err(store::err)?;
        Ok(Self { store })
    }
    #[cfg(test)]
    pub fn schedule(&mut self, key: Brick, original: &Image, due: u64) -> io::Result<()> {
        self.schedule_many(&[(key, *original)], due)
    }
    pub fn schedule_many(&mut self, updates: &[(Brick, Image)], due: u64) -> io::Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        for (k, image) in updates {
            validate(*k, image)?;
        }
        let tx = self.store.db.begin_write().map_err(store::err)?;
        {
            let mut timers = tx.open_table(ORES).map_err(store::err)?;
            let mut deadlines = tx.open_table(DUE).map_err(store::err)?;
            for (key, original) in updates {
                let k = store::key(*key);
                let previous = timers
                    .get(k)
                    .map_err(store::err)?
                    .map(|v| unpack(v.value()))
                    .transpose()?;
                let mut merged = previous.as_ref().map(|(_, v)| *v).unwrap_or([0xffff; 512]);
                if let Some((old_due, _)) = previous {
                    deadlines.remove((old_due, k)).map_err(store::err)?;
                }
                for (saved, &value) in merged.iter_mut().zip(original) {
                    if *saved == 0xffff {
                        *saved = value;
                    }
                }
                let value = pack(due, &merged);
                timers.insert(k, value.as_slice()).map_err(store::err)?;
                deadlines.insert((due, k), 0).map_err(store::err)?;
            }
        }
        tx.commit().map_err(store::err)
    }
    pub fn due(&self, now: u64, limit: usize) -> io::Result<Vec<(Brick, Image)>> {
        let tx = self.store.db.begin_read().map_err(store::err)?;
        let timers = tx.open_table(ORES).map_err(store::err)?;
        let deadlines = tx.open_table(DUE).map_err(store::err)?;
        let mut result = Vec::new();
        for entry in deadlines
            .range(..=(now, u128::MAX))
            .map_err(store::err)?
            .take(limit)
        {
            let (entry, _) = entry.map_err(store::err)?;
            let (due, k) = entry.value();
            let value = timers
                .get(k)
                .map_err(store::err)?
                .ok_or_else(|| invalid("deadline has no ore timer"))?;
            let (stored_due, image) = unpack(value.value())?;
            if stored_due != due {
                return Err(invalid("ore deadline index mismatch"));
            }
            result.push((store::unkey(k), image));
        }
        Ok(result)
    }
    pub fn remove(&mut self, key: Brick) -> io::Result<()> {
        let tx = self.store.db.begin_write().map_err(store::err)?;
        {
            let mut timers = tx.open_table(ORES).map_err(store::err)?;
            let mut deadlines = tx.open_table(DUE).map_err(store::err)?;
            let k = store::key(key);
            let old = timers
                .remove(k)
                .map_err(store::err)?
                .map(|v| unpack(v.value()))
                .transpose()?;
            if let Some((due, _)) = old {
                deadlines.remove((due, k)).map_err(store::err)?;
            }
        }
        tx.commit().map_err(store::err)
    }
}
pub(crate) fn legacy_records(dir: &Path) -> io::Result<Vec<Record>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    let mut keys = BTreeSet::new();
    for file in fs::read_dir(dir)? {
        let file = file?;
        if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let record: Record = serde_json::from_slice(&fs::read(file.path())?)?;
        validate(record.key, &record.original)?;
        if file.file_name()
            != std::ffi::OsString::from(format!(
                "{}-{}-{}.json",
                record.key[0], record.key[1], record.key[2]
            ))
            || !keys.insert(record.key)
        {
            return Err(invalid("invalid ore regeneration filename"));
        }
        records.push(record);
    }
    Ok(records)
}
fn validate(key: Brick, original: &[u16]) -> io::Result<()> {
    if key.iter().any(|v| v % 8 != 0)
        || original.len() != 512
        || original
            .iter()
            .any(|&v| v != 0xffff && (v & 255 == 0 || v & 255 >= 128))
    {
        return Err(invalid("invalid ore regeneration record"));
    }
    Ok(())
}
fn pack(due: u64, image: &Image) -> Vec<u8> {
    let mut v = due.to_le_bytes().to_vec();
    v.extend(codec::encode(image));
    v
}
fn unpack(value: &[u8]) -> io::Result<(u64, Image)> {
    if value.len() < 9 {
        return Err(invalid("truncated ore timer"));
    }
    Ok((
        u64::from_le_bytes(value[..8].try_into().unwrap()),
        codec::decode(&value[8..])?,
    ))
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timer_survives_restart_and_preserves_first_ore_shape_only() {
        let p = std::env::temp_dir().join(format!("ore-test-{}-{}", std::process::id(), now()));
        let key = [8, 16, 24];
        let mut first = [0xffff; 512];
        first[5] = 0xff23;
        let mut q = Ores::open(&p).unwrap();
        q.schedule(key, &first, 100).unwrap();
        let mut next = first;
        next[5] = 0x8023;
        next[6] = 0xff23;
        q.schedule(key, &next, 200).unwrap();
        drop(q);
        let mut q = Ores::open(&p).unwrap();
        assert!(q.due(199, 8).unwrap().is_empty());
        let result = q.due(200, 8).unwrap();
        assert_eq!(result[0].1[5], 0xff23);
        assert_eq!(result[0].1[6], 0xff23);
        assert_eq!(result[0].1[7], 0xffff);
        q.remove(key).unwrap();
        assert!(q.due(300, 8).unwrap().is_empty());
        drop(q);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn deadlines_are_sorted_and_old_deadlines_removed() {
        let p = std::env::temp_dir().join(format!("ore-index-{}-{}", std::process::id(), now()));
        let mut q = Ores::open(&p).unwrap();
        let image = [0xff23; 512];
        q.schedule([0, 0, 0], &image, 100).unwrap();
        q.schedule([8, 0, 0], &image, 50).unwrap();
        q.schedule([0, 0, 0], &image, 150).unwrap();
        assert_eq!(
            q.due(100, 8)
                .unwrap()
                .iter()
                .map(|v| v.0)
                .collect::<Vec<_>>(),
            vec![[8, 0, 0]]
        );
        drop(q);
        fs::remove_dir_all(p).unwrap();
    }
}

//! Durable, sparse original ore values and their regeneration deadlines.
use crate::hooks::{Brick, Image};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    key: Brick,
    due: u64,
    original: Vec<u16>,
}
pub struct Ores {
    dir: PathBuf,
    records: BTreeMap<Brick, Record>,
}
impl Ores {
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut records = BTreeMap::new();
        for file in fs::read_dir(&dir)? {
            let file = file?;
            if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let record: Record = serde_json::from_slice(&fs::read(file.path())?)?;
            if record.key.iter().any(|v| v % 8 != 0)
                || record.original.len() != 512
                || record
                    .original
                    .iter()
                    .any(|&w| w != 0xffff && (w & 255 == 0 || w & 255 >= 128))
                || file.file_name() != std::ffi::OsString::from(name(record.key))
                || records.contains_key(&record.key)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid ore regeneration record",
                ));
            }
            records.insert(record.key, record);
        }
        Ok(Self { dir, records })
    }
    pub fn schedule(&mut self, key: Brick, original: &Image, due: u64) -> io::Result<()> {
        let mut record = self.records.get(&key).cloned().unwrap_or(Record {
            key,
            due,
            original: vec![0xffff; 512],
        });
        record.due = due;
        for (saved, &value) in record.original.iter_mut().zip(original) {
            if *saved == 0xffff {
                *saved = value;
            }
        }
        let path = self.dir.join(name(key));
        let temp = path.with_extension("tmp");
        use std::io::Write;
        let mut file = fs::File::create(&temp)?;
        file.write_all(&serde_json::to_vec(&record)?)?;
        file.sync_all()?;
        fs::rename(temp, path)?;
        self.records.insert(key, record);
        Ok(())
    }
    pub fn due(&self, now: u64, limit: usize) -> Vec<(Brick, Image)> {
        self.records
            .values()
            .filter(|r| r.due <= now)
            .take(limit)
            .map(|r| (r.key, r.original.clone().try_into().unwrap()))
            .collect()
    }
    pub fn remove(&mut self, key: Brick) -> io::Result<()> {
        match fs::remove_file(self.dir.join(name(key))) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.records.remove(&key);
        Ok(())
    }
}
fn name(k: Brick) -> String {
    format!("{}-{}-{}.json", k[0], k[1], k[2])
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
        let mut queue = Ores::open(&p).unwrap();
        queue.schedule(key, &first, 100).unwrap();
        let mut second = [0xffff; 512];
        second[5] = 0x8023;
        second[6] = 0xff23;
        queue.schedule(key, &second, 200).unwrap();
        drop(queue);
        let mut queue = Ores::open(&p).unwrap();
        assert!(queue.due(199, 8).is_empty());
        let restored = queue.due(200, 8);
        assert_eq!(restored[0].1[5], 0xff23);
        assert_eq!(restored[0].1[6], 0xff23);
        assert_eq!(restored[0].1[7], 0xffff);
        queue.remove(key).unwrap();
        assert!(Ores::open(&p).unwrap().due(300, 8).is_empty());
        fs::remove_dir_all(p).unwrap();
    }
}

//! Compact persistent timers for armed chests and resource nodes.
use crate::store::{self, Store};
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use std::io;
use std::ops::Bound::{Excluded, Included, Unbounded};

const RECORDS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("loot_timers");
const DEADLINES: TableDefinition<(&[u8; 8], &[u8; 41]), u8> =
    TableDefinition::new("loot_deadlines");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd)]
pub struct Key {
    pub kind: u8,
    pub template: [u8; 16],
    pub position: [i64; 3],
}

impl Key {
    pub fn encode(self) -> io::Result<[u8; 41]> {
        if self.kind != 1 && self.kind != 2 {
            return Err(invalid("invalid loot timer kind"));
        }
        let mut out = [0; 41];
        out[0] = self.kind;
        out[1..17].copy_from_slice(&self.template);
        for (i, n) in self.position.iter().enumerate() {
            out[17 + i * 8..25 + i * 8].copy_from_slice(&n.to_le_bytes());
        }
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() != 41 || (bytes[0] != 1 && bytes[0] != 2) {
            return Err(invalid("invalid loot timer key"));
        }
        let mut template = [0; 16];
        template.copy_from_slice(&bytes[1..17]);
        let mut position = [0; 3];
        for (i, n) in position.iter_mut().enumerate() {
            *n = i64::from_le_bytes(bytes[17 + i * 8..25 + i * 8].try_into().unwrap());
        }
        Ok(Self {
            kind: bytes[0],
            template,
            position,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub key: Key,
    pub due: u64,
    pub orientation: [f32; 4],
    pub fill_table: [u8; 16],
    pub loot_context: [u32; 2],
}

pub struct Timers {
    store: Store,
}
impl Timers {
    pub fn open_store(store: Store) -> io::Result<Self> {
        let tx = store.db.begin_write().map_err(store::err)?;
        {
            tx.open_table(RECORDS).map_err(store::err)?;
            tx.open_table(DEADLINES).map_err(store::err)?;
        }
        tx.commit().map_err(store::err)?;
        let tx = store.db.begin_read().map_err(store::err)?;
        {
            let records = tx.open_table(RECORDS).map_err(store::err)?;
            let deadlines = tx.open_table(DEADLINES).map_err(store::err)?;
            for entry in records.iter().map_err(store::err)? {
                let (key, value) = entry.map_err(store::err)?;
                let record = unpack(key.value(), value.value())?;
                let due = record.due.to_be_bytes();
                if deadlines
                    .get((&due, key.value().try_into().unwrap()))
                    .map_err(store::err)?
                    .is_none()
                {
                    return Err(invalid("loot timer has no deadline index"));
                }
            }
            for entry in deadlines.iter().map_err(store::err)? {
                let (index, _) = entry.map_err(store::err)?;
                let (due, key) = index.value();
                let record = records
                    .get(key.as_slice())
                    .map_err(store::err)?
                    .ok_or_else(|| invalid("deadline has no loot timer"))?;
                if unpack(key, record.value())?.due != u64::from_be_bytes(*due) {
                    return Err(invalid("loot deadline index mismatch"));
                }
            }
        }
        Ok(Self { store })
    }
    pub fn schedule(&mut self, record: &Record) -> io::Result<()> {
        let key = record.key.encode()?;
        validate_orientation(record.orientation)?;
        let value = pack(record)?;
        let due = record.due.to_be_bytes();
        let tx = self.store.db.begin_write().map_err(store::err)?;
        {
            let mut records = tx.open_table(RECORDS).map_err(store::err)?;
            let mut deadlines = tx.open_table(DEADLINES).map_err(store::err)?;
            let old_record = {
                records
                    .get(key.as_slice())
                    .map_err(store::err)?
                    .map(|old| unpack(&key, old.value()))
                    .transpose()?
            };
            if let Some(old_record) = old_record {
                let old_due = old_record.due.to_be_bytes();
                deadlines.remove((&old_due, &key)).map_err(store::err)?;
            }
            records
                .insert(key.as_slice(), value.as_slice())
                .map_err(store::err)?;
            deadlines.insert((&due, &key), 0).map_err(store::err)?;
        }
        tx.commit().map_err(store::err)
    }
    pub fn remove(&mut self, key: &Key) -> io::Result<()> {
        let key = key.encode()?;
        let tx = self.store.db.begin_write().map_err(store::err)?;
        {
            let mut records = tx.open_table(RECORDS).map_err(store::err)?;
            let mut deadlines = tx.open_table(DEADLINES).map_err(store::err)?;
            let old_record = records
                .get(key.as_slice())
                .map_err(store::err)?
                .map(|old| unpack(&key, old.value()))
                .transpose()?;
            if let Some(old_record) = old_record {
                let due = old_record.due.to_be_bytes();
                records.remove(key.as_slice()).map_err(store::err)?;
                deadlines.remove((&due, &key)).map_err(store::err)?;
            }
        }
        tx.commit().map_err(store::err)
    }
    pub fn get(&self, key: &Key) -> io::Result<Option<Record>> {
        let key = key.encode()?;
        let tx = self.store.db.begin_read().map_err(store::err)?;
        let records = tx.open_table(RECORDS).map_err(store::err)?;
        records
            .get(key.as_slice())
            .map_err(store::err)?
            .map(|v| unpack(&key, v.value()))
            .transpose()
    }
    #[cfg(test)]
    pub fn due(&self, now: u64, limit: usize) -> io::Result<Vec<Record>> {
        self.due_after(now, limit, None)
    }

    pub fn due_after(
        &self,
        now: u64,
        limit: usize,
        after: Option<(u64, Key)>,
    ) -> io::Result<Vec<Record>> {
        let tx = self.store.db.begin_read().map_err(store::err)?;
        let records = tx.open_table(RECORDS).map_err(store::err)?;
        let deadlines = tx.open_table(DEADLINES).map_err(store::err)?;
        let mut out = Vec::new();
        let now_bytes = now.to_be_bytes();
        let max_key = [0xff; 41];
        let after_bytes = after
            .map(|(due, key)| -> io::Result<_> { Ok((due.to_be_bytes(), key.encode()?)) })
            .transpose()?;
        let lower = match &after_bytes {
            Some((due, key)) => Excluded((due, key)),
            None => Unbounded,
        };
        for entry in deadlines
            .range((lower, Included((&now_bytes, &max_key))))
            .map_err(store::err)?
            .take(limit)
        {
            let (index, _) = entry.map_err(store::err)?;
            let (due, key) = index.value();
            let value = records
                .get(key.as_slice())
                .map_err(store::err)?
                .ok_or_else(|| invalid("deadline has no loot timer"))?;
            let record = unpack(key, value.value())?;
            if record.due != u64::from_be_bytes(*due) {
                return Err(invalid("loot deadline index mismatch"));
            }
            out.push(record);
        }
        Ok(out)
    }
}
fn pack(r: &Record) -> io::Result<[u8; 48]> {
    validate_orientation(r.orientation)?;
    let mut out = [0; 48];
    out[..8].copy_from_slice(&r.due.to_le_bytes());
    for (i, f) in r.orientation.iter().enumerate() {
        out[8 + i * 4..12 + i * 4].copy_from_slice(&f.to_le_bytes());
    }
    out[24..40].copy_from_slice(&r.fill_table);
    out[40..44].copy_from_slice(&r.loot_context[0].to_le_bytes());
    out[44..48].copy_from_slice(&r.loot_context[1].to_le_bytes());
    Ok(out)
}
fn unpack(key: &[u8], value: &[u8]) -> io::Result<Record> {
    if ![24, 40, 48].contains(&value.len()) {
        return Err(invalid("invalid loot timer value length"));
    }
    let mut orientation = [0.; 4];
    for (i, f) in orientation.iter_mut().enumerate() {
        *f = f32::from_le_bytes(value[8 + i * 4..12 + i * 4].try_into().unwrap());
    }
    validate_orientation(orientation)?;
    let mut fill_table = [0; 16];
    if value.len() >= 40 {
        fill_table.copy_from_slice(&value[24..40]);
    }
    Ok(Record {
        key: Key::decode(key)?,
        due: u64::from_le_bytes(value[..8].try_into().unwrap()),
        orientation,
        fill_table,
        loot_context: if value.len() == 48 {
            [
                u32::from_le_bytes(value[40..44].try_into().unwrap()),
                u32::from_le_bytes(value[44..48].try_into().unwrap()),
            ]
        } else {
            [0; 2]
        },
    })
}
fn validate_orientation(o: [f32; 4]) -> io::Result<()> {
    if o.iter().any(|x| !x.is_finite()) {
        Err(invalid("non-finite loot orientation"))
    } else {
        Ok(())
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "loot-timers-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn record(n: i64, due: u64) -> Record {
        Record {
            key: Key {
                kind: 1,
                template: [n as u8; 16],
                position: [n, n + 1, n + 2],
            },
            due,
            orientation: [0.0, 0.5, 1.0, -0.25],
            fill_table: [n as u8 + 10; 16],
            loot_context: [n as u32 + 1, n as u32 + 2],
        }
    }

    #[test]
    fn persists_orders_reschedules_and_removes() {
        let dir = temp_dir();
        let store = Store::open(dir.join("records.redb")).unwrap();
        let mut timers = Timers::open_store(store.clone()).unwrap();
        let a = record(1, 200);
        let b = record(2, 100);
        timers.schedule(&a).unwrap();
        timers.schedule(&b).unwrap();
        let moved_a = record(1, 50);
        timers.schedule(&moved_a).unwrap();
        drop(timers);
        drop(store);

        let store = Store::open(dir.join("records.redb")).unwrap();
        let mut timers = Timers::open_store(store.clone()).unwrap();
        assert_eq!(timers.get(&moved_a.key).unwrap(), Some(moved_a.clone()));
        assert_eq!(timers.due(100, 8).unwrap(), vec![moved_a, b]);
        timers.remove(&a.key).unwrap();
        assert_eq!(timers.get(&a.key).unwrap(), None);
        assert_eq!(timers.due(300, 8).unwrap().len(), 1);
        drop(timers);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn due_after_pages_across_sorted_deadlines() {
        let dir = temp_dir();
        let store = Store::open(dir.join("records.redb")).unwrap();
        let mut timers = Timers::open_store(store.clone()).unwrap();
        let records = [record(1, 5), record(2, 5), record(3, 7)];
        for item in &records {
            timers.schedule(item).unwrap();
        }
        let first = timers.due_after(10, 1, None).unwrap();
        assert_eq!(first.len(), 1);
        let cursor = (first[0].due, first[0].key);
        let second = timers.due_after(10, 1, Some(cursor)).unwrap();
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].key, second[0].key);
        let cursor = (second[0].due, second[0].key);
        let third = timers.due_after(10, 1, Some(cursor)).unwrap();
        assert_eq!(third, vec![records[2].clone()]);
        assert!(
            timers
                .due_after(10, 1, Some((third[0].due, third[0].key)))
                .unwrap()
                .is_empty()
        );
        drop(timers);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decodes_legacy_value_with_empty_fill_table() {
        let record = record(4, 81);
        let key = record.key.encode().unwrap();
        let mut legacy = [0; 24];
        legacy[..8].copy_from_slice(&record.due.to_le_bytes());
        for (i, value) in record.orientation.iter().enumerate() {
            legacy[8 + i * 4..12 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        let decoded = unpack(&key, &legacy).unwrap();
        assert_eq!(decoded.key, record.key);
        assert_eq!(decoded.due, record.due);
        assert_eq!(decoded.orientation, record.orientation);
        assert_eq!(decoded.fill_table, [0; 16]);

        let packed = pack(&record).unwrap();
        assert_eq!(packed.len(), 48);
        let old = unpack(&key, &packed[..40]).unwrap();
        assert_eq!(old.fill_table, record.fill_table);
        assert_eq!(old.loot_context, [0; 2]);
        assert_eq!(unpack(&key, &packed).unwrap(), record);
    }

    #[test]
    fn malformed_existing_value_fails_without_overwriting() {
        let dir = temp_dir();
        let store = Store::open(dir.join("records.redb")).unwrap();
        let bad = record(3, 77);
        let key = bad.key.encode().unwrap();
        let tx = store.db.begin_write().unwrap();
        {
            let mut records = tx.open_table(RECORDS).unwrap();
            records.insert(key.as_slice(), &[9, 8, 7][..]).unwrap();
            let mut deadlines = tx.open_table(DEADLINES).unwrap();
            deadlines.insert((&bad.due.to_be_bytes(), &key), 0).unwrap();
        }
        tx.commit().unwrap();

        let mut timers = Timers {
            store: store.clone(),
        };
        assert_eq!(
            timers.schedule(&bad).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            timers.remove(&bad.key).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let open_error = match Timers::open_store(store.clone()) {
            Ok(_) => panic!("malformed store unexpectedly opened"),
            Err(error) => error,
        };
        assert_eq!(open_error.kind(), io::ErrorKind::InvalidData);
        let tx = store.db.begin_read().unwrap();
        let records = tx.open_table(RECORDS).unwrap();
        assert_eq!(
            records.get(key.as_slice()).unwrap().unwrap().value(),
            &[9, 8, 7]
        );
        drop(records);
        drop(tx);
        drop(timers);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

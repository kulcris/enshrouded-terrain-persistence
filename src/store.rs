//! A shared transactional store for one world's terrain and ore timers.
use redb::{Database, ReadableTable, TableDefinition};
use std::{fs, io, path::Path, sync::Arc};

pub const META: TableDefinition<&str, u8> = TableDefinition::new("metadata");
#[derive(Clone, Debug)]
pub struct Store {
    pub db: Arc<Database>,
}
impl Store {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut builder = Database::builder();
        builder.set_cache_size(16 * 1024 * 1024);
        // An existing but damaged file must never silently become a new store.
        let db = if path.exists() {
            builder.open(path).map_err(err)?
        } else {
            builder.create(path).map_err(err)?
        };
        let tx = db.begin_write().map_err(err)?;
        {
            let mut metadata = tx.open_table(META).map_err(err)?;
            let version = metadata
                .get("format_version")
                .map_err(err)?
                .map(|v| v.value());
            if version.is_some_and(|v| v != 1) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported terrain database format",
                ));
            }
            if version.is_none() {
                metadata.insert("format_version", 1).map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(Self { db: Arc::new(db) })
    }
}
pub fn key(k: [u32; 3]) -> u128 {
    ((k[0] as u128) << 64) | ((k[1] as u128) << 32) | (k[2] as u128)
}
pub fn unkey(k: u128) -> [u32; 3] {
    [(k >> 64) as u32, (k >> 32) as u32, k as u32]
}
pub fn err(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(e)
}

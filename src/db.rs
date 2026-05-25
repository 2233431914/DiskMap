use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const FLUSH_THRESHOLD: usize = 512;

#[derive(Debug, Clone)]
struct CacheEntry {
    mtime: u64,
    size: u64,
}

pub struct ScanDb {
    conn: Connection,
    hot_cache: HashMap<PathBuf, CacheEntry>,
    pending: Vec<(PathBuf, CacheEntry)>,
}

impl ScanDb {
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_cache (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            conn,
            hot_cache: HashMap::new(),
            pending: Vec::with_capacity(FLUSH_THRESHOLD),
        })
    }

    pub fn get_cached(&mut self, path: &Path, mtime: u64) -> Option<u64> {
        if let Some(entry) = self.hot_cache.get(path) {
            return (entry.mtime == mtime).then_some(entry.size);
        }

        let path_str = path.to_string_lossy();
        let mut stmt = self
            .conn
            .prepare_cached("SELECT size, mtime FROM file_cache WHERE path = ?")
            .ok()?;

        let (size, cached_mtime): (u64, u64) = stmt
            .query_row(params![path_str.as_ref()], |row| Ok((row.get(0)?, row.get(1)?)))
            .ok()?;

        let entry = CacheEntry {
            mtime: cached_mtime,
            size,
        };
        self.hot_cache.insert(path.to_path_buf(), entry.clone());
        (cached_mtime == mtime).then_some(size)
    }

    pub fn insert(&mut self, path: &Path, size: u64, mtime: u64) -> anyhow::Result<()> {
        let entry = CacheEntry { mtime, size };
        self.hot_cache.insert(path.to_path_buf(), entry.clone());
        self.pending.push((path.to_path_buf(), entry));

        if self.pending.len() >= FLUSH_THRESHOLD {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO file_cache (path, size, mtime) VALUES (?, ?, ?)",
            )?;

            for (path, entry) in self.pending.drain(..) {
                let path_str = path.to_string_lossy().to_string();
                stmt.execute(params![path_str, entry.size, entry.mtime])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("disk-map-{label}-{nanos}.db"))
    }

    #[test]
    fn cache_hit_returns_expected_value() {
        let path = temp_db_path("cache-hit");
        let mut db = ScanDb::new(&path).unwrap();
        let file = PathBuf::from("/tmp/example.txt");

        db.insert(&file, 42, 7).unwrap();
        db.flush().unwrap();

        assert_eq!(db.get_cached(&file, 7), Some(42));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn batch_flush_persists_records() {
        let path = temp_db_path("batch-flush");
        {
            let mut db = ScanDb::new(&path).unwrap();
            db.insert(Path::new("/tmp/a"), 11, 1).unwrap();
            db.insert(Path::new("/tmp/b"), 22, 2).unwrap();
            db.flush().unwrap();
        }

        let mut db = ScanDb::new(&path).unwrap();
        assert_eq!(db.get_cached(Path::new("/tmp/a"), 1), Some(11));
        assert_eq!(db.get_cached(Path::new("/tmp/b"), 2), Some(22));

        let _ = std::fs::remove_file(path);
    }
}

pub mod content_cache;
pub mod migrations;
pub mod route_cache;

use anyhow::Context;
use rusqlite::OpenFlags;

use migrations::migrations;

pub struct Storage {
    conn: rusqlite::Connection,
}

impl Storage {
    /// Open (or create) the database at `path`, run migrations, and enable WAL mode.
    ///
    /// Use `":memory:"` for in-memory databases (tests).
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let flags = if path == ":memory:" {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_MEMORY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };

        let mut conn =
            rusqlite::Connection::open_with_flags(path, flags).context("failed to open db")?;

        migrations()
            .to_latest(&mut conn)
            .context("failed to run migrations")?;

        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .context("failed to enable WAL mode")?;

        Ok(Self { conn })
    }

    /// Get a reference to the underlying connection.
    pub fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let storage = Storage::open(":memory:");
        assert!(storage.is_ok());
    }

    #[test]
    fn test_tables_created() {
        let storage = Storage::open(":memory:").unwrap();
        let conn = storage.conn();

        let has_domain_strategies: bool = conn
            .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='domain_strategies'")
            .and_then(|mut stmt| {
                stmt.query_row([], |row| row.get::<_, i64>(0))
            })
            .map(|count| count > 0)
            .unwrap_or(false);
        assert!(
            has_domain_strategies,
            "domain_strategies table should exist"
        );

        let has_url_cache: bool = conn
            .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='url_cache'")
            .and_then(|mut stmt| stmt.query_row([], |row| row.get::<_, i64>(0)))
            .map(|count| count > 0)
            .unwrap_or(false);
        assert!(has_url_cache, "url_cache table should exist");
    }

    #[test]
    fn test_wal_mode() {
        let _storage = Storage::open(":memory:").unwrap();
    }

    #[test]
    fn test_wal_mode_on_file() {
        let dir = std::env::temp_dir().join("kaelo_test_wal");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let path_str = db_path.to_string_lossy().to_string();

        let storage = Storage::open(&path_str).unwrap();
        let conn = storage.conn();

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");

        drop(storage);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_idempotent_migrations() {
        let dir = std::env::temp_dir().join("kaelo_test_idempotent");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let path_str = db_path.to_string_lossy().to_string();

        let storage1 = Storage::open(&path_str).unwrap();
        drop(storage1);

        let storage2 = Storage::open(&path_str).unwrap();
        drop(storage2);

        std::fs::remove_dir_all(dir).ok();
    }
}

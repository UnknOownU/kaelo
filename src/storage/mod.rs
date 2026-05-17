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

    pub fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }

    #[allow(clippy::too_many_arguments)]
    pub fn store_evidence(
        &self,
        url: &str,
        content_hash: &str,
        strategy: &str,
        retrieved_at: &str,
        confidence: &str,
        citations_json: Option<&str>,
        diagnostics_json: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO fetch_evidence (url, content_hash, strategy, retrieved_at, confidence, citations_json, diagnostics_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![url, content_hash, strategy, retrieved_at, confidence, citations_json, diagnostics_json],
        )?;
        Ok(())
    }

    pub fn get_evidence(
        &self,
        url: &str,
    ) -> anyhow::Result<Option<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, strategy, confidence, retrieved_at FROM fetch_evidence WHERE url = ?1 ORDER BY id DESC LIMIT 1"
        )?;
        let mut rows = stmt.query(rusqlite::params![url])?;
        if let Some(row) = rows.next()? {
            Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        } else {
            Ok(None)
        }
    }

    pub fn store_page_history(
        &self,
        url: &str,
        content_hash: &str,
        content_body: &str,
        strategy: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO page_history (url, content_hash, content_body, strategy) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![url, content_hash, content_body, strategy],
        )?;
        self.conn.execute(
            "DELETE FROM page_history WHERE url = ?1 AND id NOT IN (SELECT id FROM page_history WHERE url = ?1 ORDER BY fetched_at DESC LIMIT 10)",
            rusqlite::params![url],
        )?;
        Ok(())
    }

    pub fn get_page_history(
        &self,
        url: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash, content_body, strategy, fetched_at FROM page_history WHERE url = ?1 ORDER BY fetched_at DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(rusqlite::params![url, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

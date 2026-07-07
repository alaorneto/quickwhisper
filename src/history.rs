//! Transcription history, stored in SQLite at
//! `~/.local/share/quickwhisper/history.db`.
//!
//! Timestamps are stored in UTC (ISO 8601); display conversion to local time
//! is done by SQLite itself, so no date/time crate is needed.

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Entry {
    pub id: i64,
    /// UTC ISO 8601, as stored.
    pub created_at: String,
    /// Same instant in the machine's local time, for display.
    pub created_at_local: String,
    pub text: String,
    pub duration_ms: i64,
    /// Language used (fixed or whisper-detected), when known.
    pub lang: Option<String>,
    pub model: String,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open() -> Result<Self> {
        let path = crate::config::history_db_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("abrindo {}", path.display()))?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at  TEXT NOT NULL,
                text        TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                lang        TEXT,
                model       TEXT NOT NULL
            );",
        )
        .context("criando schema do histórico")?;
        Ok(Self { conn })
    }

    pub fn insert(
        &self,
        text: &str,
        duration_ms: i64,
        lang: Option<&str>,
        model: &str,
    ) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO transcriptions (created_at, text, duration_ms, lang, model)
                 VALUES (strftime('%Y-%m-%dT%H:%M:%SZ','now'), ?1, ?2, ?3, ?4)",
                (text, duration_ms, lang, model),
            )
            .context("gravando transcrição no histórico")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Newest first.
    pub fn list(&self, limit: usize) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at,
                    strftime('%Y-%m-%d %H:%M:%S', created_at, 'localtime'),
                    text, duration_ms, lang, model
             FROM transcriptions ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(Entry {
                id: row.get(0)?,
                created_at: row.get(1)?,
                created_at_local: row.get(2)?,
                text: row.get(3)?,
                duration_ms: row.get(4)?,
                lang: row.get(5)?,
                model: row.get(6)?,
            })
        })?;
        rows.collect::<Result<_, _>>().context("lendo histórico")
    }

    pub fn get(&self, id: i64) -> Result<Entry> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at,
                    strftime('%Y-%m-%d %H:%M:%S', created_at, 'localtime'),
                    text, duration_ms, lang, model
             FROM transcriptions WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(Entry {
                id: row.get(0)?,
                created_at: row.get(1)?,
                created_at_local: row.get(2)?,
                text: row.get(3)?,
                duration_ms: row.get(4)?,
                lang: row.get(5)?,
                model: row.get(6)?,
            })
        })?;
        match rows.next() {
            Some(entry) => Ok(entry?),
            None => bail!("transcrição {id} não existe (veja: quickwhisper history list)"),
        }
    }

    /// Returns how many rows were actually deleted.
    pub fn delete(&self, ids: &[i64]) -> Result<usize> {
        let mut deleted = 0;
        for id in ids {
            deleted += self
                .conn
                .execute("DELETE FROM transcriptions WHERE id = ?1", [id])?;
        }
        Ok(deleted)
    }

    pub fn count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn clear(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM transcriptions", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_in_memory() -> History {
        History::from_conn(Connection::open_in_memory().expect("sqlite in memory"))
            .expect("schema")
    }

    #[test]
    fn insert_then_list_returns_newest_first() {
        let h = history_in_memory();
        h.insert("primeira", 1000, Some("pt"), "small").unwrap();
        h.insert("segunda", 2000, None, "small").unwrap();

        let entries = h.list(10).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            vec!["segunda", "primeira"]
        );
    }

    #[test]
    fn list_should_respect_limit() {
        let h = history_in_memory();
        for i in 0..5 {
            h.insert(&format!("t{i}"), 0, None, "base").unwrap();
        }
        assert_eq!(h.list(2).unwrap().len(), 2);
    }

    #[test]
    fn get_should_fail_for_missing_id() {
        let h = history_in_memory();
        assert!(h.get(42).is_err());
    }

    #[test]
    fn delete_returns_number_of_existing_rows_removed() {
        let h = history_in_memory();
        let id = h.insert("apagar", 0, None, "base").unwrap();
        assert_eq!(h.delete(&[id, 999]).unwrap(), 1);
        assert_eq!(h.count().unwrap(), 0);
    }

    #[test]
    fn clear_removes_everything() {
        let h = history_in_memory();
        h.insert("a", 0, None, "base").unwrap();
        h.insert("b", 0, None, "base").unwrap();
        assert_eq!(h.clear().unwrap(), 2);
        assert_eq!(h.count().unwrap(), 0);
    }

    #[test]
    fn created_at_is_stored_as_utc_iso8601() {
        let h = history_in_memory();
        let id = h.insert("x", 0, None, "base").unwrap();
        let entry = h.get(id).unwrap();
        assert!(
            entry.created_at.ends_with('Z') && entry.created_at.contains('T'),
            "esperava ISO 8601 UTC, veio: {}",
            entry.created_at
        );
    }
}

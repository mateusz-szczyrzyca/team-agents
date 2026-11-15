//! Snapshot statistics for direct readers.
use super::{ReadError, Reader, query};

impl Reader {
    /// Returns the logical database size, exact table counts and oldest recorded timestamp.
    ///
    /// # Errors
    /// [`ReadError::Query`] when SQLite cannot obtain the snapshot.
    pub fn statistics(&self) -> Result<serde_json::Value, ReadError> {
        let snapshot = self.snapshot("sqlite_master")?;
        let fail = query("sqlite_master");
        let pages: i64 = snapshot
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .map_err(&fail)?;
        let size: i64 = snapshot
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(&fail)?;
        let mut statement = snapshot
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .map_err(&fail)?;
        let names = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(&fail)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(&fail)?;
        let mut tables = serde_json::Map::new();
        let mut oldest: Option<String> = None;
        for name in names {
            // Names originate in the database, so quote identifiers even though values cannot name tables.
            let quoted = name.replace('"', "\"\"");
            let count: i64 = snapshot
                .query_row(&format!("SELECT count(*) FROM \"{quoted}\""), [], |r| r.get(0))
                .map_err(&fail)?;
            tables.insert(name.clone(), count.into());
            if matches!(name.as_str(), "history_records" | "daemon_logs") {
                let at: Option<String> = snapshot
                    .query_row(&format!("SELECT min(at) FROM \"{quoted}\""), [], |r| r.get(0))
                    .map_err(&fail)?;
                if let Some(at) = at
                    && oldest.as_ref().is_none_or(|previous| at < *previous)
                {
                    oldest = Some(at);
                }
            }
        }
        Ok(serde_json::json!({"bytes": pages.saturating_mul(size), "tables": tables, "oldest": oldest}))
    }
}

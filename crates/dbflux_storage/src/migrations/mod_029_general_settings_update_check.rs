//! Migration 029: Add `check_for_updates` and `skipped_update_version` columns
//! to `cfg_general_settings`.
//!
//! Persists whether the app asks the release repository for a newer version,
//! and which version the user dismissed, so both survive restarts. The check
//! defaults to on; the skipped version defaults to empty, meaning "nothing
//! dismissed".

use rusqlite::Transaction;

use crate::migrations::{Migration, MigrationError};

pub struct MigrationImpl;

impl Migration for MigrationImpl {
    fn name(&self) -> &str {
        "029_general_settings_update_check"
    }

    fn run(&self, tx: &Transaction) -> Result<(), MigrationError> {
        let table_exists: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='cfg_general_settings'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(sqlite_err)?;

        if !table_exists {
            return Ok(());
        }

        for (column, ddl) in [
            (
                "check_for_updates",
                "ALTER TABLE cfg_general_settings ADD COLUMN check_for_updates INTEGER NOT NULL DEFAULT 1;",
            ),
            (
                "skipped_update_version",
                "ALTER TABLE cfg_general_settings ADD COLUMN skipped_update_version TEXT NOT NULL DEFAULT '';",
            ),
        ] {
            let exists: bool = tx
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('cfg_general_settings') WHERE name = ?1",
                    [column],
                    |row| row.get::<_, i64>(0),
                )
                .map(|n| n > 0)
                .map_err(sqlite_err)?;

            if !exists {
                tx.execute_batch(ddl).map_err(sqlite_err)?;
            }
        }

        Ok(())
    }
}

fn sqlite_err(source: rusqlite::Error) -> MigrationError {
    MigrationError::Sqlite {
        path: std::path::PathBuf::from("<unknown>"),
        source,
    }
}

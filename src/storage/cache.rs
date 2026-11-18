//! Results of passed tests, which CI can save and restore.

use crate::{execution::CapturedOutput, identity::CacheKey};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const DATABASE_FILENAME: &str = "cache.sqlite3";
const MIGRATIONS: &[&str] = &[include_str!("../../migrations/cache/001.sql")];

pub struct Cache(Connection);

impl Cache {
    pub fn open(directory: &Path) -> Result<Self> {
        Ok(Self(super::open(directory, DATABASE_FILENAME, MIGRATIONS)?))
    }

    pub fn get_result(&self, key: &CacheKey) -> Result<Option<CapturedOutput>> {
        Ok(self
            .0
            .query_row(
                "SELECT stdout, stderr FROM passed_test_results WHERE cache_key = ?1",
                [&key.0[..]],
                |row| {
                    Ok(CapturedOutput {
                        stdout: row.get(0)?,
                        stderr: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn put_result(
        &self,
        key: &CacheKey,
        binary_id: &str,
        name: &str,
        output: &CapturedOutput,
    ) -> Result<()> {
        self.0.execute(
            "INSERT INTO passed_test_results (cache_key, binary_id, name, stdout, stderr)
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(cache_key) DO UPDATE SET
             stdout = excluded.stdout, stderr = excluded.stderr",
            params![&key.0[..], binary_id, name, output.stdout, output.stderr],
        )?;
        Ok(())
    }

    pub fn delete_result(&self, key: &CacheKey) -> Result<()> {
        self.0.execute(
            "DELETE FROM passed_test_results WHERE cache_key = ?1",
            [&key.0[..]],
        )?;
        Ok(())
    }
}

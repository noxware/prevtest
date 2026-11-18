//! Persistence layer with connection and migration handling.

mod cache;
mod state;

pub use cache::Cache;
pub use state::State;

use anyhow::{Result, ensure};
use rusqlite::{Connection, TransactionBehavior};
use std::{fs, path::Path, time::Duration};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

fn open(directory: &Path, filename: &str, migrations: &[&str]) -> Result<Connection> {
    fs::create_dir_all(directory)?;
    let mut db = Connection::open(directory.join(filename))?;
    db.busy_timeout(BUSY_TIMEOUT)?;
    migrate(&mut db, migrations)?;
    Ok(db)
}

fn migrate(db: &mut Connection, migrations: &[&str]) -> Result<()> {
    let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        version as usize <= migrations.len(),
        "unsupported schema version {version}"
    );
    if version as usize == migrations.len() {
        return Ok(());
    }

    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another wrapper may have migrated the database while we waited.
    let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        version as usize <= migrations.len(),
        "unsupported schema version {version}"
    );
    for (index, sql) in migrations.iter().enumerate().skip(version as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i32::try_from(index + 1)?)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATIONS: &[&str] = &["CREATE TABLE probe (value INTEGER);"];

    #[test]
    fn rejects_corrupt_and_newer_databases_without_modifying_them() {
        let directory = tempfile::tempdir().unwrap();
        let open = || open(directory.path(), "test.sqlite3", MIGRATIONS);
        let path = directory.path().join("test.sqlite3");
        fs::write(&path, "not sqlite").unwrap();
        assert!(open().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not sqlite");

        fs::remove_file(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        let newer = MIGRATIONS.len() as u32 + 1;
        db.pragma_update(None, "user_version", newer).unwrap();
        assert!(open().is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            newer
        );
    }

    #[test]
    fn migrations_apply_only_pending_steps_and_roll_back_on_failure() {
        let mut db = Connection::open_in_memory().unwrap();
        let migrations = [
            "CREATE TABLE probe (value INTEGER); INSERT INTO probe VALUES (7);",
            "ALTER TABLE probe ADD COLUMN extra TEXT;",
        ];
        migrate(&mut db, &migrations[..1]).unwrap();
        migrate(&mut db, &migrations).unwrap();
        migrate(&mut db, &migrations).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("SELECT value FROM probe", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            7
        );

        let failing = [
            migrations[0],
            migrations[1],
            "UPDATE probe SET value = 8;",
            "INVALID SQL",
        ];
        assert!(migrate(&mut db, &failing).is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("SELECT value FROM probe", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            7
        );
    }
}

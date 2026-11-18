//! State that only makes sense on this machine, kept apart from the cache that
//! CI saves and restores.
//!
//! Each wrapper run is a separate, stateless process, so runs use this to share
//! what they learn, such as file hashes.

use crate::hashing::Digest;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{self, OpenOptions},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

const DATABASE_FILENAME: &str = "state.sqlite3";
const CLOCK_FILENAME: &str = "clock";
const MIGRATIONS: &[&str] = &[include_str!("../../migrations/state/001.sql")];

pub struct State {
    db: Connection,
    clock: PathBuf,
}

impl State {
    pub fn open(directory: &Path) -> Result<Self> {
        Ok(Self {
            db: super::open(directory, DATABASE_FILENAME, MIGRATIONS)?,
            clock: directory.join(CLOCK_FILENAME),
        })
    }

    /// Writes to the clock file, so its timestamps show the current time of the
    /// filesystem holding the state, and returns its metadata. Concurrent writers
    /// only move that time forward. The clock must be a regular file.
    pub fn touch_clock(&self) -> Result<fs::Metadata> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.clock)?;
        ensure!(
            file.metadata()?.is_file(),
            "{} is not a regular file",
            self.clock.display()
        );
        file.write_all_at(&[0], 0)?;
        Ok(file.metadata()?)
    }

    /// Returns the hash saved for this file, if it was saved with the same stamp.
    pub fn get_file_hash(&self, path: &Path, stamp: &Digest) -> Result<Option<Digest>> {
        Ok(self
            .db
            .query_row(
                "SELECT hash FROM file_hashes WHERE path = ?1 AND stamp = ?2",
                params![path.as_os_str().as_bytes(), stamp.as_bytes()],
                |row| Ok(Digest::from_bytes(row.get(0)?)),
            )
            .optional()?)
    }

    /// Replaces any hash saved for this file.
    pub fn put_file_hash(&self, path: &Path, stamp: &Digest, hash: &Digest) -> Result<()> {
        self.db.execute(
            "INSERT INTO file_hashes (path, stamp, hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET stamp = excluded.stamp, hash = excluded.hash",
            params![
                path.as_os_str().as_bytes(),
                stamp.as_bytes(),
                hash.as_bytes()
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashing::hash;
    use std::{ffi::CString, sync::mpsc, thread, time::Duration};

    #[test]
    fn keeps_one_hash_per_file_matching_its_stamp() {
        let directory = tempfile::tempdir().unwrap();
        let state = State::open(directory.path()).unwrap();
        let path = Path::new("/workspace/binary");
        let (first, second) = (Digest::from_bytes([1; 32]), Digest::from_bytes([2; 32]));
        assert_eq!(state.get_file_hash(path, &first).unwrap(), None);

        state.put_file_hash(path, &first, &hash(b"one")).unwrap();
        assert_eq!(
            state.get_file_hash(path, &first).unwrap(),
            Some(hash(b"one"))
        );
        assert_eq!(state.get_file_hash(path, &second).unwrap(), None);

        state.put_file_hash(path, &second, &hash(b"two")).unwrap();
        assert_eq!(state.get_file_hash(path, &first).unwrap(), None);
        assert_eq!(
            state.get_file_hash(path, &second).unwrap(),
            Some(hash(b"two"))
        );
    }

    #[test]
    fn clock_does_not_write_through_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let state = State::open(directory.path()).unwrap();
        let target = directory.path().join("target");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, directory.path().join(CLOCK_FILENAME)).unwrap();

        assert!(state.touch_clock().is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn clock_does_not_wait_on_fifos() {
        let directory = tempfile::tempdir().unwrap();
        let clock = directory.path().join(CLOCK_FILENAME);
        let path = CString::new(clock.as_os_str().as_bytes()).unwrap();
        // SAFETY: `path` is a valid NUL-terminated string that outlives the call,
        // and `mkfifo` doesn't keep the pointer.
        let result = unsafe { libc::mkfifo(path.as_ptr(), 0o644) };
        assert_eq!(result, 0);

        // Fail instead of hanging the test run if opening the FIFO blocks.
        let (sender, receiver) = mpsc::channel();
        let state_dir = directory.path().to_owned();
        thread::spawn(move || {
            let state = State::open(&state_dir).unwrap();
            sender.send(state.touch_clock().is_err()).unwrap();
        });
        assert!(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
    }
}

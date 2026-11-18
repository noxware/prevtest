//! Identifies test inputs: content hashes of files and cache keys.

use crate::{
    hashing::{self, Digest, Hasher},
    storage::State,
};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    fs::{File, Metadata},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
};

/// Returns the hash of a file's contents. With local state, a hash saved for the
/// same file metadata is reused. A new hash is saved only if the file last changed
/// before it was read, according to the filesystem's own clock, so a later write
/// within the same timestamp tick can't leave the metadata unchanged. Problems
/// with the local state only cost a rehash.
pub fn file_hash(path: &Path, state: Option<&State>) -> Result<Digest> {
    let metadata = FileMetadata::read(path)?;
    let Some(state) = state else {
        return Ok(hashing::hash_reader(File::open(path)?)?);
    };
    let stamp = metadata.to_stamp();
    match state.get_file_hash(path, &stamp) {
        Ok(Some(hash)) => return Ok(hash),
        Ok(None) => {}
        Err(error) => on_state_error(path, &error),
    }
    // Read the clock before the contents. A write after the read gets a timestamp
    // no earlier than the clock, while saving requires a change strictly before
    // it, so the saved stamp can't match the rewritten file.
    let changed_before_read = match state.touch_clock() {
        Ok(clock) => metadata.changed_before(&FileMetadata::from(&clock)),
        Err(error) => {
            on_state_error(path, &error);
            false
        }
    };
    let hash = hashing::hash_reader(File::open(path)?)?;
    if changed_before_read {
        state
            .put_file_hash(path, &stamp, &hash)
            .unwrap_or_else(|error| on_state_error(path, &error));
    }
    Ok(hash)
}

fn on_state_error(path: &Path, error: &anyhow::Error) {
    eprintln!(
        "prevtest: file hash cache failed for {}: {error:#}",
        path.display()
    );
}

/// Metadata that changes when a file is rewritten or replaced, but not when it
/// is read.
struct FileMetadata {
    dev: u64,
    ino: u64,
    size: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    /// Seconds and nanoseconds since the Unix epoch.
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl FileMetadata {
    fn read(path: &Path) -> Result<Self> {
        let metadata = path
            .metadata()
            .with_context(|| format!("reading {}", path.display()))?;
        ensure!(
            metadata.is_file(),
            "{} is not a regular file",
            path.display()
        );
        Ok(Self::from(&metadata))
    }

    /// Whether this file last changed before `other` was last modified. Timestamps
    /// from different filesystems may come from different clocks or granularities,
    /// so they never compare as earlier.
    fn changed_before(&self, other: &FileMetadata) -> bool {
        self.dev == other.dev && self.mtime.max(self.ctime) < other.mtime
    }

    /// Digest of the metadata, telling whether a hash saved for the file still
    /// applies.
    fn to_stamp(&self) -> Digest {
        let mut hasher = Hasher::new();
        for value in [self.dev, self.ino, self.size] {
            hasher.update(&value.to_le_bytes());
        }
        for value in [self.mode, self.uid, self.gid] {
            hasher.update(&value.to_le_bytes());
        }
        for value in [self.mtime.0, self.mtime.1, self.ctime.0, self.ctime.1] {
            hasher.update(&value.to_le_bytes());
        }
        hasher.finalize()
    }
}

impl From<&Metadata> for FileMetadata {
    fn from(metadata: &Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            size: metadata.size(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mtime: (metadata.mtime(), metadata.mtime_nsec()),
            ctime: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CacheKey(pub [u8; 32]);

impl CacheKey {
    pub fn new(
        binary_hash: &Digest,
        binary_id: &str,
        test_name: &str,
        working_dir: &Path,
        workspace: &Path,
        args: &[OsString],
    ) -> Self {
        let mut hasher = Hasher::new();
        let mut field = |bytes: &[u8]| {
            hasher
                .update(&(bytes.len() as u64).to_le_bytes())
                .update(bytes);
        };
        field(&1_u16.to_le_bytes());
        field(binary_hash.as_bytes());
        field(binary_id.as_bytes());
        field(test_name.as_bytes());
        let cwd = working_dir.strip_prefix(workspace).unwrap_or(working_dir);
        field(cwd.as_os_str().as_bytes());
        for arg in args {
            field(arg.as_bytes());
        }
        Self(*hasher.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashing::hash;
    use std::{fs, thread, time::Duration};

    fn stamp(path: &Path) -> Digest {
        FileMetadata::read(path).unwrap().to_stamp()
    }

    fn metadata(dev: u64, mtime: (i64, i64), ctime: (i64, i64)) -> FileMetadata {
        FileMetadata {
            dev,
            ino: 1,
            size: 6,
            mode: 0o100644,
            uid: 501,
            gid: 20,
            mtime,
            ctime,
        }
    }

    /// Waits until the state's clock passes the file's last change, which takes
    /// up to one timestamp tick of the filesystem.
    fn wait_for_clock(path: &Path, state: &State) {
        for _ in 0..5000 {
            let clock = FileMetadata::from(&state.touch_clock().unwrap());
            if FileMetadata::read(path).unwrap().changed_before(&clock) {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("the filesystem clock did not advance");
    }

    #[test]
    fn stamp_ignores_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"contents").unwrap();
        let before = stamp(&path);
        hashing::hash_reader(File::open(&path).unwrap()).unwrap();
        assert_eq!(stamp(&path), before);
    }

    #[test]
    fn stamp_changes_with_ctime_alone() {
        let original = metadata(1, (100, 5), (100, 5));
        let rewritten = metadata(1, (100, 5), (100, 6));
        assert_ne!(original.to_stamp(), rewritten.to_stamp());
    }

    #[test]
    fn changes_compare_with_a_clock_on_the_same_filesystem_only() {
        let clock = metadata(1, (100, 5), (100, 5));
        assert!(metadata(1, (100, 4), (100, 4)).changed_before(&clock));
        // Within the same tick, a later write could keep the same timestamps.
        assert!(!metadata(1, (100, 5), (100, 5)).changed_before(&clock));
        assert!(!metadata(1, (100, 4), (100, 5)).changed_before(&clock));
        assert!(!metadata(1, (101, 0), (101, 0)).changed_before(&clock));
        assert!(!metadata(2, (100, 4), (100, 4)).changed_before(&clock));
    }

    #[test]
    fn reuses_saved_file_hash_only_while_its_stamp_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"contents").unwrap();
        let state = State::open(&dir.path().join("state")).unwrap();

        // A saved hash that differs from the contents shows it was reused.
        let saved = hash(b"saved");
        state.put_file_hash(&path, &stamp(&path), &saved).unwrap();
        assert_eq!(file_hash(&path, Some(&state)).unwrap(), saved);

        let other_stamp = Digest::from_bytes([0; 32]);
        state.put_file_hash(&path, &other_stamp, &saved).unwrap();
        assert_eq!(file_hash(&path, Some(&state)).unwrap(), hash(b"contents"));
    }

    #[test]
    fn saves_file_hash_once_the_clock_passes_the_last_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"contents").unwrap();
        let state = State::open(&dir.path().join("state")).unwrap();

        wait_for_clock(&path, &state);
        file_hash(&path, Some(&state)).unwrap();
        assert_eq!(
            state.get_file_hash(&path, &stamp(&path)).unwrap(),
            Some(hash(b"contents"))
        );
    }

    #[test]
    fn hashes_without_saving_when_the_clock_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"contents").unwrap();
        let state_dir = dir.path().join("state");
        let state = State::open(&state_dir).unwrap();
        fs::create_dir(state_dir.join("clock")).unwrap();

        assert_eq!(file_hash(&path, Some(&state)).unwrap(), hash(b"contents"));
        assert_eq!(state.get_file_hash(&path, &stamp(&path)).unwrap(), None);
    }

    #[test]
    fn hashes_files_without_local_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"contents").unwrap();
        assert_eq!(file_hash(&path, None).unwrap(), hash(b"contents"));
    }

    #[test]
    fn key_distinguishes_binary_content_and_test_context() {
        let key = |contents: &[u8], id, name, cwd| {
            CacheKey::new(
                &hash(contents),
                id,
                name,
                Path::new(cwd),
                Path::new("/workspace"),
                &[],
            )
        };
        let original = key(b"before", "binary", "first", "/workspace");
        assert_ne!(original, key(b"before", "other", "first", "/workspace"));
        assert_ne!(original, key(b"before", "binary", "second", "/workspace"));
        assert_ne!(
            original,
            key(b"before", "binary", "first", "/workspace/package")
        );
        assert_ne!(original, key(b"after", "binary", "first", "/workspace"));
    }

    #[test]
    fn argument_boundaries_and_order_affect_the_key() {
        let binary_hash = hash(b"contents");
        let key = |args: &[&str]| {
            let args: Vec<_> = args.iter().map(OsString::from).collect();
            CacheKey::new(
                &binary_hash,
                "binary",
                "test",
                Path::new("/workspace"),
                Path::new("/workspace"),
                &args,
            )
        };
        assert_ne!(key(&["ab", "c"]), key(&["a", "bc"]));
        assert_ne!(key(&["a", "b"]), key(&["b", "a"]));
        assert_ne!(key(&[]), key(&[""]));
    }
}

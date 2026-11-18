//! Content hashing, so the rest of the crate doesn't depend on the hash function.

use std::io::{self, Read};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub struct Hasher(blake3::Hasher);

impl Hasher {
    pub fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    pub fn update(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(bytes);
        self
    }

    pub fn update_reader(&mut self, reader: impl Read) -> io::Result<&mut Self> {
        self.0.update_reader(reader)?;
        Ok(self)
    }

    pub fn finalize(&self) -> Digest {
        Digest(*self.0.finalize().as_bytes())
    }
}

pub fn hash_reader(reader: impl Read) -> io::Result<Digest> {
    Ok(Hasher::new().update_reader(reader)?.finalize())
}

#[cfg(test)]
pub fn hash(bytes: &[u8]) -> Digest {
    Hasher::new().update(bytes).finalize()
}

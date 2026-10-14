//! Checksummed data files: written and read in chunks, each hashed once.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use univo_core::{Error, Result};
use univo_embed::Digest;

use super::files::{private_file, regular_file};
use super::{invalid, io};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Blob {
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Digest::Sha256(Sha256::digest(bytes).into()).to_hex()
}
pub(super) fn valid_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Bytes hashed and written at once: records and vectors are written a few
/// bytes at a time, and hashing them that way costs more than the I/O.
pub(super) const CHUNK: usize = 1 << 16;

pub(super) struct CheckedWriter {
    pub(super) output: File,
    pub(super) buffer: Vec<u8>,
    pub(super) hash: Sha256,
    pub(super) bytes: u64,
    pub(super) limit: u64,
    pub(super) exhausted: bool,
}
impl Write for CheckedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.limit.saturating_sub(self.bytes) {
            self.exhausted = true;
            return Err(std::io::Error::other("snapshot byte budget exhausted"));
        }
        if self.buffer.len() + bytes.len() > CHUNK {
            self.write_buffer()?;
        }
        if bytes.len() >= CHUNK {
            self.hash.update(bytes);
            self.output.write_all(bytes)?;
        } else {
            self.buffer.extend_from_slice(bytes);
        }
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.write_buffer()?;
        self.output.flush()
    }
}
impl CheckedWriter {
    pub(super) fn write_buffer(&mut self) -> std::io::Result<()> {
        self.hash.update(&self.buffer);
        self.output.write_all(&self.buffer)?;
        self.buffer.clear();
        Ok(())
    }
}
pub(super) fn write_blob(
    path: &Path,
    limit: u64,
    body: impl FnOnce(&mut CheckedWriter) -> Result<()>,
) -> Result<Blob> {
    let mut writer = CheckedWriter {
        output: private_file(path, true)?,
        buffer: Vec::with_capacity(CHUNK),
        hash: Sha256::new(),
        bytes: 0,
        limit,
        exhausted: false,
    };
    if let Err(error) = body(&mut writer) {
        return Err(if writer.exhausted {
            Error::snapshot_limit()
        } else {
            error
        });
    }
    writer.flush().map_err(io)?;
    writer.output.sync_all().map_err(io)?;
    Ok(Blob {
        bytes: writer.bytes,
        sha256: Digest::Sha256(writer.hash.finalize().into()).to_hex(),
    })
}

/// Reads a data file in chunks, each hashed as a whole when it is read.
pub(super) struct CheckedReader {
    pub(super) input: File,
    pub(super) buffer: Box<[u8]>,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) hash: Sha256,
    /// Bytes of the file not yet read into the buffer.
    pub(super) unread: u64,
}
impl Read for CheckedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.start == self.end {
            let wanted = self
                .buffer
                .len()
                .min(usize::try_from(self.unread).unwrap_or(usize::MAX));
            let n = self.input.read(&mut self.buffer[..wanted])?;
            self.hash.update(&self.buffer[..n]);
            self.unread -= n as u64;
            (self.start, self.end) = (0, n);
        }
        let n = bytes.len().min(self.end - self.start);
        bytes[..n].copy_from_slice(&self.buffer[self.start..self.start + n]);
        self.start += n;
        Ok(n)
    }
}
impl CheckedReader {
    pub(super) fn new(file: File, blob: &Blob) -> Result<Self> {
        if file.metadata().map_err(io)?.len() != blob.bytes || !valid_digest(&blob.sha256) {
            return Err(invalid("file size or checksum declaration"));
        }
        Ok(Self {
            input: file,
            buffer: vec![0; CHUNK].into_boxed_slice(),
            start: 0,
            end: 0,
            hash: Sha256::new(),
            unread: blob.bytes,
        })
    }
    /// Bytes of the file not consumed yet, buffered or not.
    pub(super) fn remaining(&self) -> u64 {
        self.unread + (self.end - self.start) as u64
    }
    pub(super) fn read_fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_exact(&mut bytes)
            .map_err(|_| invalid("truncated data file"))?;
        Ok(bytes)
    }
    pub(super) fn count(&mut self) -> Result<usize> {
        usize::try_from(u64::from_le_bytes(self.read_fixed()?))
            .map_err(|_| invalid("count overflow"))
    }
    pub(super) fn finish(self, blob: &Blob) -> Result<()> {
        if self.remaining() != 0
            || Digest::Sha256(self.hash.finalize().into()).to_hex() != blob.sha256
        {
            return Err(invalid("data checksum or trailing bytes"));
        }
        Ok(())
    }
}
pub(super) fn count(out: &mut impl Write, n: usize) -> Result<()> {
    out.write_all(&(n as u64).to_le_bytes()).map_err(io)
}

pub(super) fn verify_blob(path: &Path, blob: &Blob) -> Result<()> {
    let mut reader = CheckedReader::new(regular_file(path)?, blob)?;
    let mut buffer = [0; 16384];
    while reader.read(&mut buffer).map_err(io)? != 0 {}
    reader.finish(blob)
}

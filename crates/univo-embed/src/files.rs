//! Model files and their verification.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use sha1::Digest as _;
use univo_core::{Error, Result};

/// Files of a model in the `Model2Vec` format, the only format the shipped
/// backend runs. Nothing else is downloaded, and no code is ever executed.
pub const MODEL_FILES: [&str; 3] = ["config.json", "model.safetensors", "tokenizer.json"];

/// Expected content of a file, as published by the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Digest {
    /// SHA-256 of the content: pinned files and Git LFS objects.
    Sha256([u8; 32]),
    /// Git blob SHA-1, `sha1("blob <size>\0" + content)`: small files that
    /// the Hugging Face hub stores in Git rather than LFS.
    GitBlobSha1([u8; 20]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileExpectation {
    pub name: String,
    pub size: u64,
    pub digest: Digest,
}

/// Decodes a lowercase or uppercase hexadecimal digest of `N` bytes.
pub(crate) fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * index..2 * index + 2], 16).ok()?;
    }
    Some(out)
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

impl Digest {
    pub fn to_hex(&self) -> String {
        match self {
            Self::Sha256(bytes) => encode_hex(bytes),
            Self::GitBlobSha1(bytes) => encode_hex(bytes),
        }
    }
}

impl Digest {
    /// Digest of `content` computed the same way as `self`.
    fn of_content_like(&self, content: &[u8]) -> Self {
        match self {
            Self::Sha256(_) => Self::Sha256(sha2::Sha256::digest(content).into()),
            Self::GitBlobSha1(_) => {
                let mut hasher = sha1::Sha1::new();
                hasher.update(format!("blob {}\0", content.len()).as_bytes());
                hasher.update(content);
                Self::GitBlobSha1(hasher.finalize().into())
            }
        }
    }
}

/// SHA-256 of `content`, the identity of a file read from a local model.
pub(crate) fn sha256(content: &[u8]) -> Digest {
    Digest::Sha256(sha2::Sha256::digest(content).into())
}

/// Checks the size and digest of content already in memory, so that the
/// bytes verified are exactly the bytes used.
///
/// # Errors
///
/// Returns `model.integrity` when the content differs in size or digest.
pub fn verify_content(model: &str, content: &[u8], expected: &FileExpectation) -> Result<()> {
    let integrity = |reason: String| Error::model_integrity(model, &expected.name, &reason);
    let size = content.len() as u64;
    if size != expected.size {
        return Err(integrity(format!(
            "size is {size} bytes, expected {}",
            expected.size
        )));
    }
    let actual = expected.digest.of_content_like(content);
    if actual != expected.digest {
        return Err(integrity(format!(
            "digest is {}, expected {}",
            actual.to_hex(),
            expected.digest.to_hex()
        )));
    }
    Ok(())
}

/// Checks a file's size and digest, reading it once in fixed-size chunks.
///
/// # Errors
///
/// Returns `model.integrity` when the file is missing, unreadable, or
/// differs in size or digest.
pub fn verify_file(model: &str, path: &Path, expected: &FileExpectation) -> Result<()> {
    let integrity = |reason: String| Error::model_integrity(model, &expected.name, &reason);
    let file = File::open(path).map_err(|error| integrity(format!("cannot open: {error}")))?;
    let size = file
        .metadata()
        .map_err(|error| integrity(format!("cannot read metadata: {error}")))?
        .len();
    if size != expected.size {
        return Err(integrity(format!(
            "size is {size} bytes, expected {}",
            expected.size
        )));
    }

    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut buffer = vec![0u8; 1 << 20];
    let actual = match expected.digest {
        Digest::Sha256(_) => {
            let mut hasher = sha2::Sha256::new();
            hash_reader(&mut reader, &mut buffer, |chunk| hasher.update(chunk))
                .map_err(|error| integrity(format!("cannot read: {error}")))?;
            Digest::Sha256(hasher.finalize().into())
        }
        Digest::GitBlobSha1(_) => {
            let mut hasher = sha1::Sha1::new();
            hasher.update(format!("blob {size}\0").as_bytes());
            hash_reader(&mut reader, &mut buffer, |chunk| hasher.update(chunk))
                .map_err(|error| integrity(format!("cannot read: {error}")))?;
            Digest::GitBlobSha1(hasher.finalize().into())
        }
    };
    if actual != expected.digest {
        return Err(integrity(format!(
            "digest is {}, expected {}",
            actual.to_hex(),
            expected.digest.to_hex()
        )));
    }
    Ok(())
}

fn hash_reader(
    reader: &mut impl Read,
    buffer: &mut [u8],
    mut update: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    loop {
        let read = reader.read(buffer)?;
        if read == 0 {
            return Ok(());
        }
        update(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_and_file_digests_agree() {
        let content = b"{\"model_type\": \"model2vec\"}\n";
        let path = std::env::temp_dir().join(format!("univo-digest-{}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        for digest in [Digest::Sha256([0; 32]), Digest::GitBlobSha1([0; 20])] {
            let expected = FileExpectation {
                name: "config.json".into(),
                size: content.len() as u64,
                digest: digest.of_content_like(content),
            };
            verify_content("m", content, &expected).unwrap();
            verify_file("m", &path, &expected).unwrap();
        }
        let _ = std::fs::remove_file(path);
        // `git hash-object` of an empty file.
        assert_eq!(
            Digest::GitBlobSha1([0; 20]).of_content_like(b"").to_hex(),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    #[test]
    fn hex_round_trips() {
        let bytes = decode_hex::<4>("00ff7Fa0").unwrap();
        assert_eq!(bytes, [0x00, 0xff, 0x7f, 0xa0]);
        assert_eq!(encode_hex(&bytes), "00ff7fa0");
        assert_eq!(decode_hex::<4>("00ff7f"), None);
        assert_eq!(decode_hex::<2>("zzzz"), None);
    }
}

//! Filesystem primitives: private files and directories, durable syncs and
//! bounded reads of regular files only.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use univo_core::Result;

use super::{invalid, io};

pub(super) fn parent_directory(path: &Path) -> &Path {
    let parent = path.parent().unwrap_or(path);
    if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    }
}

pub(super) fn private_file(path: &Path, exclusive: bool) -> Result<File> {
    let mut options = File::options();
    options.write(true).read(true);
    if exclusive {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(io)
}
pub(super) fn private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(io)
}
pub(super) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(io)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
pub(super) fn small_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = regular_file(path)?;
    if file.metadata().map_err(io)?.len() > limit {
        return Err(invalid("metadata is too large"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("metadata is too large"));
    }
    Ok(bytes)
}
pub(super) fn regular_file(path: &Path) -> Result<File> {
    if !fs::symlink_metadata(path)
        .map_err(io)?
        .file_type()
        .is_file()
    {
        return Err(invalid("expected a regular snapshot file"));
    }
    File::open(path).map_err(io)
}
pub(super) fn write_small(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = private_file(path, true)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}

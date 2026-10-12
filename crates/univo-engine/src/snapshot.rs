//! Immutable, bounded local snapshots with a single atomic publication point.
use std::fs::{self, File, TryLockError};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use univo_core::{Error, NORMALIZER_VERSION, Result, SCORING_VERSION};
use univo_embed::{Digest, ModelDescriptor, ModelSource};

use crate::vectors::{FieldVectors, RecordVectors};
use crate::{SEARCH_VERSION, SearchIndex};

const FORMAT: u32 = 1;
const METADATA_LIMIT: u64 = 1 << 20;
const MAX_DIMENSION: usize = 65_536;
const RECORD_MAGIC: &[u8; 8] = b"UNIVOR01";
const VECTOR_MAGIC: &[u8; 8] = b"UNIVOV01";
static SERIAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Blob {
    bytes: u64,
    sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    generation: u64,
    normalizer: u32,
    scoring: u32,
    search: u32,
    schema_fingerprint: u64,
    schema: Value,
    records: usize,
    dimension: usize,
    model: Option<Value>,
    configuration: Value,
    record_file: Blob,
    vector_file: Blob,
}

/// A stable generation, opened once. Later saves never change its files.
#[derive(Debug)]
pub struct Snapshot {
    manifest: Manifest,
    records: File,
    vectors: File,
}

#[allow(clippy::needless_pass_by_value)] // Used directly by Result::map_err.
fn io(error: std::io::Error) -> Error {
    Error::snapshot_io(&error)
}
fn invalid(reason: &str) -> Error {
    Error::snapshot_invalid(reason)
}
fn digest(bytes: &[u8]) -> String {
    Digest::Sha256(Sha256::digest(bytes).into()).to_hex()
}
fn valid_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn check_shape(records: usize, dimension: usize, has_model: bool) -> Result<()> {
    if records > u32::MAX as usize || dimension > MAX_DIMENSION || has_model != (dimension > 0) {
        return Err(invalid("record count or dimension"));
    }
    Ok(())
}

fn parent_directory(path: &Path) -> &Path {
    let parent = path.parent().unwrap_or(path);
    if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    }
}

fn schema_identity(schema: &univo_core::Schema) -> Value {
    let fields: Vec<_> = schema.fields().iter().map(|field| {
        let options = match field.kind {
            univo_core::FieldKind::Text(text) => json!({"text": {"mode": format!("{:?}", text.mode), "weight": text.weight, "required": text.required, "case_fold": text.case_fold}}),
            univo_core::FieldKind::Exact(exact) => json!({"exact": {"normalizer": format!("{:?}", exact.normalizer), "weight": exact.weight, "required": exact.required, "conflict": format!("{:?}", exact.conflict)}}),
        };
        json!({"path": field.path.keys(), "options": options})
    }).collect();
    json!({"fields": fields, "id": schema.id_field().map(univo_core::FieldPath::keys), "partitions": schema.partition_by().iter().map(univo_core::FieldPath::keys).collect::<Vec<_>>(),
        "limits": [schema.limits().max_fields, schema.limits().max_depth, schema.limits().max_selected_bytes]})
}

fn check_metadata(index: &SearchIndex) -> Result<()> {
    let schema = index.schema();
    let mut bytes = schema.fields().len().saturating_mul(128);
    for path in schema
        .id_field()
        .into_iter()
        .chain(schema.partition_by())
        .chain(schema.fields().iter().map(|field| &field.path))
    {
        for key in path.keys() {
            bytes = bytes.saturating_add(key.len()).saturating_add(16);
        }
    }
    if let Some(vectors) = &index.vectors {
        let model = vectors.model();
        let (source_bytes, files) = match &model.source {
            ModelSource::Hub {
                repository,
                revision,
                files,
            } => (
                repository
                    .organization
                    .len()
                    .saturating_add(repository.name.len())
                    .saturating_add(1)
                    .saturating_add(revision.len()),
                files,
            ),
            ModelSource::Local { path, files } => {
                (path.as_os_str().as_encoded_bytes().len(), files)
            }
        };
        bytes = bytes
            .saturating_add(source_bytes)
            .saturating_add(model.backend.len());
        for file in files {
            bytes = bytes.saturating_add(file.name.len()).saturating_add(128);
        }
    }
    if bytes as u64 > METADATA_LIMIT {
        return Err(invalid(
            "metadata is too large; use shorter field paths or model identities",
        ));
    }
    Ok(())
}

fn model_identity(model: &ModelDescriptor) -> Value {
    let (source, files) = match &model.source {
        ModelSource::Hub {
            repository,
            revision,
            files,
        } => (
            json!({"hub": repository.to_string(), "revision": revision}),
            files,
        ),
        ModelSource::Local { path, files } => (json!({"local": path}), files),
    };
    let files: Vec<_> = files.iter().map(|file| json!({
        "name": file.name, "bytes": file.size, "digest": file.digest.to_hex(),
        "algorithm": match file.digest { Digest::Sha256(_) => "sha256", Digest::GitBlobSha1(_) => "git-blob-sha1" },
    })).collect();
    json!({"source": source, "files": files, "dimension": model.dimension, "backend": model.backend})
}

fn private_file(path: &Path, exclusive: bool) -> Result<File> {
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
fn private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(io)
}
fn sync_directory(path: &Path) -> Result<()> {
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
fn small_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
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
fn regular_file(path: &Path) -> Result<File> {
    if !fs::symlink_metadata(path)
        .map_err(io)?
        .file_type()
        .is_file()
    {
        return Err(invalid("expected a regular snapshot file"));
    }
    File::open(path).map_err(io)
}
fn write_small(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = private_file(path, true)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}

struct CheckedWriter {
    output: BufWriter<File>,
    hash: Sha256,
    bytes: u64,
    limit: u64,
    exhausted: bool,
}
impl Write for CheckedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.limit.saturating_sub(self.bytes) {
            self.exhausted = true;
            return Err(std::io::Error::other("snapshot byte budget exhausted"));
        }
        let n = self.output.write(bytes)?;
        self.hash.update(&bytes[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
fn write_blob(
    path: &Path,
    limit: u64,
    body: impl FnOnce(&mut CheckedWriter) -> Result<()>,
) -> Result<Blob> {
    let mut writer = CheckedWriter {
        output: BufWriter::new(private_file(path, true)?),
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
    writer.output.get_ref().sync_all().map_err(io)?;
    Ok(Blob {
        bytes: writer.bytes,
        sha256: Digest::Sha256(writer.hash.finalize().into()).to_hex(),
    })
}

struct CheckedReader {
    input: BufReader<File>,
    hash: Sha256,
    left: u64,
}
impl Read for CheckedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = bytes
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.input.read(&mut bytes[..n])?;
        self.hash.update(&bytes[..n]);
        self.left -= n as u64;
        Ok(n)
    }
}
impl CheckedReader {
    fn new(file: File, blob: &Blob) -> Result<Self> {
        if file.metadata().map_err(io)?.len() != blob.bytes || !valid_digest(&blob.sha256) {
            return Err(invalid("file size or checksum declaration"));
        }
        Ok(Self {
            input: BufReader::new(file),
            hash: Sha256::new(),
            left: blob.bytes,
        })
    }
    fn read_fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_exact(&mut bytes)
            .map_err(|_| invalid("truncated data file"))?;
        Ok(bytes)
    }
    fn count(&mut self) -> Result<usize> {
        usize::try_from(u64::from_le_bytes(self.read_fixed()?))
            .map_err(|_| invalid("count overflow"))
    }
    fn finish(self, blob: &Blob) -> Result<()> {
        if self.left != 0 || Digest::Sha256(self.hash.finalize().into()).to_hex() != blob.sha256 {
            return Err(invalid("data checksum or trailing bytes"));
        }
        Ok(())
    }
}
fn count(out: &mut impl Write, n: usize) -> Result<()> {
    out.write_all(&(n as u64).to_le_bytes()).map_err(io)
}

impl SearchIndex {
    /// Publishes an explicit snapshot, refusing a concurrent writer.
    /// `configuration` is binding metadata, capped at 1 MiB; no code is executed.
    /// # Errors
    /// Returns typed I/O, lock, budget or compatibility errors. An error after
    /// publication may still leave the complete new generation current.
    pub fn save(
        &self,
        path: &Path,
        generation: u64,
        configuration: Value,
        max_bytes: u64,
    ) -> Result<()> {
        self.save_with(path, generation, configuration, max_bytes, &mut |_| Ok(()))
    }
    fn save_with(
        &self,
        path: &Path,
        generation: u64,
        configuration: Value,
        max_bytes: u64,
        checkpoint: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        check_shape(
            self.len(),
            self.vectors.as_ref().map_or(0, |v| v.model().dimension),
            self.vectors.is_some(),
        )?;
        check_metadata(self)?;
        if max_bytes < 16 {
            return Err(Error::snapshot_limit());
        }
        if !path.exists() {
            private_directory(path)?;
        }
        let lock = private_file(&path.join("WRITER.lock"), false)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(Error::snapshot_locked()),
            Err(TryLockError::Error(error)) => return Err(io(error)),
        }
        checkpoint("locked")?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("system clock"))?
            .as_nanos();
        let name = format!(
            "g-{stamp:x}-{:x}-{:x}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let directory = path.join(&name);
        private_directory(&directory)?;
        let pointer = path.join(format!("CURRENT-{name}.tmp"));
        let mut published = false;
        let result = (|| {
            checkpoint("directory")?;
            let checksum = write_generation(
                self,
                &directory,
                generation,
                configuration,
                max_bytes,
                checkpoint,
            )?;
            sync_directory(&directory)?;
            checkpoint("generation_synced")?;
            write_small(&pointer, format!("{name}\n{checksum}\n").as_bytes())?;
            checkpoint("pointer")?;
            fs::rename(&pointer, path.join("CURRENT")).map_err(io)?;
            published = true;
            checkpoint("published")?;
            sync_directory(path)?;
            checkpoint("root_synced")?;
            sync_directory(parent_directory(path))?;
            checkpoint("parent_synced")?;
            Ok(())
        })();
        if !published {
            let _ = fs::remove_dir_all(directory);
        }
        let _ = fs::remove_file(pointer);
        result
    }
}
fn write_generation(
    index: &SearchIndex,
    directory: &Path,
    generation: u64,
    configuration: Value,
    max_bytes: u64,
    checkpoint: &mut impl FnMut(&str) -> Result<()>,
) -> Result<String> {
    let record_file = write_blob(&directory.join("records.bin"), max_bytes, |out| {
        out.write_all(RECORD_MAGIC).map_err(io)?;
        count(out, index.len())?;
        for record in index.store().records() {
            univo_core::snapshot::write_record(out, record)?;
        }
        Ok(())
    })?;
    checkpoint("records")?;
    let vector_file = write_blob(
        &directory.join("vectors.bin"),
        max_bytes.saturating_sub(record_file.bytes),
        |out| {
            out.write_all(VECTOR_MAGIC).map_err(io)?;
            count(out, index.schema().fields().len())?;
            for i in 0..index.schema().fields().len() {
                let field = index.vectors.as_ref().and_then(|vectors| vectors.field(i));
                out.write_all(&[u8::from(field.is_some())]).map_err(io)?;
                if let Some(field) = field {
                    count(out, field.dimension)?;
                    count(out, field.data.len() / field.dimension.max(1))?;
                    count(out, field.rows.len())?;
                    for row in &field.rows {
                        out.write_all(&row.unwrap_or(u32::MAX).to_le_bytes())
                            .map_err(io)?;
                    }
                    for value in &field.data {
                        out.write_all(&value.to_le_bytes()).map_err(io)?;
                    }
                }
            }
            Ok(())
        },
    )?;
    checkpoint("vectors")?;
    let manifest = Manifest {
        format: FORMAT,
        generation,
        normalizer: NORMALIZER_VERSION,
        scoring: SCORING_VERSION,
        search: SEARCH_VERSION,
        schema_fingerprint: index.schema().preparation_fingerprint(),
        schema: schema_identity(index.schema()),
        records: index.len(),
        dimension: index.vectors.as_ref().map_or(0, |v| v.model().dimension),
        model: index.vectors.as_ref().map(|v| model_identity(v.model())),
        configuration,
        record_file,
        vector_file,
    };
    let data_bytes = manifest
        .record_file
        .bytes
        .saturating_add(manifest.vector_file.bytes);
    let manifest_file = write_blob(
        &directory.join("manifest.json"),
        METADATA_LIMIT.min(max_bytes.saturating_sub(data_bytes).saturating_sub(64)),
        |out| serde_json::to_writer(out, &manifest).map_err(|_| invalid("manifest serialization")),
    )?;
    checkpoint("manifest")?;
    let checksum = manifest_file.sha256.clone();
    write_small(&directory.join("COMPLETE"), checksum.as_bytes())?;
    checkpoint("complete")?;
    // Verify the exact written files before exposing the generation.
    verify_blob(&directory.join("manifest.json"), &manifest_file)?;
    if small_file(&directory.join("COMPLETE"), 64)? != checksum.as_bytes() {
        return Err(invalid("complete marker verification"));
    }
    verify_blob(&directory.join("records.bin"), &manifest.record_file)?;
    verify_blob(&directory.join("vectors.bin"), &manifest.vector_file)?;
    checkpoint("verified")?;
    Ok(checksum)
}
fn verify_blob(path: &Path, blob: &Blob) -> Result<()> {
    let mut reader = CheckedReader::new(regular_file(path)?, blob)?;
    let mut buffer = [0; 16384];
    while reader.read(&mut buffer).map_err(io)? != 0 {}
    reader.finish(blob)
}

impl Snapshot {
    /// Opens CURRENT exactly once, checking metadata and byte budgets before
    /// allocating record or vector buffers. Local filesystems only.
    /// # Errors
    /// Rejects incomplete, corrupt, incompatible or oversized snapshots.
    pub fn open(path: &Path, max_bytes: u64) -> Result<Self> {
        let current = small_file(&path.join("CURRENT"), 256)?;
        let text = std::str::from_utf8(&current).map_err(|_| invalid("CURRENT encoding"))?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len() != 2
            || !lines[0].starts_with("g-")
            || lines[0].len() > 96
            || !lines[0][2..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
            || !valid_digest(lines[1])
        {
            return Err(invalid("CURRENT pointer"));
        }
        let directory = path.join(lines[0]);
        if !fs::symlink_metadata(&directory)
            .map_err(io)?
            .file_type()
            .is_dir()
        {
            return Err(invalid("generation directory"));
        }
        if regular_file(&directory.join("manifest.json"))?
            .metadata()
            .map_err(io)?
            .len()
            > max_bytes
        {
            return Err(Error::snapshot_limit());
        }
        let bytes = small_file(
            &directory.join("manifest.json"),
            METADATA_LIMIT.min(max_bytes),
        )?;
        if digest(&bytes) != lines[1]
            || small_file(&directory.join("COMPLETE"), 64)? != lines[1].as_bytes()
        {
            return Err(invalid("incomplete generation or manifest checksum"));
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| invalid("manifest structure"))?;
        if manifest.format != FORMAT
            || manifest.normalizer != NORMALIZER_VERSION
            || manifest.scoring != SCORING_VERSION
            || manifest.search != SEARCH_VERSION
        {
            return Err(invalid(
                "format, normalizer, scoring or search version; rebuild the index",
            ));
        }
        if manifest
            .record_file
            .bytes
            .checked_add(manifest.vector_file.bytes)
            .and_then(|n| n.checked_add(bytes.len() as u64 + 64))
            .is_none_or(|n| n > max_bytes)
        {
            return Err(Error::snapshot_limit());
        }
        check_shape(
            manifest.records,
            manifest.dimension,
            manifest.model.is_some(),
        )?;
        let records = regular_file(&directory.join("records.bin"))?;
        let vectors = regular_file(&directory.join("vectors.bin"))?;
        for (file, blob) in [
            (&records, &manifest.record_file),
            (&vectors, &manifest.vector_file),
        ] {
            if file.metadata().map_err(io)?.len() != blob.bytes || !valid_digest(&blob.sha256) {
                return Err(invalid("data file size or checksum declaration"));
            }
        }
        Ok(Self {
            manifest,
            records,
            vectors,
        })
    }
    pub fn configuration(&self) -> &Value {
        &self.manifest.configuration
    }
    /// Checks the caller's record capacity before any model resolution.
    /// # Errors
    /// Returns `limit.index_capacity` when the saved count exceeds the limit.
    pub fn check_capacity(&self, max_records: usize) -> Result<()> {
        if self.manifest.records > max_records {
            return Err(Error::index_capacity(self.manifest.records, max_records));
        }
        Ok(())
    }

    pub fn generation(&self) -> u64 {
        self.manifest.generation
    }

    /// Rebuilds an empty compatible index from saved fields and f32 vectors,
    /// without invoking inference. Publication to the caller is all-or-nothing.
    /// # Errors
    /// Rejects checksums, malformed values, dimensions, rows or model identity.
    pub fn restore(self, mut index: SearchIndex) -> Result<SearchIndex> {
        check_metadata(&index)?;
        if self.manifest.schema != schema_identity(index.schema())
            || !index.is_empty()
            || index.schema().preparation_fingerprint() != self.manifest.schema_fingerprint
        {
            return Err(invalid("schema changed; rebuild the index"));
        }
        self.check_capacity(index.store().max_records())?;
        let model = index.vectors.as_ref().map(|v| v.model().clone());
        if model.as_ref().map(model_identity) != self.manifest.model {
            return Err(invalid("model identity changed; rebuild the index"));
        }
        let vectors = read_vectors(self.vectors, &self.manifest, index.schema(), model)?;
        let mut input = CheckedReader::new(self.records, &self.manifest.record_file)?;
        if &input.read_fixed::<8>()? != RECORD_MAGIC || input.count()? != self.manifest.records {
            return Err(invalid("record header"));
        }
        // Do not reserve based on untrusted count; each record consumes bytes.
        let mut records = Vec::new();
        for slot in 0..self.manifest.records {
            let left = usize::try_from(input.left).unwrap_or(usize::MAX);
            let record = univo_core::snapshot::read_record(&mut input, index.schema(), left)?;
            if let Some(vectors) = &vectors {
                for i in 0..index.schema().fields().len() {
                    if vectors.field(i).and_then(|v| v.get(slot)).is_some()
                        && !matches!(&record.fields()[i].state, univo_core::FieldState::Text(text) if !text.trim().is_empty())
                    {
                        return Err(invalid("vector without semantic text"));
                    }
                }
            }
            records.push(record);
        }
        input.finish(&self.manifest.record_file)?;
        index.add_preembedded(records, vectors)?;
        Ok(index)
    }
}

fn read_vectors(
    file: File,
    manifest: &Manifest,
    schema: &univo_core::Schema,
    model: Option<ModelDescriptor>,
) -> Result<Option<RecordVectors>> {
    let mut vectors = model.map(|model| RecordVectors::empty(schema, model));
    let mut input = CheckedReader::new(file, &manifest.vector_file)?;
    if &input.read_fixed::<8>()? != VECTOR_MAGIC || input.count()? != schema.fields().len() {
        return Err(invalid("vector header"));
    }
    for i in 0..schema.fields().len() {
        let present = input.read_fixed::<1>()?[0];
        let field = vectors.as_mut().and_then(|v| v.fields[i].as_mut());
        if present != u8::from(field.is_some()) {
            return Err(invalid("vector field presence"));
        }
        if let Some(field) = field {
            *field = read_vector_field(&mut input, manifest)?;
        }
    }
    input.finish(&manifest.vector_file)?;
    Ok(vectors)
}

fn read_vector_field(input: &mut CheckedReader, manifest: &Manifest) -> Result<FieldVectors> {
    let dimension = input.count()?;
    let rows = input.count()?;
    let records = input.count()?;
    if dimension != manifest.dimension
        || records != manifest.records
        || rows > records
        || rows >= u32::MAX as usize
    {
        return Err(invalid("vector shape"));
    }
    let values = rows
        .checked_mul(dimension)
        .ok_or_else(|| invalid("vector shape overflow"))?;
    let bytes = values
        .checked_add(records)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| invalid("vector byte overflow"))?;
    if bytes as u64 > input.left {
        return Err(invalid("vector buffer size"));
    }
    let mut slots = Vec::with_capacity(records);
    for _ in 0..records {
        let row = u32::from_le_bytes(input.read_fixed()?);
        if row != u32::MAX && row as usize >= rows {
            return Err(invalid("vector row offset"));
        }
        slots.push((row != u32::MAX).then_some(row));
    }
    let mut data = Vec::with_capacity(values);
    for _ in 0..values {
        data.push(f32::from_le_bytes(input.read_fixed()?));
    }
    // Validate directly in the owned buffer: no second matrix allocation.
    for row in data.chunks_exact(dimension) {
        let norm: f32 = row.iter().map(|v| v * v).sum();
        if !norm.is_finite() || (norm.sqrt() - 1.0).abs() > 1e-3 {
            return Err(invalid("non-finite or non-unit vector"));
        }
    }
    Ok(FieldVectors {
        dimension,
        data,
        rows: slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use univo_core::{ErrorCode, Schema, Value as RecordValue, prepare};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let id = SERIAL.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!("univo-snapshot-{}-{id}", std::process::id())))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn index(texts: &[&str]) -> SearchIndex {
        let schema = Schema::strings();
        let records = texts
            .iter()
            .map(|s| prepare(&schema, &RecordValue::Str((*s).into())).unwrap())
            .collect();
        let mut index = SearchIndex::new(schema, 100, None).unwrap();
        index.add(records).unwrap();
        index
    }
    fn restore(path: &Path) -> SearchIndex {
        Snapshot::open(path, 1 << 20)
            .unwrap()
            .restore(index(&[]))
            .unwrap()
    }
    fn current_directory(path: &Path) -> PathBuf {
        let pointer = fs::read_to_string(path.join("CURRENT")).unwrap();
        path.join(pointer.lines().next().unwrap())
    }
    fn change_manifest(path: &Path, change: impl FnOnce(&mut Manifest)) {
        let directory = current_directory(path);
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        change(&mut manifest);
        let bytes = serde_json::to_vec(&manifest).unwrap();
        fs::write(directory.join("manifest.json"), &bytes).unwrap();
        let checksum = digest(&bytes);
        fs::write(directory.join("COMPLETE"), &checksum).unwrap();
        let name = directory.file_name().unwrap().to_str().unwrap();
        fs::write(path.join("CURRENT"), format!("{name}\n{checksum}\n")).unwrap();
    }

    #[test]
    fn readers_keep_their_generation_and_rebuild_identical_results() {
        let directory = Directory::new();
        let first = index(&["AmAzOn", "Amazon", "Amiazon"]);
        first.save(&directory.0, 7, json!({}), 1 << 20).unwrap();
        let reader = Snapshot::open(&directory.0, 1 << 20).unwrap();
        index(&["Microsoft"])
            .save(&directory.0, 8, json!({}), 1 << 20)
            .unwrap();
        assert_eq!(reader.generation(), 7);
        let loaded = reader.restore(index(&[])).unwrap();
        assert_eq!(loaded.store().records(), first.store().records());
        assert_eq!(
            restore(&directory.0).store().records(),
            index(&["Microsoft"]).store().records()
        );
    }

    #[test]
    fn failure_at_every_boundary_leaves_one_complete_generation() {
        let checkpoints = [
            "locked",
            "directory",
            "records",
            "vectors",
            "manifest",
            "complete",
            "verified",
            "generation_synced",
            "pointer",
            "published",
            "root_synced",
            "parent_synced",
        ];
        for (position, stop) in checkpoints.iter().enumerate() {
            let directory = Directory::new();
            index(&["old"])
                .save(&directory.0, 1, json!({}), 1 << 20)
                .unwrap();
            let error = index(&["new", "new"])
                .save_with(&directory.0, 2, json!({}), 1 << 20, &mut |step| {
                    if step == *stop {
                        Err(io(std::io::Error::other("injected I/O failure")))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), ErrorCode::SnapshotIo);
            let expected = if position < 9 {
                index(&["old"])
            } else {
                index(&["new", "new"])
            };
            assert_eq!(
                restore(&directory.0).store().records(),
                expected.store().records(),
                "{stop}"
            );
            // A failed writer releases its OS lock for the next call.
            index(&["retry"])
                .save(&directory.0, 3, json!({}), 1 << 20)
                .unwrap();
        }
    }

    #[test]
    fn crash_writer() {
        let Ok(stop) = std::env::var("UNIVO_SNAPSHOT_CRASH_STEP") else {
            return;
        };
        let path = std::env::var_os("UNIVO_SNAPSHOT_CRASH_PATH").unwrap();
        index(&["new", "new"])
            .save_with(Path::new(&path), 2, json!({}), 1 << 20, &mut |step| {
                if step == stop {
                    std::process::exit(91);
                }
                Ok(())
            })
            .unwrap();
        panic!("crash checkpoint was not reached");
    }

    #[test]
    fn abrupt_process_exit_at_every_boundary_keeps_complete_state() {
        let checkpoints = [
            "locked",
            "directory",
            "records",
            "vectors",
            "manifest",
            "complete",
            "verified",
            "generation_synced",
            "pointer",
            "published",
            "root_synced",
            "parent_synced",
        ];
        for (position, stop) in checkpoints.iter().enumerate() {
            let directory = Directory::new();
            index(&["old"])
                .save(&directory.0, 1, json!({}), 1 << 20)
                .unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "snapshot::tests::crash_writer"])
                .env("UNIVO_SNAPSHOT_CRASH_STEP", stop)
                .env("UNIVO_SNAPSHOT_CRASH_PATH", &directory.0)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(91));
            assert_eq!(
                restore(&directory.0).len(),
                if position < 9 { 1 } else { 2 },
                "{stop}"
            );
            index(&["retry"])
                .save(&directory.0, 3, json!({}), 1 << 20)
                .unwrap();
        }
    }

    #[test]
    fn writer_refuses_a_second_writer_and_readers_continue() {
        let directory = Directory::new();
        index(&["old"])
            .save(&directory.0, 1, json!({}), 1 << 20)
            .unwrap();
        let lock = private_file(&directory.0.join("WRITER.lock"), false).unwrap();
        lock.try_lock().unwrap();
        let error = index(&["new"])
            .save(&directory.0, 2, json!({}), 1 << 20)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::SnapshotLocked);
        assert_eq!(restore(&directory.0).len(), 1);
        drop(lock);
        index(&["new"])
            .save(&directory.0, 2, json!({}), 1 << 20)
            .unwrap();
    }

    #[test]
    fn byte_budgets_and_failed_writes_preserve_previous_snapshot() {
        let directory = Directory::new();
        index(&["old"])
            .save(&directory.0, 1, json!({}), 1 << 20)
            .unwrap();
        let error = index(&[&"x".repeat(1000)])
            .save(&directory.0, 2, json!({}), 100)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::SnapshotLimit);
        assert_eq!(
            restore(&directory.0).store().records(),
            index(&["old"]).store().records()
        );
        assert_eq!(
            Snapshot::open(&directory.0, 100).unwrap_err().code(),
            ErrorCode::SnapshotLimit
        );
        assert_eq!(
            index(&["new"])
                .save(
                    &directory.0,
                    2,
                    json!({"large": "x".repeat(2 << 20)}),
                    4 << 20
                )
                .unwrap_err()
                .code(),
            ErrorCode::SnapshotLimit
        );
    }

    #[test]
    fn corruption_versions_and_pointer_traversal_are_rejected() {
        for file in ["records.bin", "vectors.bin", "manifest.json", "COMPLETE"] {
            let directory = Directory::new();
            index(&["old"])
                .save(&directory.0, 1, json!({}), 1 << 20)
                .unwrap();
            let path = current_directory(&directory.0).join(file);
            let mut bytes = fs::read(&path).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
            fs::write(path, bytes).unwrap();
            let result = Snapshot::open(&directory.0, 1 << 20).and_then(|s| s.restore(index(&[])));
            assert!(result.is_err(), "{file}");
        }
        let directory = Directory::new();
        index(&["old"])
            .save(&directory.0, 1, json!({}), 1 << 20)
            .unwrap();
        change_manifest(&directory.0, |m| m.normalizer += 1);
        assert_eq!(
            Snapshot::open(&directory.0, 1 << 20).unwrap_err().code(),
            ErrorCode::SnapshotInvalid
        );
        fs::write(
            directory.0.join("CURRENT"),
            "../outside\n0000000000000000000000000000000000000000000000000000000000000000\n",
        )
        .unwrap();
        assert!(Snapshot::open(&directory.0, 1 << 20).is_err());
    }

    #[derive(Debug)]
    struct CountingModel {
        descriptor: ModelDescriptor,
        calls: AtomicU64,
    }
    impl univo_embed::Embedder for CountingModel {
        fn descriptor(&self) -> &ModelDescriptor {
            &self.descriptor
        }
        fn encode_batch(&self, texts: &[&str]) -> Result<univo_embed::Embeddings> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut values = Vec::new();
            for text in texts {
                values.extend_from_slice(if *text == "unknown" {
                    &[0.0, 0.0]
                } else {
                    &[1.0, 0.0]
                });
            }
            univo_embed::Embeddings::new("test", values, 2)
        }
    }
    fn semantic_index(model: Arc<CountingModel>) -> SearchIndex {
        let schema = Schema::builder()
            .text(
                univo_core::FieldPath::parse("name").unwrap(),
                univo_core::Text {
                    mode: univo_core::TextMode::Semantic,
                    ..Default::default()
                },
            )
            .build()
            .unwrap();
        SearchIndex::new(schema, 100, Some(model)).unwrap()
    }
    fn test_model() -> Arc<CountingModel> {
        Arc::new(CountingModel {
            descriptor: ModelDescriptor {
                source: ModelSource::Local {
                    path: "test".into(),
                    files: vec![],
                },
                dimension: 2,
                backend: "test",
            },
            calls: AtomicU64::new(0),
        })
    }
    #[test]
    fn format_dimension_limit_is_checked_before_any_save_side_effect() {
        let directory = Directory::new();
        let mut model = test_model();
        Arc::get_mut(&mut model).unwrap().descriptor.dimension = MAX_DIMENSION;
        let valid = semantic_index(Arc::clone(&model));
        valid.save(&directory.0, 1, json!({}), 1 << 20).unwrap();
        let current = fs::read(directory.0.join("CURRENT")).unwrap();
        let loaded = Snapshot::open(&directory.0, 1 << 20)
            .unwrap()
            .restore(semantic_index(model))
            .unwrap();
        assert!(loaded.is_empty());
        assert_eq!(
            loaded.vectors.as_ref().unwrap().model().dimension,
            MAX_DIMENSION
        );

        let mut model = test_model();
        Arc::get_mut(&mut model).unwrap().descriptor.dimension = MAX_DIMENSION + 1;
        let invalid = semantic_index(model);
        for path in [&directory.0, &directory.0.join("new-index")] {
            let error = invalid.save(path, 2, json!({}), 1 << 20).unwrap_err();
            assert_eq!(error.code(), ErrorCode::SnapshotInvalid);
        }
        assert!(!directory.0.join("new-index").exists());
        assert_eq!(fs::read(directory.0.join("CURRENT")).unwrap(), current);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 3); // generation, CURRENT, lock
    }

    #[test]
    fn simple_relative_paths_use_the_current_directory_as_parent() {
        assert_eq!(parent_directory(Path::new("index")), Path::new("."));
        assert_eq!(
            parent_directory(Path::new("parent/index")),
            Path::new("parent")
        );
    }

    #[cfg(unix)]
    #[test]
    fn relative_parent_sync_worker() {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("UNIVO_SNAPSHOT_RELATIVE_SYNC_TEST").is_none() {
            return;
        }
        let mut read_denied = false;
        let result =
            index(&["old"]).save_with(Path::new("index"), 1, json!({}), 1 << 20, &mut |step| {
                if step == "root_synced" {
                    fs::set_permissions(".", fs::Permissions::from_mode(0o300)).unwrap();
                    read_denied = File::open(".").is_err();
                }
                Ok(())
            });
        fs::set_permissions(".", fs::Permissions::from_mode(0o700)).unwrap();
        if read_denied {
            assert_eq!(result.unwrap_err().code(), ErrorCode::SnapshotIo);
        } else {
            // Privileged users may bypass permissions; path selection is also tested directly.
            result.unwrap();
        }
        assert_eq!(restore(Path::new("index")).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn relative_parent_sync_failure_is_reported_after_complete_publication() {
        let directory = Directory::new();
        private_directory(&directory.0).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "snapshot::tests::relative_parent_sync_worker"])
            .env("UNIVO_SNAPSHOT_RELATIVE_SYNC_TEST", "1")
            .current_dir(&directory.0)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(restore(&directory.0.join("index")).len(), 1);
    }

    #[test]
    fn semantic_restore_never_encodes_and_preserves_shared_and_missing_rows() {
        let directory = Directory::new();
        let model = test_model();
        let mut first = semantic_index(Arc::clone(&model));
        let records = ["Amazon", "Amazon", "unknown", ""]
            .iter()
            .map(|s| {
                prepare(
                    first.schema(),
                    &RecordValue::Map(
                        univo_core::Map::new(vec![("name".into(), RecordValue::Str((*s).into()))])
                            .unwrap(),
                    ),
                )
                .unwrap()
            })
            .collect();
        first.add(records).unwrap();
        first.save(&directory.0, 1, json!({}), 1 << 20).unwrap();
        let before = model.calls.load(Ordering::Relaxed);
        let loaded = Snapshot::open(&directory.0, 1 << 20)
            .unwrap()
            .restore(semantic_index(Arc::clone(&model)))
            .unwrap();
        assert_eq!(model.calls.load(Ordering::Relaxed), before);
        assert_eq!(first.vectors, loaded.vectors);
        assert_eq!(first.store().records(), loaded.store().records());
        let mut different = test_model();
        Arc::get_mut(&mut different).unwrap().descriptor.backend = "another-backend";
        assert!(
            Snapshot::open(&directory.0, 1 << 20)
                .unwrap()
                .restore(semantic_index(different))
                .is_err()
        );
    }

    #[test]
    fn forged_counts_offsets_and_nan_fail_even_with_valid_checksums() {
        for (offset, replacement) in [
            (17, u64::MAX.to_le_bytes().to_vec()),
            (41, u32::MAX.saturating_sub(1).to_le_bytes().to_vec()),
            (45, f32::NAN.to_le_bytes().to_vec()),
        ] {
            let directory = Directory::new();
            let model = test_model();
            let mut first = semantic_index(Arc::clone(&model));
            let record = prepare(
                first.schema(),
                &RecordValue::Map(
                    univo_core::Map::new(vec![("name".into(), RecordValue::Str("Amazon".into()))])
                        .unwrap(),
                ),
            )
            .unwrap();
            first.add(vec![record]).unwrap();
            first.save(&directory.0, 1, json!({}), 1 << 20).unwrap();
            let file = current_directory(&directory.0).join("vectors.bin");
            let mut bytes = fs::read(&file).unwrap();
            bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
            fs::write(file, &bytes).unwrap();
            change_manifest(&directory.0, |m| m.vector_file.sha256 = digest(&bytes));
            assert!(
                Snapshot::open(&directory.0, 1 << 20)
                    .unwrap()
                    .restore(semantic_index(model))
                    .is_err()
            );
        }
    }
}

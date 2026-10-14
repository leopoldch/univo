//! Immutable, bounded local snapshots with a single atomic publication point.
use std::fs::{self, File, TryLockError};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use univo_core::{Error, NORMALIZER_VERSION, Result, SCORING_VERSION};
use univo_embed::{Digest, ModelDescriptor, ModelSource};

use crate::vectors::{FieldVectors, RecordVectors};
use crate::{SEARCH_VERSION, SearchIndex};

mod blob;
mod files;
#[cfg(test)]
mod tests;

use blob::{Blob, CheckedReader, count, digest, valid_digest, verify_blob, write_blob};
use files::{
    parent_directory, private_directory, private_file, regular_file, small_file, sync_directory,
    write_small,
};

const FORMAT: u32 = 1;
const METADATA_LIMIT: u64 = 1 << 20;
const MAX_DIMENSION: usize = 65_536;
const RECORD_MAGIC: &[u8; 8] = b"UNIVOR01";
const VECTOR_MAGIC: &[u8; 8] = b"UNIVOV01";
static SERIAL: AtomicU64 = AtomicU64::new(0);
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ann: Option<Value>,
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

fn check_shape(records: usize, dimension: usize, has_model: bool) -> Result<()> {
    if records > u32::MAX as usize || dimension > MAX_DIMENSION || has_model != (dimension > 0) {
        return Err(invalid("record count or dimension"));
    }
    Ok(())
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
        for slot in index.store().active_slots() {
            univo_core::snapshot::write_record(out, &index.store().records()[slot])?;
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
                    if index.store().len() == index.len() {
                        count(out, field.data.len() / field.dimension.max(1))?;
                        count(out, field.rows.len())?;
                        for row in &field.rows {
                            out.write_all(&row.unwrap_or(u32::MAX).to_le_bytes())
                                .map_err(io)?;
                        }
                        for value in &field.data {
                            out.write_all(&value.to_le_bytes()).map_err(io)?;
                        }
                    } else {
                        let (rows, source) = field.selected_rows(index.store().active_slots())?;
                        count(out, source.len())?;
                        count(out, rows.len())?;
                        for row in rows {
                            out.write_all(&row.unwrap_or(u32::MAX).to_le_bytes())
                                .map_err(io)?;
                        }
                        for row in source {
                            let start = row as usize * field.dimension;
                            for value in &field.data[start..start + field.dimension] {
                                out.write_all(&value.to_le_bytes()).map_err(io)?;
                            }
                        }
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
        ann: index.ann_options.map(crate::AnnOptions::identity),
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
            || !index.store().is_empty()
            || index.schema().preparation_fingerprint() != self.manifest.schema_fingerprint
        {
            return Err(invalid("schema changed; rebuild the index"));
        }
        if self.manifest.ann != index.ann_options.map(crate::AnnOptions::identity) {
            return Err(invalid("ANN configuration changed; rebuild the index"));
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
            let left = usize::try_from(input.remaining()).unwrap_or(usize::MAX);
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
    if bytes as u64 > input.remaining() {
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

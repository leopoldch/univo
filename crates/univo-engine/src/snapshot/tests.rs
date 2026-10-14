use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest as _, Sha256};

use super::blob::CHUNK;
use super::*;
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
fn chunked_hashing_matches_the_digest_of_the_bytes() {
    let directory = Directory::new();
    fs::create_dir_all(&directory.0).unwrap();
    let path = directory.0.join("blob");
    let pieces: Vec<Vec<u8>> = [1, 3, CHUNK - 1, 7, CHUNK, 2, 3 * CHUNK + 5, 0, 11]
        .iter()
        .enumerate()
        .map(|(i, &len)| {
            (0..len)
                .map(|j| u8::try_from((i * 31 + j) % 251).unwrap())
                .collect()
        })
        .collect();
    let blob = write_blob(&path, u64::MAX, |out| {
        for piece in &pieces {
            out.write_all(piece).map_err(io)?;
        }
        Ok(())
    })
    .unwrap();
    let all: Vec<u8> = pieces.concat();
    assert_eq!(blob.bytes, all.len() as u64);
    assert_eq!(
        blob.sha256,
        Digest::Sha256(Sha256::digest(&all).into()).to_hex()
    );
    assert_eq!(fs::read(&path).unwrap(), all);
    // Read back in small and large pieces: same bytes, same digest.
    let mut reader = CheckedReader::new(File::open(&path).unwrap(), &blob).unwrap();
    let mut read = Vec::new();
    let mut size = 1;
    loop {
        let mut piece = vec![0; size];
        let n = reader.read(&mut piece).unwrap();
        if n == 0 {
            break;
        }
        read.extend_from_slice(&piece[..n]);
        size = size * 7 % 70_001 + 1;
    }
    assert_eq!(read, all);
    reader.finish(&blob).unwrap();
    // Bytes left unread are trailing data, even once hashed.
    let mut reader = CheckedReader::new(File::open(&path).unwrap(), &blob).unwrap();
    reader.read_fixed::<4>().unwrap();
    assert!(reader.finish(&blob).is_err());
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

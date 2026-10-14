# 0008. Model download, cache and inference backend

- Status: Accepted
- Date: 2026-10-08
- Roadmap: U-009

## Context

[ADR 0007](0007-default-embedding-model.md) selects static Model2Vec models, a pinned default and model selection by string. [04-models-and-cache.md](../../specs/04-models-and-cache.md) requires a download on first use only, pinned revisions and checksums, verification before an atomic publication, a lock per model, a strict offline path and no network access at import or Finder construction. [AGENTS.md](../../AGENTS.md) adds minimal installed size and dependencies, no network client in the core, and no panic across a binding.

A download needs an HTTP client and a TLS stack. The core is shared by every binding (Python now, Java later), and each of these languages already ships an HTTP client that uses the system's certificates and proxies.

## Decision

- **A separate crate, `univo-embed`**, holds models: references, the release registry, download plans, the cache and inference. `univo-core` does not depend on it.
- **No network access in Rust.** The crate builds URLs and validates what comes back; the binding performs the HTTP GET requests with its language's standard library (`urllib` in Python) and writes each response into the staging path Rust gives it. No HTTP, TLS or async crate is linked, which `tools/checks/check_native_dependencies.py` enforces in CI.
- **Policy in Rust, shared by every binding**:
  - `ModelSpec::parse` reads `default`, `org/name[@revision]` and local paths (absolute, `./`, `../`, `~`, `file://`); names and revisions are restricted to characters that are safe in a URL and a path.
  - The default model's commit, file sizes and SHA-256 are compiled into the release (`registry::default_model`); its plan needs no metadata request.
  - Another hub model is planned from the hub's metadata at the requested revision, `main` when none is given. Files are pinned to the commit the metadata reports, so a branch that moves during a download cannot mix versions. Only `config.json`, `model.safetensors` and `tokenizer.json` are fetched. Each needs a published digest: the LFS SHA-256 for large files, or the Git blob SHA-1 (`sha1("blob <size>\0" + content)`) for small files the hub keeps in Git. The total size is capped, 2 GiB by default (`model.too_large`).
  - The cache lives under `UNIVO_CACHE_DIR` or the platform cache directory: `models/<org>/<name>/<commit>/` with a `univo-model.json` manifest written last, `refs/<revision>` for branches and tags (`main` included), and `<commit>.lock`.
  - Files are staged in `<commit>.partial/`, verified, and published with one directory rename.
  - A per-commit OS file lock, released by the OS if the holder dies, serializes downloads; waiting times out with `model.cache_locked`. When the commit is already published, staging records the requested reference instead of downloading.
  - A dropped staging removes its files; the leftovers of a process that died are removed by the next download of the same commit.
  - Reading the cache never writes, so a read-only cache works.
  - Loading a cached model reads each file once and verifies the bytes in memory against the manifest's digests, so a file changed in place, even at the same size, is reported as `model.integrity`, and the bytes checked are the bytes used. Hashing runs on a second thread while the model is built.
  - A local directory is read as it is, and identified by the SHA-256 of the files loaded, so that a changed file changes the descriptor.
- **Inference with `model2vec-rs` 0.3.0**, pinned exactly, without default features (`local-only`, `fancy-regex`). Rust reads the verified files and hands their bytes to `StaticModel::from_bytes`, so the backend never resolves paths or downloads. Backend panics are caught and reported as `model.corrupt`. Every vector must be finite and of unit length. A text without any known token is marked absent instead of producing an arbitrary vector. Texts are never truncated, since a static model averages token vectors. Truncation and padding configured in `tokenizer.json` are removed before loading: truncation would drop tokens silently, and padding would make a text's vector depend on the other texts of its batch.
- **Compatibility**: a `ModelDescriptor` records the source (repository, commit and file digests, or local path and file SHA-256), the dimension and the backend version. U-010 puts it into the embedding cache key and the index compatibility check.

## Alternatives

- **`hf-hub` with `ureq` and `rustls` in Rust**: one implementation for every binding, but a TLS stack and certificate store inside each wheel, proxy and certificate settings that differ from the host language, and a network crate reachable from the core. Rejected.
- **`native-tls`/OpenSSL in Rust**: links a system library that manylinux wheels cannot assume. Rejected.
- **`huggingface_hub` as a Python dependency**: a runtime dependency tree for a few GET requests, and a second implementation of the policy that Java would have to repeat. Rejected; the Hugging Face cache layout is not reused for the same reason, though a user may point a local path at it.
- **ONNX Runtime or Candle as the backend**: needed only for contextual encoders, which ADR 0007 excludes from the defaults; both are far larger than static inference.
- **An in-house WordPiece tokenizer instead of `tokenizers`**: about 100 fewer linked crates and potentially a much faster encoder (see Evidence), but a second implementation of the Hugging Face normalizer, pre-tokenizer and WordPiece model to keep exact. Deferred; see Revisit when.

## Evidence

Apple M2, macOS, release builds, default model `minishlab/potion-base-8M@bf8b0566`, files in the OS page cache:

| Measure | Result |
| --- | --- |
| Load from local files, verify every file and encode one text, median of 5 | 18 ms (specification: p95 ≤ 2 s); SHA-256 of the 30 MB of weights takes 13 ms and building the model 12 ms, run in parallel |
| 20 000 descriptions of 30 words (39 tokens), one thread, median of 5 | 0.95 s, 47 µs per text, about 21 000 per second (specification: ≥ 2000 objects per second; U-004 spike: 21 563) |
| Same, 8 threads | 0.37 s |
| `tokenizers` alone on the same texts, one thread | the same time within 1 %: tokenization is almost the whole cost |
| Stripped release `cdylib`, inference linked against plans and cache only | +2.68 MB, +1.15 MB compressed |
| Linked crates | 108 for `univo-embed`, 102 more than `univo-core`; no network crate, no C or C++ build |

The Git blob SHA-1 of the default model's `config.json` and `tokenizer.json` was checked against the hub's published `blobId`; the SHA-256 of `model.safetensors` against its LFS digest.

## Consequences

- The minimum supported Rust version becomes 1.89 (`model2vec-rs` needs 1.88; `File::try_lock` is stable from 1.89).
- `tokenizers` tokenizes batches on Rayon's global pool. Callers that bound threads run encoding inside their own pool; U-010 sets that policy so that encoding and search do not oversubscribe the machine.
- Each binding implements only the transport: fetch a URL into a path, with a timeout and a bounded number of retries. Integrity, layout, locking and publication stay identical across languages.
- Models from a mirror served over `http://` are protected by the digests of the metadata that mirror serves. The default model is verified against the release's checksums whatever the endpoint.
- Third-party notices for the new crates ship with the wheels in U-019.

## Revisit when

- U-010 measures tokenization as a significant share of end-to-end latency: replace `tokenizers` with a WordPiece implementation validated against it as an oracle on the supported models.
- `model2vec-rs` changes its file format or loading API, or stops being maintained.
- A binding's language has no usable standard HTTP client.
- The hub changes its metadata API or its digest scheme.

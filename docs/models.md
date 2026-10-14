# Models, cache and offline use

Univo computes text embeddings locally with a static [Model2Vec](https://github.com/MinishLab/model2vec) model ([ADR 0007](adr/0007-default-embedding-model.md), [ADR 0021](adr/0021-multilingual-default-model.md)). Records never leave the machine. The only network access is the download of model files, on first use, and it can be done in advance or forbidden.

## Choosing a model

A model is named by a string:

| String | Model |
| --- | --- |
| `"default"` | The release's default model, `univo-models/static-similarity-mrl-multilingual-int8-128` at a pinned commit, verified against checksums compiled into Univo: 50 languages, 128 dimensions, 15 MB |
| `"org/name"` | A Hugging Face repository at its `main` branch, resolved to a commit at the first download |
| `"org/name@revision"` | A repository at a branch, tag or commit; a full commit hash is recommended |
| `/path`, `./path`, `../path`, `~/path`, `file:///path` | A local directory holding `config.json`, `model.safetensors` and `tokenizer.json` |

The default model is a reduced copy of `sentence-transformers/static-similarity-mrl-multilingual-v1` (Apache-2.0). Its [model card](https://huggingface.co/univo-models/static-similarity-mrl-multilingual-int8-128) lists the modifications and the measured quality. On organization names it recovers 32.9% of aliases against 19.6% for the previous English-only default, `minishlab/potion-base-8M`, at the same ≥ 98% validation precision. On an exploratory test split, the gain comes mostly from aliases written in another language, at the cost of somewhat more false matches between similar names. To keep the previous default, name it: `model="minishlab/potion-base-8M@bf8b056651a2c21b8d2565580b8569da283cab23"`. A snapshot always reloads the model it was saved with.

`org/name` is never read as a relative path: prefix local directories with `./`. Only Model2Vec models run; another format, such as a sentence-transformers repository, is refused with `model.unsupported_format`, and no downloaded code is ever executed.

## Preparing a machine

```sh
python -m univo.models download                      # the default model
python -m univo.models download minishlab/potion-base-2M
python -m univo.models inspect                       # what is available, without network access
python -m univo.models download --json               # machine-readable output
```

In Python, `univo.models.download(model, hub_url=None, cache_dir=None)` and `univo.models.inspect(model, cache_dir=None)` return a `ModelInfo` with the model's name, commit, path, dimension, backend and file digests.

## The cache

Models are stored under `UNIVO_CACHE_DIR`, or `~/Library/Caches/univo` on macOS, `%LOCALAPPDATA%\univo\cache` on Windows and `$XDG_CACHE_HOME/univo` or `~/.cache/univo` elsewhere:

```text
models/<org>/<name>/<commit>/        model files and univo-model.json
models/<org>/<name>/refs/<revision>  commit a branch or tag resolved to
```

- A cached model is used without any request, including a branch resolved by an earlier download. To move to a newer revision, name it explicitly.
- Downloads are verified before they are published: against the release's checksums for the default model, otherwise against the SHA-256 (large files) or Git blob id (small files) the hub publishes. Files are pinned to one commit, so a branch that moves during a download cannot mix versions.
- Every load verifies the files again, in memory, and fails with `model.integrity` if one changed.
- Two processes downloading the same model wait for each other through a file lock, which the operating system releases if a process dies. Interrupted downloads leave no partial model behind.
- A read-only cache works once the model is in it.

## Offline use

With `offline=True`, Univo makes no network request at all, not even to check metadata. A missing model fails immediately with `ModelUnavailableError` (`model.unavailable`), naming the cache directory. Importing `univo` or `univo.models` loads no network module, and loading a cached model does not either.

## Mirrors and proxies

`hub_url`, or the `HF_ENDPOINT` environment variable, names a mirror that implements the Hugging Face API. Requests carry only the repository, revision and file names, and a `univo/<version>` User-Agent. Downloads use Python's standard library: the system's proxy settings (`HTTPS_PROXY`, `NO_PROXY`) and the certificates of Python's OpenSSL. Each request times out after 30 s of silence and is attempted at most three times; only connection failures, timeouts and HTTP 429 and 5xx are retried.

Private or gated repositories are not supported yet: download them with your own tools and pass the local directory.

## Errors

| Code | Exception | Cause |
| --- | --- | --- |
| `model.invalid_spec` | `ModelUnavailableError` | The string is not a valid reference, or the hub URL is invalid |
| `model.unavailable` | `ModelUnavailableError` | Not cached and offline, or the repository does not exist |
| `model.unsupported_format` | `ModelUnavailableError` | Not a Model2Vec model |
| `model.download_failed` | `ModelUnavailableError` | A request failed after its retries |
| `model.cache_locked` | `ModelUnavailableError` | Another process holds the model's lock for more than 10 minutes |
| `model.too_large` | `ResourceLimitError` | The model exceeds 2 GiB |
| `model.integrity` | `ModelIntegrityError` | A file differs from its expected size or digest |
| `model.corrupt` | `ModelIntegrityError` | The files cannot be read as a model |

See the [error catalog](errors.md) for messages and hints.

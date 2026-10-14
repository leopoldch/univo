# Releasing

Publishing a GitHub release publishes Univo on PyPI. [`release.yml`](../.github/workflows/release.yml) does the rest, without a stored token:

1. **Check the version.** The tag must be `v` followed by the workspace version of `Cargo.toml`, and the tagged commit must be on `main`. Version `0.0.0` is refused.
2. **Run the whole CI.** This is [`ci.yml`](../.github/workflows/ci.yml): lint, Rust on six platforms, then one wheel per platform, each installed and tested outside the source tree.
3. **Check the source distribution.** It is built, compiled in a clean environment and run against the test suite.
4. **Upload.** The job first checks that `dist/` holds exactly one sdist and one wheel per platform ([`check_release.py`](../tools/check_release.py)). It then uploads them with [trusted publishing](https://docs.pypi.org/trusted-publishers/), which uses GitHub's OIDC identity instead of an API token.
5. **Attach the same files to the GitHub release.**

If any step fails, nothing is uploaded. A release marked as a **pre-release** goes to [TestPyPI](https://test.pypi.org/project/univo/) instead of PyPI: use it to try the whole chain.

## One-time setup

The maintainer does this once. Nothing in the repository can do it.

1. **GitHub Actions minutes.** A release runs the six-platform CI. The repository must be public, which makes standard runners free, or have enough minutes.
2. **GitHub environments.** In *Settings → Environments*, create `pypi` and `testpypi`.
   - Adding yourself as a required reviewer on `pypi` makes every upload wait for your approval.
3. **PyPI trusted publisher.** On [pypi.org](https://pypi.org/manage/account/publishing/), under *Add a new pending publisher*, enter:
   - project `univo`;
   - owner `leopoldch`;
   - repository `univo`;
   - workflow `release.yml`;
   - environment `pypi`.

   A pending publisher does not reserve the name: anyone can still register `univo` until the first upload creates the project. Publish the first release soon after.
4. **TestPyPI trusted publisher.** Do the same on [test.pypi.org](https://test.pypi.org/manage/account/publishing/), with environment `testpypi`.

## Each release

1. **Set the version.** Open a PR that changes `Cargo.toml`, then merge it.
   - Set `version` under `[workspace.package]`, for example `0.1.0`, or `0.1.0-rc.1` for a candidate.
   - Set the same version in the three `univo-*` entries of `[workspace.dependencies]`.
   - Run `cargo update --workspace` so that `Cargo.lock` follows.
   - PyPI spells the version as PEP 440 requires, for example `0.1.0rc1`.
2. **Create the release.** On GitHub, open *Releases → Draft a new release*. Create the tag `v0.1.0` on `main` and write the notes.
   - To publish to TestPyPI first, tick *Set as a pre-release*.
3. **Publish it** and follow the *Release* workflow in the *Actions* tab. If you configured a required reviewer, approve the `pypi` environment when asked.

A version cannot be uploaded twice to PyPI, even after deletion. If the workflow fails after uploading, fix the problem and release the next version.

The default model is downloaded from Hugging Face, not from PyPI. Its repository must be published at the commit the release pins ([models](models.md)).

# Security

Univo is built to handle sensitive data safely:

- **Your records stay on your machine.** Inference is local. Univo has no telemetry and never sends records anywhere.
- **Models are verified.** The only network access downloads model files, which must match pinned checksums. Offline mode makes no network access at all.
- **No code execution from files.** Univo never runs downloaded code and never uses pickle.
- **Snapshots are checked before use.** Size, checksums and versions are verified before anything is loaded. Symbolic links are refused, and on Unix the files are readable only by their owner.
- **Bounded inputs.** Batches, reports and indexes have explicit limits that fail with typed errors instead of exhausting memory.

## Reporting a vulnerability

Report it privately through [GitHub security advisories](https://github.com/leopoldch/univo/security/advisories/new), not in a public issue. You will get an answer within a week.

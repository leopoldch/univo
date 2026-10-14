# Security

Report vulnerabilities privately through [GitHub security advisories](https://github.com/leopoldch/univo/security/advisories/new), not in public issues. Expect an answer within a week.

Univo has no release yet; fixes go to `main` and, from the first release, to the latest one.

**Boundaries**
- **Network.** Records never leave the machine. The only network access downloads model files, verified against pinned checksums; offline mode makes none.
- **Untrusted input.** No downloaded code is executed, and no state uses pickle.
- **Snapshots.** They are checked for size, checksum and version before loading, and regular files are required, so symbolic links are refused.
- **Limits.** Batches, reports and indexes have explicit limits that fail with typed errors.

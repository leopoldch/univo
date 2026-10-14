# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately, through [GitHub's private vulnerability reporting](https://github.com/leopoldch/univo/security/advisories/new), not in a public issue. Include the affected version or commit, a description of the impact and the steps to reproduce. You will receive an answer within a week.

## Supported versions

Univo has no release yet. Until 1.0, fixes go to the latest release and to `main`.

## Scope

Univo processes data locally, so these boundaries matter most:
- **Network.** Records never leave the machine. The only network access downloads model files, which are verified against pinned checksums before use. Strict offline mode makes no network access at all.
- **Models.** Downloaded model code is never executed, and no persistent state uses pickle.
- **Snapshots.** Loading a snapshot checks sizes, checksums and versions before allocating, and rejects symbolic links in place of snapshot files. A snapshot is data, never code.
- **Resources.** Batches, reports and indexes have explicit limits; exceeding one is a typed error, not unbounded memory use.

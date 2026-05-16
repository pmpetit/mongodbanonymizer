# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [0.1.0] — 2026-05-16

### Added

#### Commands

- **`manon init`** — create a project directory structure (`source/collections/`,
  `config/`) and a `.conf` file that stores the MongoDB URI, namespace, and
  sampling parameters for repeatable runs.
- **`manon infer`** — sample a MongoDB collection, infer its BSON schema
  (field types, presence probabilities, distinct-value counts), annotate
  sensitive fields automatically, and write a YAML schema file with masking
  rules.
- **`manon mask`** — re-read the identifier dictionaries, re-annotate fields,
  and refresh the anonymized `values` samples in an existing YAML schema file
  without re-connecting to MongoDB.
- **`manon apply`** — stream documents from a source MongoDB collection, apply
  masking rules from a YAML schema file in memory, and bulk-insert the
  anonymized documents into a target collection in batches of 500.

#### Namespace scoping

- **DB-level inference**: passing only a database name (no `.`) to
  `--namespace` in `manon infer` enumerates and infers every non-system
  collection in the database.
- **DB-level apply**: passing only a database name (no `.`) to `--namespace`
  in `manon apply` processes every collection that has a matching YAML file in
  the `--masking-rules` directory.

#### Sampling and filtering

- `--number` / `-n` on `infer` — fixed document sample size (default 1 000).
- `--percent` / `-p` on `infer` — percentage-based document sampling.
- `--percent` / `-p` on `apply` — copy only a fraction of each source
  collection; useful for ephemeral or short-lived environments.

#### Config-file support

- `-c / --config` on `infer` and `apply` — derive the MongoDB URI, namespace,
  and output paths from a `.conf` file created by `manon init`; CLI flags
  always take priority.

#### Masking methods

Nine built-in masking methods, all deterministic (same input → same output):

| Method | Description |
|---|---|
| `PRESERVE_TOKEN` | Replaces characters class-by-class; keeps separators |
| `REDACT_ALPHANUMERIC` | Replaces alphanumeric chars with `X` / `0` |
| `MASK_CONTACT_URI` | Anonymizes e-mail local parts, URL paths, phone digits |
| `MASK_NETWORK_ID` | Anonymizes IPv4, IPv6, and MAC addresses segment by segment |
| `GENERALIZE_LOCATION` | Truncates postal codes to the first 2–3 characters |
| `NOISY_DATE` | Shifts dates by a deterministic ±30-day noise |
| `NOISY_POSITION` | Adds deterministic ±0.009° (~1 km) noise to coordinates |
| `STATIC_MAPPING` | Maps to one of five tokens (A–E) deterministically |
| `STATIC_BLOB_REPLACEMENT` | Replaces the value with `[REDACTED]` |

#### Field-detection dictionaries

- `data/identifier.csv` — maps field-name patterns (with locale) to masking
  categories.
- `data/identifier_category.csv` — maps categories to masking methods.
- Both files are embedded at compile time; no runtime data directory required.

#### Documentation

- Full MkDocs site under `docs/` (install, how-to, reference, masking methods,
  tutorial).
- `CONTRIBUTING.md` with guidance on extending the identifier dictionaries and
  proposing new masking methods.

#### CI / CD

- GitHub Actions workflow (`release.yml`) — builds `manon` for six targets
  (Linux x86_64/arm64, macOS x86_64/arm64, Windows x86_64/arm64) and creates
  a GitHub Release on `v*.*.*` tags.
- GitHub Actions workflow (`pr-preview.yml`) — builds a Linux x86_64 binary
  on every pull request and posts a download link as a PR comment.

[Unreleased]: https://github.com/pmpetit/mongodbanonymizer/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pmpetit/mongodbanonymizer/releases/tag/v0.1.0

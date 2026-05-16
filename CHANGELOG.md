# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [0.1.2] — 2026-05-16

### Added

- **End-to-end test suite** (`tests/e2e_tests.rs`): 6 testcontainer-based tests
  using synthetic data that exercise the full infer → apply pipeline, DB-level
  operations, `--percent`, and the `--number` sampling limit.  No external
  setup required beyond Docker.

- **Sample-dataset e2e tests** (`tests/e2e_sample_tests.rs`): tests against all
  7 official MongoDB sample databases
  ([neelabalan/mongodb-sample-dataset](https://github.com/neelabalan/mongodb-sample-dataset)),
  downloaded on the fly from GitHub — no local data files needed.  Covers
  schema inference, sensitive-field detection, DB-level workflow, `--percent`
  apply, and the `helpers` module (`existing_db`, `existing_collection`,
  `get_locale`, `get_metadata`).

- **Unit tests for `manon init` / `read_conf` / `manon mask`** (`tests/cli_tests.rs`):
  10 new tests covering directory creation, `.conf` file generation, config
  parsing (all fields, commented lines, missing required keys), in-place
  masking, `--output` path redirection, and error handling on missing input
  files.

- **Unit tests for `helpers::parse_namespace`** (`tests/cli_tests.rs`):
  6 tests covering the happy path, first-dot splitting, no-dot error, empty
  string, leading dot, and trailing dot edge cases.

- **Coverage reporting** via [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):
  - `task coverage` — HTML report from all tests (Docker required), opens browser.
  - `task coverage:unit` — HTML report from unit tests only (no Docker).
  - CI `coverage` job in `pr-preview.yml` collects LCOV data from both test
    suites and uploads to Codecov on every PR.

- **Task shortcuts** in `Taskfile.yml`:
  - `task test:unit` — unit tests only (no Docker).
  - `task test:e2e` — e2e tests (Docker required).
  - `task test` — full suite (unit then e2e).
  - `task coverage` / `task coverage:unit` — see above.

### Changed

- `pr-preview.yml` now runs the full test suite (unit + e2e) in a `test` job
  that must pass before the `build` job starts.

- `CONTRIBUTING.md` — "Run the test suite" section expanded with the Task
  command table, Docker requirements, and a "Coverage reports" subsection.

---

## [0.1.1] — 2026-05-16

### Added

- **`manon infer` — DB-level namespace**: passing only a database name (no `.`)
  to `--namespace` now enumerates and infers every non-system collection in the
  database, writing one YAML file per collection.

- **`manon apply` — DB-level namespace**: passing only a database name (no `.`)
  to `--namespace` now processes every collection that has a matching
  `<name>/<name>.yaml` file under the `--masking-rules` directory.  Collections
  with no YAML file are skipped with a warning.

- **`manon apply --percent / -p`**: copy only a given percentage of each source
  collection (e.g. `--percent 10`).  The limit is `ceil(total × pct / 100)`,
  minimum 1 document.  Intended for ephemeral or short-lived environments.

- **`manon apply` — optional `--masking-rules`**: when `-c <config>` is given
  and `--masking-rules` is omitted, `manon` automatically uses
  `<BASE_DIR>/<PROJECT_DIR>/source/collections/` as the masking-rules
  directory, which is exactly where `manon infer` writes its output.  The
  typical project-based workflow now only requires passing `-c` and
  `--target-uri` to `apply`.

### Changed

- `--masking-rules` / `-m` on `manon apply` is now **optional** when `-c` is
  provided (see above).

### Documentation

- `CONTRIBUTING.md` added: guidance on extending `identifier.csv` /
  `identifier_category.csv` and proposing new masking methods.
- `CHANGELOG.md` added (this file).
- `docs/install.md`: added "Download a pre-built binary" section with platform
  table and `curl` one-liner, modelled on the mongo2pg USAGE.md.
- `docs/index.md` Quick Start: added "Whole database" and "Project-based
  workflow" subsections.
- `docs/how-to/README.md`: split infer and apply recipes into single-collection
  and whole-database subsections; added mixed-types / probability note.
- `docs/reference/README.md`: updated `manon apply` flag table to reflect
  optional `--masking-rules`, new `--percent`, and the auto-default from
  `-c`; added mixed-types / probability admonition under `manon infer`.
- `docs/tutorial/README.md`: Step 5 now shows the short `-c`-only form for
  `apply`; new "Ephemeral environments" section demonstrates `--percent`.

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

[Unreleased]: https://github.com/pmpetit/mongodbanonymizer/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/pmpetit/mongodbanonymizer/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/pmpetit/mongodbanonymizer/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pmpetit/mongodbanonymizer/releases/tag/v0.1.0

# manon — MongoDB Anonymizer

`manon` is a command-line tool that anonymizes MongoDB collections.  
It samples a collection to infer its schema and automatically detects
sensitive fields, then produces a deterministic, referentially-consistent
anonymized copy of the data that can be used safely in non-production
environments.

---

## Motivation

Development, testing, and analytics teams often need realistic data that
mirrors production volumes and shapes.  Sharing raw production dumps is
risky: they contain names, addresses, contact details, financial identifiers,
and other personal information protected by GDPR and similar regulations.

> **Static masking only.**  `manon` produces a new, permanently anonymized
> copy of your data.  It does **not** act as a proxy or rewrite queries
> on the fly (unlike dynamic-masking tools such as pgAnonymizer).  The
> source collection is never modified; the anonymized copy exists as a
> separate dataset and must be refreshed whenever the source changes.

`manon` solves this by:

- **Automatically detecting** sensitive fields through a curated dictionary
  of field-name patterns (`identifier.csv`) mapped to masking categories
  (`identifier_category.csv`).
- **Applying deterministic masking** — the same input always produces the
  same output, so foreign-key relationships and join patterns are preserved
  across collections.
- **Keeping the schema intact** — non-sensitive fields (numbers, dates,
  booleans, nested objects, arrays) pass through unchanged unless explicitly
  annotated.
- **Working entirely from the command line** with no external services or
  configuration servers required.

### Inspiration

`manon` stands on the shoulders of two existing open-source tools:

- **[pgAnonymizer](https://gitlab.com/dalibo/postgresql_anonymizer)** — the
  idea of driving anonymization from a plain-text dictionary of field-name
  patterns mapped to masking categories comes directly from pgAnonymizer.
  The bundled `identifier.csv` and `identifier_category.csv` files are
  adapted from its approach.
- **[mongo2pg](https://github.com/pmpetit/mongo2pg)** — the schema
  statistics format (field frequencies, type distributions, sampled values,
  distinct counts) is inspired by the schema-analysis output produced by
  mongo2pg.

---

## Installation

`manon` is a single Rust binary with no runtime dependencies.

---

### Download a pre-built binary

Go to the [Releases page](https://github.com/pmpetit/mongodbanonymizer/releases) and download the archive that matches your platform:

| Platform | File to download |
|---|---|
| Linux x86_64 | `manon-linux-x86_64` |
| Linux arm64 | `manon-linux-aarch64` |
| macOS Intel | `manon-macos-x86_64` |
| macOS Apple Silicon | `manon-macos-aarch64` |
| Windows x86_64 | `manon-windows-x86_64.exe` |
| Windows arm64 | `manon-windows-aarch64.exe` |

#### Linux / macOS

```bash
# Replace <version> and <platform> with your values, e.g. v0.1.0 and linux-x86_64
version="v0.1.0"
platform="linux-x86_64"
curl -fL "https://github.com/pmpetit/mongodbanonymizer/releases/download/${version}/manon-${platform}" \
  -o manon
chmod +x manon
sudo mv manon /usr/local/bin/
```

## Features

| Feature | Description |
|---|---|
| Schema inference | Samples a MongoDB collection and produces a YAML schema that describes field types, probabilities, and distinct-value counts |
| Automatic field detection | Maps field names (exact, suffix, and prefix) to masking categories using bundled CSV dictionaries |
| Deterministic masking | Same input → same output; referential integrity is preserved across collections |
| Nested document support | Masking rules are applied recursively into embedded objects and arrays of objects |
| Array-of-scalars support | Coordinate arrays and similar numeric arrays are noisy-masked element by element |
| Compound value samples | The `values` lists in the YAML schema (including compound documents) are fully anonymized |
| Manual rule editing | The generated YAML schema can be hand-edited to add, remove, or change masking rules before applying |
| Live collection apply | Streams documents from a source cluster, masks them in flight, and inserts into a target cluster |
| Batch insert | Documents are written in configurable batches (default 500) for efficient bulk loading |
| Project management | `init` creates a project directory structure and config file for repeatable runs |

### Masking methods

| Method | Behaviour |
|---|---|
| `PRESERVE_TOKEN` | Replaces each character class-by-class (uppercase↔uppercase, digit↔digit, separator kept); deterministic |
| `REDACT_ALPHANUMERIC` | Replaces every alphanumeric character with `X` / `0`; separators kept |
| `MASK_CONTACT_URI` | Anonymizes e-mail local parts, URL paths/queries, and phone digits |
| `MASK_NETWORK_ID` | Anonymizes IPv4, IPv6, and MAC addresses segment by segment |
| `GENERALIZE_LOCATION` | Truncates postal codes to the first 2–3 characters |
| `NOISY_DATE` | Shifts dates by a deterministic ±30-day noise |
| `NOISY_POSITION` | Adds deterministic ±0.009° (~1 km) noise to geographic coordinates |
| `STATIC_MAPPING` | Maps the value to one of five tokens (A–E) deterministically |
| `STATIC_BLOB_REPLACEMENT` | Replaces the value with the literal string `[REDACTED]` |

---

## CLI Usage

### `manon init` — create a project

```
manon init --project-cluster <base-dir> \
           --project-dbname  <project-name> \
           [--source-uri     mongodb://localhost:27017] \
           [--namespace      mydb.mycollection]
```

Creates `<base-dir>/<project-name>/` with a `config/` sub-directory
containing a `.conf` file that stores the URI, namespace, and sampling
parameters for subsequent commands.

---

### `manon infer` — sample a collection and infer its schema

```
manon infer --source-uri mongodb://localhost:27017 \
            --namespace  mydb.listings \
            --output-dir ./schema \
            [--number 1000 | --percent 10]
```

Or using a project config file:

```
manon infer -c ./myproject/config/myproject.conf
```

Writes `./schema/listings/listings.yaml` containing the inferred schema
with masking annotations for all recognised sensitive fields.

**Key options**

| Flag | Description |
|---|---|
| `-s, --source-uri` | MongoDB connection URI |
| `-n, --namespace` | `<db>.<collection>` or just `<db>` to infer all collections |
| `--number` | Number of documents to sample (default 1000) |
| `--percent` | Percentage of the collection to sample |
| `-o, --output-dir` | Directory to write YAML/JSON output |
| `-c, --config` | Path to a `.conf` file created by `manon init` |

---

### `manon mask` — refresh anonymized values in an existing schema file

```
manon mask <schema.yaml> [--output <out.yaml>]
```

Re-reads the CSV dictionaries, re-annotates any newly-recognised fields
(including fields that were missed in a previous run), and replaces all
`values` lists with freshly-anonymized data.  Useful after editing a schema
file manually or after updating the identifier dictionaries.

**Key options**

| Flag | Description |
|---|---|
| *(positional)* | Path to the YAML schema file to update |
| `-o, --output` | Write result to a new file instead of updating in-place |

---

### `manon apply` — anonymize a live collection

```
manon apply --source-uri  mongodb://prod:27017   \
            --namespace   mydb.listings          \
            --target-uri  mongodb://dev:27017    \
            --masking-rules ./schema/listings/listings.yaml \
            [--target-namespace mydb_anon.listings]
```

Streams every document from the source collection, applies the masking rules
defined in the YAML schema file, and inserts the anonymized documents into the
target collection in batches of 500.

**Key options**

| Flag | Description |
|---|---|
| `-s, --source-uri` | Source MongoDB URI |
| `-n, --namespace` | Source namespace (`<db>.<collection>`) |
| `-m, --masking-rules` | Path to the YAML schema file with masking annotations |
| `-t, --target-uri` | Target MongoDB URI |
| `--target-namespace` | Target namespace (defaults to the same as `--namespace`) |

---

## Typical workflow

```
# 1. Infer schema and masking rules from production
manon infer -s mongodb://prod:27017 -n mydb.users -o ./schema

# 2. Review / adjust ./schema/users/users.yaml as needed

# 3. Refresh anonymized value samples in the schema file
manon mask ./schema/users/users.yaml

# 4. Apply masking rules to populate a dev cluster
manon apply -s mongodb://prod:27017 \
            -n mydb.users \
            -t mongodb://dev:27017 \
            -m ./schema/users/users.yaml
```

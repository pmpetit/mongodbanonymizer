<p align="center">
  <img src="logo.png" alt="manon logo" width="500">
</p>

# manon — MongoDB Anonymizer

`manon` is a command-line tool that **anonymizes MongoDB collections**.  
It samples a collection to infer its schema, automatically detects sensitive
fields, and produces a deterministic, referentially-consistent anonymized copy
of the data that can be used safely in non-production environments.

!!! note "Static masking only"
    `manon` produces a new, permanently anonymized copy of your data.  It does
    **not** act as a proxy or rewrite queries on the fly (unlike dynamic-masking
    tools such as pgAnonymizer).  The source collection is never modified; the
    anonymized copy exists as a separate dataset and must be refreshed whenever
    the source changes.

---

## Why manon?

Development, testing, and analytics teams often need realistic data that mirrors
production volumes and shapes.  Sharing raw production dumps is risky: they
contain names, addresses, contact details, financial identifiers, and other
personal information protected by GDPR and similar regulations.

`manon` solves this by:

- **Automatically detecting** sensitive fields through a curated dictionary of
  field-name patterns (`identifier.csv`) mapped to masking categories
  (`identifier_category.csv`).
- **Applying deterministic masking** — the same input always produces the same
  output, so foreign-key relationships and join patterns are preserved across
  collections.
- **Keeping the schema intact** — non-sensitive fields pass through unchanged
  unless explicitly annotated.
- **Working entirely from the command line** with no external services or
  configuration servers required.

---

## Key Features

| Feature | Description |
|---|---|
| Schema inference | Samples a MongoDB collection and produces a YAML schema describing field types, probabilities, and distinct-value counts |
| Automatic field detection | Maps field names to masking categories using bundled CSV dictionaries |
| Deterministic masking | Same input → same output; referential integrity is preserved |
| Nested document support | Rules applied recursively into embedded objects and arrays |
| Array-of-scalars support | Coordinate arrays and numeric arrays noisy-masked element by element |
| Compound value samples | `values` lists in the YAML schema (including compound documents) are fully anonymized |
| Manual rule editing | Generated YAML can be hand-edited before applying |
| Live collection apply | Streams, masks, and bulk-inserts into a target cluster |
| Project management | `init` creates a project directory structure and config file |

---

## Inspiration

`manon` stands on the shoulders of two open-source tools:

- **[pgAnonymizer](https://gitlab.com/dalibo/postgresql_anonymizer)** — the
  identifier/category CSV-driven approach to detecting sensitive field names.
- **[mongo2pg](https://github.com/pmpetit/mongo2pg)** — the schema statistics
  format (field frequencies, type distributions, sampled values, distinct counts).

---

## Quick Start

### Single collection

```bash
# 1. Infer schema and masking rules from one collection
manon infer -s mongodb://prod:27017 -n mydb.users -o ./schema

# 2. Apply masking rules to populate a dev cluster
manon apply -s mongodb://prod:27017 \
            -n mydb.users \
            -t mongodb://dev:27017 \
            -m ./schema/users/users.yaml
```

### Whole database

Pass only the database name (no `.`) to process every collection at once:

```bash
# 1. Infer schema for all collections in mydb
manon infer -s mongodb://prod:27017 -n mydb -o ./schema

# 2. Apply masking rules for all collections in mydb
#    --masking-rules points to the directory written by infer
manon apply -s mongodb://prod:27017 \
            -n mydb \
            -t mongodb://dev:27017 \
            -m ./schema \
            --target-namespace mydb_anon
```

Collections that have no matching YAML file in `--masking-rules` are skipped
with a warning.

See the [Tutorial](tutorial/README.md) for a complete step-by-step walkthrough.

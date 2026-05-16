# Contributing

Contributions are welcome!  Here are the main ways you can help.

---

## Extend the field-detection dictionaries

The automatic detection of sensitive fields is driven by two CSV files in the
`data/` directory:

| File | Role |
|---|---|
| `data/identifier.csv` | Maps a **field name** (and locale) to a **category** |
| `data/identifier_category.csv` | Maps a **category** to a **masking method** |

### `identifier.csv` format

```
<locale> <TAB> <field_name> <TAB> <category>
```

Example:

```
en_US email email
en_US email_address email
fr_FR adresse_email email
```

If you encounter a field name that `manon infer` does not automatically
annotate, adding a row here (and opening a PR) benefits every user who has the
same field in their schema.

**Tips for good entries:**

- Use `en_US` as the locale for English/generic field names; add a
  language-specific locale (e.g. `fr_FR`, `de_DE`) for names that are unique to
  a language.
- Match the field name exactly as it appears in MongoDB (case-sensitive).
- Prefer re-using an existing category rather than inventing a new one.  Check
  `identifier_category.csv` for the current list.
- Add both the singular and plural forms if both are common
  (`address` / `addresses`).

### `identifier_category.csv` format

```
<category> <SPACE> <masking_method>
```

Example:

```
email MASK_CONTACT_URI
birth_date NOISY_DATE
```

If you think a category should use a different masking method, or if you need a
new category for a field type that has no good match, open an issue or a PR with
your proposal and a brief justification.

### Available masking methods

| Method | Best for |
|---|---|
| `PRESERVE_TOKEN` | Names, tokens — keeps structure, replaces characters |
| `REDACT_ALPHANUMERIC` | IDs, codes, numbers — replaces all alphanumeric chars with `X`/`0` |
| `MASK_CONTACT_URI` | Emails, phone numbers, URLs |
| `MASK_NETWORK_ID` | IPv4, IPv6, MAC addresses |
| `GENERALIZE_LOCATION` | Postal codes — truncates to first 2–3 characters |
| `NOISY_DATE` | Dates — shifts by a deterministic ±30-day noise |
| `NOISY_POSITION` | Geographic coordinates — adds ±0.009° (~1 km) noise |
| `STATIC_MAPPING` | Categorical values — maps to one of five tokens (A–E) |
| `STATIC_BLOB_REPLACEMENT` | Free-text, blobs — replaces with `[REDACTED]` |

---

## Propose new masking methods

If none of the existing methods fit your use case, open a GitHub issue with:

1. A description of the field type (what kind of data it holds).
2. An example of a raw value and the expected anonymized output.
3. Any determinism / referential-integrity requirements (should the same input
   always produce the same output?).

New masking methods are implemented in `src/masking.rs`.

---

## Development setup

### Prerequisites

- [Rust](https://rustup.rs/) stable toolchain
- [Docker](https://docs.docker.com/get-docker/) (for integration tests against
  a real MongoDB instance)

### Build

```bash
git clone https://github.com/pmpetit/mongodbanonymizer.git
cd mongodbanonymizer
cargo build
```

### Run the test suite

```bash
cargo test
```

### Try against real data

Start a local MongoDB instance and import the
[MongoDB sample datasets](https://github.com/neelabalan/mongodb-sample-dataset):

```bash
# Source container
docker run --name prod_mongodb -d \
  -p 2717:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=user \
  -e MONGO_INITDB_ROOT_PASSWORD=pass \
  mongodb/mongodb-community-server

# Import sample data
git clone https://github.com/neelabalan/mongodb-sample-dataset
cd mongodb-sample-dataset
docker cp . prod_mongodb:/tmp/mongodb-sample-dataset
docker exec -it prod_mongodb bash -c "
  cd /tmp/mongodb-sample-dataset &&
  bash start.sh 'mongodb://user:pass@localhost:27017/?authSource=admin'
"
```

Then exercise `manon` against the imported data:

```bash
URI='mongodb://user:pass@localhost:2717/?authSource=admin'

# Infer all collections in sample_airbnb
cargo run -- infer -s "$URI" -n sample_airbnb -o /tmp/schema

# Check the generated YAML
cat /tmp/schema/listingsAndReviews/listingsAndReviews.yaml
```

---

## Opening a pull request

1. Fork the repository and create a feature branch.
2. Keep changes focused — one concern per PR.
3. Run `cargo test` before pushing.
4. Describe **why** the change is needed in the PR description, not just what
   it does.

All contributions are subject to the project [LICENSE](LICENSE).

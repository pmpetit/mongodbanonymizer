# How-To Guides

Practical recipes for common `manon` tasks.

---

## How to set up a project

Use `manon init` to create a repeatable project directory:

```bash
manon init --project-cluster ./projects \
           --project-dbname  airbnb \
           --source-uri      mongodb://localhost:27017 \
           --namespace       airbnb.listings
```

This creates:

```
projects/airbnb/
  config/
    airbnb.conf    ← stores URI, namespace, and sampling parameters
  source/
    collections/   ← placeholder for source snapshots
```

You can then pass `-c projects/airbnb/config/airbnb.conf` to subsequent
commands instead of repeating all flags.

---

## How to infer a schema

### Single collection

```bash
manon infer --source-uri mongodb://localhost:27017 \
            --namespace  airbnb.listings \
            --output-dir ./schema \
            --number     2000
```

`--number` controls how many documents are sampled (default 1000).  Use
`--percent` to sample a percentage of the collection instead.

The output is a YAML file at `./schema/listings/listings.yaml`.

### All collections in a database

Omit the `.collection` part of `--namespace` to infer every collection in the
database in one go:

```bash
manon infer --source-uri mongodb://localhost:27017 \
            --namespace  airbnb \
            --output-dir ./schema
```

`manon` lists all non-system collections in `airbnb` and writes one YAML file
per collection under `./schema/<collection>/`.

---

## How to review and edit masking rules

Open the generated YAML file and look for `masking:` blocks:

```yaml
host_name:
  types:
    String:
      masking:
        enabled: true
        method: PRESERVE_TOKEN
      values:
        - Txkmf          # ← already anonymized
        - Bqz
        - Wkrpn Hdjx
```

!!! note "About `values`"
    Each field stores up to **20 reservoir-sampled values** collected during
    `manon infer`.  When a masking rule is enabled the values shown are
    **already anonymized** — they reflect what the output will look like after
    `manon apply` runs.  Use them as a quick sanity check to make sure the
    chosen masking method produces realistic-looking results before applying
    rules to the real collection.

    Run `manon mask` at any time to refresh these samples after changing a
    method or editing the YAML by hand.

You can:

- Change `method` to a different [masking method](../masking-methods/README.md).
- Set `enabled: false` to skip masking for a field.
- Add a `masking:` block to a field that was not automatically detected.

!!! note "Mixed types"
    Because MongoDB is schemaless, the same field can contain different BSON
    types in different documents.  The YAML lists each observed type under
    `types:` with its own `probability` (how often that type appears among all
    sampled occurrences of the field).  You can enable masking on one type and
    leave another untouched — useful for legacy fields that mix strings and
    numbers.

After editing, run `manon mask` to refresh the anonymized `values` samples:

```bash
manon mask ./schema/listings/listings.yaml
```

---

## How to anonymize a live collection

### Single collection

```bash
manon apply --source-uri       mongodb://prod:27017 \
            --namespace        airbnb.listings \
            --masking-rules    ./schema/listings/listings.yaml \
            --target-uri       mongodb://dev:27017 \
            --target-namespace airbnb_anon.listings
```

Documents are read from the source, masked in memory in batches of 500, and
bulk-inserted into the target.  The source is never modified.

### All collections in a database

Pass only the database name (no `.`) to `--namespace` and point
`--masking-rules` to the directory produced by `manon infer`:

```bash
manon apply --source-uri       mongodb://prod:27017 \
            --namespace        airbnb \
            --masking-rules    ./schema \
            --target-uri       mongodb://dev:27017 \
            --target-namespace airbnb_anon
```

Each collection that has a matching `<name>/<name>.yaml` file under `./schema`
is processed in turn.  Collections with no YAML file are skipped with a
warning.  Add `--percent 10` to limit the copy to 10 % of each collection.

---

## How to refresh an anonymized dataset

Re-run `manon apply` whenever the source data changes.  Drop or truncate the
target collection first if you want a clean replacement:

```bash
# Using mongosh to drop the target collection first
mongosh mongodb://dev:27017 --eval \
  "db.getSiblingDB('airbnb_anon').listings.drop()"

manon apply -s mongodb://prod:27017 \
            -n airbnb.listings \
            -t mongodb://dev:27017 \
            -m ./schema/listings/listings.yaml
```

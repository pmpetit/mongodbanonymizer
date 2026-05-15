# Tutorial — Anonymizing an AirBnB-style Dataset

This walkthrough takes you from a raw MongoDB collection to a fully anonymized
copy in a development cluster using all four `manon` commands.

!!! tip "Using a config file"
    Every parameter shown in this tutorial (`--source-uri`, `--namespace`,
    `--number`, `--percent`) can be stored in a project config file created by
    `manon init`.  Once the file exists, pass `-c <path-to-config>` instead of
    repeating all flags on every command.  Command-line arguments always take
    priority over values in the config file.

---

## Prerequisites

- `manon` installed and on your `PATH` (see [Installation](../install.md))
- A source MongoDB instance with a collection to anonymize
- A target MongoDB instance for the anonymized data (can be the same instance
  with a different database name)

---

## Step 1 — Create a project

```bash
manon init \
  --project-cluster ./projects \
  --project-dbname  airbnb \
  --source-uri      mongodb://localhost:27017 \
  --namespace       airbnb.listings
```

This creates:

```
projects/airbnb/
  config/
    airbnb.conf
  source/
    collections/
```

The generated `airbnb.conf` stores `URI`, `NAMESPACE`, and optional sampling
parameters (`NUMBER` / `PERCENT`).  You can edit it directly to change these
defaults at any time.

---

## Step 2 — Infer the schema

Sample 2 000 documents and write the schema with automatic masking annotations:

```bash
manon infer \
  --source-uri mongodb://localhost:27017 \
  --namespace  airbnb.listings \
  --output-dir ./schema \
  --number     2000
```

Or, using the config file created in Step 1 (with `--output-dir` still
required unless you rely on the default path inside the project folder):

```bash
manon infer -c ./projects/airbnb/config/airbnb.conf \
            --output-dir ./schema
```

Output: `./schema/listings/listings.yaml`

Open the file and check the detected fields.  You will see blocks like:

```yaml
host_name:
  count: 1987
  types:
    String:
      count: 1987
      prob: 0.9935
      masking:
        enabled: true
        method: PRESERVE_TOKEN
      values:
        - Alice
        - Bob
        - ...
```

Fields whose names were not recognised by the CSV dictionaries will have no
`masking:` block.  Add one manually if needed.

---

## Step 3 — Review and adjust rules

Edit `./schema/listings/listings.yaml` as required:

- Change a `method` to better fit the data (e.g. `NOISY_DATE` for a timestamp
  field that was detected as `PRESERVE_TOKEN`).
- Add `masking: {enabled: true, method: STATIC_BLOB_REPLACEMENT}` to a free-text
  comment field.
- Set `enabled: false` on a field that does not need masking.

See [Masking Methods](../masking-methods/README.md) for a full description of
each method.

---

## Step 4 — Refresh value samples

After editing, refresh the `values` lists in the schema file so the samples
reflect the new rules:

```bash
manon mask ./schema/listings/listings.yaml
```

This re-annotates all fields from the CSV dictionaries and re-applies masking
to every sampled value.  The file is updated in place.

---

## Step 5 — Apply to the target cluster

```bash
manon apply \
  --source-uri       mongodb://localhost:27017 \
  --namespace        airbnb.listings \
  --masking-rules    ./schema/listings/listings.yaml \
  --target-uri       mongodb://localhost:27017 \
  --target-namespace airbnb_anon.listings
```

With the config file supplying `--source-uri` and `--namespace`:

```bash
manon apply -c ./projects/airbnb/config/airbnb.conf \
            --masking-rules    ./schema/listings/listings.yaml \
            --target-uri       mongodb://localhost:27017 \
            --target-namespace airbnb_anon.listings
```

`manon` will:

1. Open a cursor on `airbnb.listings`.
2. Read documents in batches of 500.
3. Apply the masking rules from the YAML file to each document in memory.
4. Bulk-insert the anonymized batch into `airbnb_anon.listings`.

The source collection is never modified.

---

## Repeat for other collections

Run steps 2–5 for each additional collection, changing `--namespace` and
`--output-dir` accordingly:

```bash
# Infer schema for the reviews collection
manon infer -s mongodb://localhost:27017 \
            -n airbnb.reviews \
            -o ./schema

# Apply masking
manon apply  -s mongodb://localhost:27017 \
             -n airbnb.reviews \
             -t mongodb://localhost:27017 \
             --target-namespace airbnb_anon.reviews \
             -m ./schema/reviews/reviews.yaml
```

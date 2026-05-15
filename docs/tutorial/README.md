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
- [Docker](https://docs.docker.com/get-docker/) installed and running
- `git` and `bash`

### Start two MongoDB instances

One container acts as the **production** source, the other as the **development**
target that will receive the anonymized data.

```bash
# Source — production (port 2717)
docker run --name prod_mongodb -d \
  -p 2717:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=user \
  -e MONGO_INITDB_ROOT_PASSWORD=pass \
  mongodb/mongodb-community-server

# Target — development (port 2718)
docker run --name dev_mongodb -d \
  -p 2718:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=user \
  -e MONGO_INITDB_ROOT_PASSWORD=pass \
  mongodb/mongodb-community-server
```

Verify both are running:

```bash
docker ps | grep mongodb
```

### Import the MongoDB sample datasets into the production instance

Clone the sample dataset repository and import it into `prod_mongodb`:

```bash
git clone https://github.com/neelabalan/mongodb-sample-dataset
cd mongodb-sample-dataset
```

If `mongoimport` is available locally:

```bash
bash start.sh 'mongodb://user:pass@localhost:2717/?authSource=admin'
```

Otherwise run the import from inside the container:

```bash
# Copy the datasets into the container
docker cp . prod_mongodb:/tmp/mongodb-sample-dataset

# Run the import from inside the container
docker exec -it prod_mongodb bash -c "
  cd /tmp/mongodb-sample-dataset &&
  bash start.sh 'mongodb://user:pass@localhost:27017/?authSource=admin'
"
```

The tutorial below uses the `sample_airbnb.listingsAndReviews` collection.
Substitute any other namespace if you prefer a different dataset.

---

## Step 1 — Create a project

```bash
manon init \
  --project-cluster ./projects \
  --project-dbname  airbnb \
  --source-uri      'mongodb://user:pass@localhost:2717/?authSource=admin' \
  --namespace       sample_airbnb.listingsAndReviews
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
  --source-uri 'mongodb://user:pass@localhost:2717/?authSource=admin' \
  --namespace  sample_airbnb.listingsAndReviews \
  --output-dir ./schema \
  --number     2000
```

Or, using the config file created in Step 1 (with `--output-dir` still
required unless you rely on the default path inside the project folder):

```bash
manon infer -c ./projects/airbnb/config/airbnb.conf \
            --output-dir ./schema
```

Output: `./schema/listingsAndReviews/listingsAndReviews.yaml`

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
  --source-uri       'mongodb://user:pass@localhost:2717/?authSource=admin' \
  --namespace        sample_airbnb.listingsAndReviews \
  --masking-rules    ./schema/listingsAndReviews/listingsAndReviews.yaml \
  --target-uri       'mongodb://user:pass@localhost:2718/?authSource=admin' \
  --target-namespace sample_airbnb_anon.listingsAndReviews
```

With the config file supplying `--source-uri` and `--namespace`:

```bash
manon apply -c ./projects/airbnb/config/airbnb.conf \
            --masking-rules    ./schema/listingsAndReviews/listingsAndReviews.yaml \
            --target-uri       'mongodb://user:pass@localhost:2718/?authSource=admin' \
            --target-namespace sample_airbnb_anon.listingsAndReviews
```

`manon` will:

1. Open a cursor on `sample_airbnb.listingsAndReviews`.
2. Read documents in batches of 500.
3. Apply the masking rules from the YAML file to each document in memory.
4. Bulk-insert the anonymized batch into `sample_airbnb_anon.listingsAndReviews`.

The source collection is never modified.

---

## Repeat for other collections

Run steps 2–5 for each additional collection, changing `--namespace` and
`--output-dir` accordingly:

```bash
SOURCE='mongodb://user:pass@localhost:2717/?authSource=admin'
TARGET='mongodb://user:pass@localhost:2718/?authSource=admin'

# Infer schema for the reviews collection
manon infer -s "$SOURCE" \
            -n sample_airbnb.reviews \
            -o ./schema

# Apply masking
manon apply  -s "$SOURCE" \
             -n sample_airbnb.reviews \
             -t "$TARGET" \
             --target-namespace sample_airbnb_anon.reviews \
             -m ./schema/reviews/reviews.yaml
```

### Tear down

```bash
docker stop prod_mongodb dev_mongodb
docker rm   prod_mongodb dev_mongodb
```

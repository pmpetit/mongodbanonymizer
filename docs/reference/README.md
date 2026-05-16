# CLI Reference

`manon` exposes four subcommands.

---

## `manon init`

Creates a project directory structure and a `.conf` file for repeatable runs.

```
manon init --project-cluster <base-dir>
           --project-dbname  <project-name>
           [--source-uri     <mongodb-uri>]
           [--namespace      <db.collection>]
```

| Flag | Short | Description |
|---|---|---|
| `--project-cluster` | | Base directory where the project folder will be created |
| `--project-dbname` | | Name of the project (becomes the sub-directory name) |
| `--source-uri` | `-s` | MongoDB connection URI (stored in the config file) |
| `--namespace` | `-n` | Default namespace (`db.collection`) stored in the config file |

**Output** — `<base-dir>/<project-name>/config/<project-name>.conf`

---

## `manon infer`

Samples a collection, infers its schema, annotates sensitive fields, and
writes a YAML schema file.

```
manon infer --source-uri  <mongodb-uri>
            --namespace   <db.collection>
            --output-dir  <dir>
            [--number <N> | --percent <P>]
            [--config <path-to-.conf>]
```

| Flag | Short | Description |
|---|---|---|
| `--source-uri` | `-s` | MongoDB connection URI |
| `--namespace` | `-n` | `<db>.<collection>` to infer |
| `--output-dir` | `-o` | Directory to write the YAML schema file |
| `--number` | | Number of documents to sample (default 1000) |
| `--percent` | | Percentage of the collection to sample |
| `--config` | `-c` | Path to a `.conf` file created by `manon init` |

**Output** — `<output-dir>/<collection>/<collection>.yaml`

!!! note
    `--source-uri` is required unless a `--config` file is provided that
    contains a `source_uri` value.

---

## `manon mask`

Re-reads the CSV dictionaries, re-annotates fields, and refreshes the
anonymized `values` samples in an existing schema file.

```
manon mask <schema.yaml> [--output <out.yaml>]
```

| Argument / Flag | Short | Description |
|---|---|---|
| *(positional)* | | Path to the YAML schema file to update |
| `--output` | `-o` | Write result to a new file instead of updating in-place |

Use this command after:

- Manually editing the YAML to change masking rules.
- Updating `identifier.csv` or `identifier_category.csv`.
- A `manon infer` run that produced stale samples.

---

## `manon apply`

Streams documents from a source MongoDB collection, masks them according to a
YAML schema file, and bulk-inserts them into a target collection.

```
manon apply --source-uri      <mongodb-uri>
            --namespace       <db.collection>
            --masking-rules   <schema.yaml>
            --target-uri      <mongodb-uri>
            [--target-namespace <db.collection>]
```

| Flag | Short | Description |
|---|---|---|
| `--source-uri` | `-s` | Source MongoDB connection URI |
| `--namespace` | `-n` | Source namespace (`db.collection` or just `db` for all collections) |
| `--masking-rules` | `-m` | Path to the YAML schema file, or a directory of per-collection YAML files (DB-level apply) |
| `--target-uri` | `-t` | Target MongoDB connection URI |
| `--target-namespace` | | Target namespace or DB name (defaults to the same as `--namespace`) |
| `--percent` | `-p` | Copy only this percentage of each source collection (e.g. `10` for 10%). Useful for ephemeral environments. |
| `--config` | `-c` | Path to a `.conf` file created by `manon init` |

Documents are processed in batches of **500** and written with `insert_many`.
The source collection is **never modified**.

!!! tip "Ephemeral environments"
    Pass `--percent <N>` to limit the number of documents copied per collection.
    For example, `--percent 10` copies roughly 10 % of each collection, which is
    enough for smoke tests without filling a short-lived environment with a full
    production dataset.

    ```bash
    manon apply -s mongodb://prod:27017 -n mydb \
                -m source/collections/ \
                -t mongodb://dev:27017 \
                --target-namespace mydb_anon \
                --percent 10
    ```

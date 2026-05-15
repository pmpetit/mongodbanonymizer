# Installation

`manon` is a single Rust binary.  You can build it from source or (once
releases are published) download a pre-built binary.

---

## Build from Source

### Prerequisites

| Tool | Minimum version |
|---|---|
| Rust toolchain | 1.88.0 |
| Cargo | bundled with Rust |
| A running MongoDB instance | 4.4+ (for `infer` / `apply`) |

Install Rust via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Clone and build

```bash
git clone https://github.com/pmpetit/mongodbanonymizer.git
cd mongodbanonymizer
cargo build --release
```

The binary is written to `target/release/manon` (the binary is named `manon`
even though the crate is called `mongodbanonymizer`).

### Add to PATH

```bash
# Option 1 – copy to a directory already on your PATH
sudo cp target/release/manon /usr/local/bin/

# Option 2 – install via Cargo
cargo install --path .
```

Verify the installation:

```bash
manon --version
```

---

## Data files

`manon` ships with two CSV dictionaries that drive automatic field detection.
They are read at runtime from the working directory (or the path set by the
`--data-dir` flag if supported by your build):

| File | Purpose |
|---|---|
| `data/identifier.csv` | Maps field-name patterns (locale, name, category) |
| `data/identifier_category.csv` | Maps masking categories to masking methods |

These files are included in the repository under `data/`.  When running
`manon` from a directory other than the repository root, copy the `data/`
folder alongside the binary or point `--data-dir` to its location.

---

## Docker (coming soon)

A `Dockerfile` and `docker-compose.yml` will be provided in a future release
for containerised use.

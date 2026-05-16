# Installation

`manon` is a single Rust binary with no runtime dependencies.

---

## Download a pre-built binary

Go to the [Releases page](https://github.com/pmpetit/mongodbanonymizer/releases) and download the archive that matches your platform:

| Platform | File to download |
|---|---|
| Linux x86_64 | `manon-linux-x86_64` |
| Linux arm64 | `manon-linux-aarch64` |
| macOS Intel | `manon-macos-x86_64` |
| macOS Apple Silicon | `manon-macos-aarch64` |
| Windows x86_64 | `manon-windows-x86_64.exe` |
| Windows arm64 | `manon-windows-aarch64.exe` |

### Linux / macOS

```bash
# Replace <version> and <platform> with your values, e.g. v0.1.0 and linux-x86_64
version="v0.1.0"
platform="linux-x86_64"
curl -fL "https://github.com/pmpetit/mongodbanonymizer/releases/download/${version}/manon-${platform}" \
  -o manon
chmod +x manon
sudo mv manon /usr/local/bin/
```

### Windows

Download the `.exe`, optionally rename it to `manon.exe`, and place it in a
directory that is on your `PATH`.

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

The binary is written to `target/release/manon`.

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

## Docker (coming soon)

A `Dockerfile` and `docker-compose.yml` will be provided in a future release
for containerised use.

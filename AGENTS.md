# Agent Notes

## Build

```bash
cargo build --release
```

## Test

```bash
cargo test
```

## Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Run

```bash
./target/release/ghn
```

## Requirements

- Rust toolchain (`cargo`)
- GitHub CLI (`gh`) authenticated or `GITHUB_TOKEN` set

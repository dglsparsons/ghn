# Agent Notes

## Build

```bash
cargo build --release
```

## Test

When tests are appropriate, run them yourself and report the results rather than suggesting the user run them.

```bash
cargo test
```

## Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Format

```bash
cargo fmt --all
```

## Run

```bash
./target/release/ghn
```

## Runtime Gotchas

- Use `tokio::time::MissedTickBehavior::Skip` for long-lived intervals in this app to avoid catch-up bursts after sleep/stalls.

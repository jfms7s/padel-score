# padel-score

A pure Rust padel scoring engine (`padel-scoring`): rules, match state and replay from an event log.
No clock, threads, randomness or I/O, so it also builds for `wasm32-unknown-unknown`.

## Develop

```sh
cargo test
cargo build -p padel-scoring --target wasm32-unknown-unknown
```

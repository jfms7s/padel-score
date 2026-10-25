# padel-score

- Docs live in the Obsidian vault at `personal/projects/padel-score/`, not in this repo.
- Use Conventional Commits (`type(scope): description`); CI checks every subject.
- The scoring crate must never panic (bad input returns `Err`) and must stay wasm-clean:
  it builds for `wasm32-unknown-unknown` with no extra features and uses no `std::time`, threads, `rand` or filesystem.
- Before committing: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build -p padel-scoring --target wasm32-unknown-unknown`.

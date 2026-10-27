# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rules

- Docs live in the Obsidian vault at `personal/projects/padel-score/`, not in this repo.
- Use Conventional Commits (`type(scope): description`); CI checks every subject (`scripts/ci/commit-subjects.sh`).
- The scoring crate must never panic (bad input returns `Err`) and must stay wasm-clean:
  it builds for `wasm32-unknown-unknown` with no extra features and uses no `std::time`, threads, `rand` or filesystem.
- The workspace denies `unwrap_used`, `expect_used`, `panic` and `indexing_slicing` (clippy, `Cargo.toml`); tests opt out with a file-level `#![allow(...)]`.
- Before committing: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo build -p padel-scoring --target wasm32-unknown-unknown --locked && cargo build -p padel-scoring-wasm --target wasm32-unknown-unknown --locked`.
  If `web/` changed, also run in `web/`: `npm run check && npm run format:check && npm test && npm run build`.

## Commands

Rust toolchain is pinned in `rust-toolchain.toml` (keep CI's `toolchain:` in sync).

```sh
cargo test -p padel-scoring --test tiebreak            # one integration test file
cargo test -p padel-score-server --test api some_name  # one test by name filter
```

Web (SvelteKit SPA, `web/`). It needs the wasm engine bindings first, which are git-ignored:

```sh
cargo install wasm-bindgen-cli --version <pin in crates/scoring-wasm/Cargo.toml> --locked
bash scripts/build-wasm.sh          # writes web/src/lib/wasm/
cd web && npm ci && npm run dev     # proxies /api to the server on :8080
npx vitest --run src/lib/queue.test.ts   # one test file
```

Server: `cargo run -p padel-score-server` (env: `PORT`, `DATA_DIR`/`DATABASE_URL`, `SECURE_COOKIES=false` for plain-HTTP local use, `SETUP_TOKEN`, `TRUSTED_PROXIES`, `PUBLIC_ORIGIN`, `LOG_LEVEL`; see `crates/server/src/config.rs`). It embeds `web/build/`; `build.rs` writes a placeholder page if the web app isn't built, so Rust builds and tests never need Node. Full image: see README (`docker build`).

Watch (Wear OS, Kotlin, JDK 21 + Android SDK): `cd watch && ./gradlew :app:assembleDebug` (tests: `./gradlew :app:testDebugUnitTest`).

## Architecture

One scoring engine, four consumers:

- **`crates/scoring` (`padel-scoring`)** — pure rules engine. `replay(format, start, events)` folds an event log (`Point`, `TimeUp`) through `MatchState` into a `Score`. There is no undo event: callers undo by dropping the event from the log and replaying. Also validates/expands typed final scores (`events_for_final`).
- **`crates/scoring-wasm`** — `wasm-bindgen` JSON-in/JSON-out wrapper (`replay`, `validate_final`) used by the browser to score offline (`web/src/lib/engine.ts`). The wasm-bindgen crate version is pinned with `=` and must match the CLI.
- **`crates/server` (`padel-score-server`)** — axum + SQLite (sqlx, migrations in `crates/server/migrations/`, runtime queries, no offline `.sqlx` data). The match is stored as an event log; every view is a fresh `replay`. `commands.rs` owns the command semantics (live points, offline batches against a base revision, undo, replace, typed-final locking, `client_id` idempotency where a resend returns the saved response byte-for-byte). `hub.rs` fans changes out to SSE streams per match, per owner and per event. `competitions/` holds tournaments (brackets) and non-stop/americano formats; `accounts*`, `auth`, `device`, `linking`, `players`, `privacy` cover accounts, watch device sign-in and player linking. `*_api.rs` are the HTTP layers over these.
- **`web/`** — SvelteKit with `adapter-static` (pure SPA, `fallback: index.html`), Svelte 5 runes. `src/lib/` holds the logic with colocated tests; `queue.ts` is the offline tap queue that batches to the server on reconnect and surfaces conflicts instead of merging. `*.svelte.test.ts` run in a browser-like vitest project, other tests in node.
- **`watch/`** — Wear OS app with its own Kotlin port of the engine for local matches.

### Cross-language contracts (fixtures)

Wire formats are pinned by committed fixtures shared across languages, so changing a serialized type usually means regenerating fixtures and fixing the other side:

- `crates/scoring/tests/fixtures/engine-vectors.json` — engine outputs the watch's Kotlin engine must reproduce. Regenerate: `UPDATE_ENGINE_VECTORS=1 cargo test -p padel-scoring --test engine_vectors`.
- `web/src/lib/fixtures/*.json` — recorded server responses; the web checks them against strict TS guards (`fixtures.test.ts`, `guards.ts`) and the watch's tests decode them too. `tournament-labels.json` regenerates with `UPDATE_FIXTURES=1 cargo test -p padel-score-server --test tournament`.

## CI and releases

`.github/workflows/ci.yml` runs the Rust checks (tests under `cargo llvm-cov`), the web checks, commit subjects, `cargo-deny` advisories and an eng-metrics-lab quality report. `watch.yml` runs only when `watch/`, the web fixtures or the engine vectors change. Pushing a `vX.Y.Z` tag on `main` runs `release.yml`, which reruns CI and publishes a multi-arch image to `ghcr.io/jfms7s/padel-score`.

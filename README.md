# padel-score

A pure Rust padel scoring engine (`padel-scoring`): rules, match state and replay from an event log.
No clock, threads, randomness or I/O, so it also builds for `wasm32-unknown-unknown`.

## Develop

```sh
cargo test
cargo build -p padel-scoring --target wasm32-unknown-unknown
```

## Run locally

The image holds the server with the web app embedded in it. The SQLite database lives in `/data`.

```sh
docker build --build-arg APP_VERSION=v0.0.0-dev -t padel-score:dev .
docker run --rm -p 8080:8080 -v padel-score-dev:/data padel-score:dev
```

Then open <http://localhost:8080>. With `podman build`, add `--format docker`, or the `HEALTHCHECK` is dropped.

## Releases

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`. If the tagged commit is on `main` and every CI check passes on it, the workflow builds `ghcr.io/jfms7s/padel-score:vX.Y.Z`
for `linux/amd64` and `linux/arm64` (cross-compiled, no emulation) and pushes it with signed build provenance:

```sh
gh attestation verify oci://ghcr.io/jfms7s/padel-score:vX.Y.Z --repo jfms7s/padel-score
```

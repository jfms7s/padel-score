# One image: the Rust server with the SvelteKit app (and its WASM scoring engine) embedded in it.
#
# The wasm, web and server stages run on the build machine's own platform and the server is
# cross-compiled to static musl binaries with cargo-zigbuild, so a multi-arch build
# (linux/amd64 + linux/arm64) needs no emulation: only the final COPY is per-platform.
#
#   docker buildx build --platform linux/amd64,linux/arm64 -t padel-score .
#
# Base images are fully qualified (docker.io/...) so podman/buildkit never ask.
# rust-toolchain.toml is deliberately not copied: the rust:1.97 image already is that toolchain,
# and the file would make rustup fetch components the build doesn't need.

# The scoring engine compiled to WASM, with its JS glue (scripts/build-wasm.sh).
FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.97-bookworm AS wasm
# Must match the wasm-bindgen pin in crates/scoring-wasm/Cargo.toml (build-wasm.sh checks).
ARG WASM_BINDGEN_VERSION=0.2.129
RUN set -eu; \
    rustup target add wasm32-unknown-unknown; \
    cargo install --locked wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}"
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/scoring ./crates/scoring
COPY crates/scoring-wasm ./crates/scoring-wasm
# The server is a workspace member, so its manifest must load; its code isn't needed here.
COPY crates/server/Cargo.toml crates/server/build.rs ./crates/server/
RUN mkdir -p crates/server/src && echo 'fn main() {}' > crates/server/src/main.rs && touch crates/server/src/lib.rs
COPY scripts/build-wasm.sh ./scripts/
RUN bash scripts/build-wasm.sh

FROM --platform=$BUILDPLATFORM docker.io/library/node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json web/.npmrc ./
RUN npm ci
COPY web/ ./
COPY --from=wasm /src/web/src/lib/wasm ./src/lib/wasm
RUN npm run build \
 && grep -q '_app/' build/index.html

FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.97-bookworm AS server
ARG ZIG_VERSION=0.13.0
ARG ZIGBUILD_VERSION=0.20.1
# Zig comes from PyPI (the ziglang package), which cargo-zigbuild finds on its own.
RUN set -eu; \
    apt-get update; apt-get install -y --no-install-recommends python3-pip; rm -rf /var/lib/apt/lists/*; \
    pip3 install --no-cache-dir --break-system-packages "ziglang==${ZIG_VERSION}"; \
    cargo install --locked cargo-zigbuild --version "${ZIGBUILD_VERSION}"; \
    rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
WORKDIR /src
# Dependencies first, against stub sources, so source edits don't rebuild them.
COPY Cargo.toml Cargo.lock ./
COPY crates/scoring/Cargo.toml ./crates/scoring/
COPY crates/scoring-wasm/Cargo.toml ./crates/scoring-wasm/
COPY crates/server/Cargo.toml crates/server/build.rs ./crates/server/
RUN mkdir -p crates/scoring/src crates/scoring-wasm/src crates/server/src \
 && touch crates/scoring/src/lib.rs crates/scoring-wasm/src/lib.rs crates/server/src/lib.rs \
 && echo 'fn main() {}' > crates/server/src/main.rs \
 && for t in x86_64 aarch64; do cargo zigbuild -p padel-score-server --release --locked --target "$t-unknown-linux-musl"; done \
 && rm -rf crates/scoring/src crates/scoring-wasm/src crates/server/src web/build
COPY crates/scoring ./crates/scoring
COPY crates/scoring-wasm ./crates/scoring-wasm
COPY crates/server ./crates/server
# The real web build, before compiling: build.rs only writes a placeholder when it's missing.
COPY --from=web /web/build ./web/build
RUN grep -q '_app/' web/build/index.html \
 || { echo "web/build/index.html is not the real SvelteKit build" >&2; exit 1; }
RUN touch crates/scoring/src/lib.rs crates/server/src/main.rs crates/server/src/lib.rs \
 && for t in x86_64 aarch64; do cargo zigbuild -p padel-score-server --release --locked --target "$t-unknown-linux-musl"; done \
 && mkdir -p /out/amd64 /out/arm64 /out/data \
 && cp target/x86_64-unknown-linux-musl/release/padel-score /out/amd64/ \
 && cp target/aarch64-unknown-linux-musl/release/padel-score /out/arm64/

# distroless/static: CA certificates and a nonroot user, nothing else.
FROM gcr.io/distroless/static-debian12:nonroot AS runtime
ARG TARGETARCH
# Read by the server at runtime (crates/server/src/config.rs).
ARG APP_VERSION=
ARG GIT_COMMIT=
ENV APP_VERSION=$APP_VERSION GIT_COMMIT=$GIT_COMMIT \
    DATA_DIR=/data PORT=8080
COPY --from=server /out/${TARGETARCH}/padel-score /usr/local/bin/padel-score
# uid 65532 (nonroot) owns /data, so a fresh named volume mounted there is writable.
# Mount a volume at /data for the SQLite file.
COPY --from=server --chown=65532:65532 /out/data /data
USER 65532:65532
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 CMD ["/usr/local/bin/padel-score", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/padel-score"]

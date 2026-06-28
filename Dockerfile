# Multi-stage build for iroh-tunnel.
#
# Build stage uses rust:1.91-slim; the runtime stage is distroless so the final
# image carries only the statically-resolvable binary + its shared libs.

# ---- build stage ----
FROM rust:1.91-slim AS builder

# Build dependencies first (cached layer): copy only the manifest so dependency
# compilation is reused unless Cargo.toml/Cargo.lock change.
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs && cargo build --release \
    && rm -rf src target/release/deps/iroh_tunnel*

# Now copy the real source and build the binary.
COPY src/ ./src/
RUN cargo build --release

# ---- runtime stage ----
# cc-debian12 carries the glibc + CA certs the release binary (built against
# rust:1.91-slim, glibc) needs to dial TLS relays.
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /app/target/release/iroh-tunnel /usr/local/bin/iroh-tunnel

ENTRYPOINT ["iroh-tunnel"]

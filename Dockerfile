# Multi-stage build for iroh-tunnel.
#
# Build stage uses rust:1.91-slim; the runtime stage is distroless so the final
# image carries only the statically-resolvable binary + its shared libs.
#
# No dependency-cache dummy layer: the crate's build.rs (APP_VERSION via
# vergen-gitcl) and the lib+bin target pair made the "build with dummy
# sources, overwrite, rebuild" trick fragile — the post-swap cargo run
# failed with APP_VERSION undefined at compile time. Source changes
# therefore recompile dependencies; release images use
# Dockerfile.goreleaser (prebuilt binaries) and are unaffected.

# ---- build stage ----
FROM rust:1.91-slim-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# ---- runtime stage ----
# cc-debian12 carries the glibc + CA certs the release binary (built against
# rust:1.91-slim, glibc) needs to dial TLS relays.
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /app/target/release/iroh-tunnel /usr/local/bin/iroh-tunnel

ENTRYPOINT ["iroh-tunnel"]

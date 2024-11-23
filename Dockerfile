FROM rust:1.82.0-slim-bookworm as base
RUN cargo install cargo-chef --version 0.1.68

FROM base AS planner
WORKDIR /tainter
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM base AS builder
WORKDIR /tainter
COPY --from=planner /tainter/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin tainter

FROM debian:bookworm-20241016-slim AS runtime
WORKDIR /tainter
COPY --from=builder /tainter/target/release/tainter /usr/local/bin
RUN groupadd -g 10001 tainter && \
   useradd -u 10000 -g tainter tainter \
   && chown -R tainter:tainter /usr/local/bin/tainter
USER tainter:tainter

LABEL org.opencontainers.image.title="tainter"
LABEL org.opencontainers.image.source="https://github.com/LasseHels/tainter"
LABEL org.opencontainers.image.authors="Lasse Canth Hels <lasse@hels.dk>"

ENTRYPOINT ["/usr/local/bin/tainter"]

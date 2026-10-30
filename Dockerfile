FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/canteen-mcp /usr/local/bin/canteen-mcp

FROM gcr.io/distroless/cc-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/zahraabouzeid/canteen-mcp"
COPY --from=build /usr/local/bin/canteen-mcp /usr/local/bin/canteen-mcp
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=5s CMD ["/usr/local/bin/canteen-mcp", "health"]
ENTRYPOINT ["/usr/local/bin/canteen-mcp"]

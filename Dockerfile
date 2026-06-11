FROM rust:1.88-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian12:nonroot

ARG VERSION=dev
LABEL org.opencontainers.image.source="https://github.com/brianluby/vikunja-rust-mcp" \
      org.opencontainers.image.description="Model Context Protocol server for Vikunja (projects, tasks, labels, comments, attachments, users, teams)" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=builder /app/target/release/vikunja-rust-mcp /vikunja-rust-mcp

EXPOSE 8077
ENTRYPOINT ["/vikunja-rust-mcp"]

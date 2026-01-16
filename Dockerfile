FROM lukemathwalker/cargo-chef:latest-rust-1.90.0 AS chef
WORKDIR /app
RUN cargo install cargo-chef
RUN apt update && apt install lld clang -y


FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM oven/bun:latest AS frontend
WORKDIR /app
COPY app/package.json app/bun.lock* ./
RUN bun install --frozen-lockfile
COPY app/ .
RUN bun run build

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build application
COPY . .
RUN cargo build --release --bin spinploy

FROM ubuntu:24.04 AS runtime
WORKDIR /app
RUN apt-get update -y \
    && apt-get install -y --no-install-recommends openssl ca-certificates \
    && apt-get autoremove -y \
    && apt-get clean -y \
    && rm -rf /var/lib/apt/lists/*
# Copy necessary files from builder
COPY --from=builder /app/target/release/spinploy spinploy
COPY --from=frontend /app/dist ./app/dist

# NOTE: To enable container log streaming, mount the Docker socket when running:
#   docker run -v /var/run/docker.sock:/var/run/docker.sock ...
# Without the socket, the /containers/* endpoints will return 503.

ENTRYPOINT ["./spinploy"]

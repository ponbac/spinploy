FROM lukemathwalker/cargo-chef:latest-rust-1.90.0 AS chef
WORKDIR /app
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

FROM node:22-bookworm-slim AS node-runtime
FROM docker:28-cli AS docker-cli

FROM mcr.microsoft.com/dotnet/sdk:10.0 AS runtime
WORKDIR /app
RUN apt-get update -y \
    && apt-get install -y --no-install-recommends openssl ca-certificates git \
    && apt-get autoremove -y \
    && apt-get clean -y \
    && rm -rf /var/lib/apt/lists/*
COPY --from=node-runtime /usr/local/ /usr/local/
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/ /usr/local/libexec/docker/cli-plugins/
RUN npm install --global pnpm@11.22.0 \
    && dotnet tool install Aspire.Cli --tool-path /usr/local/bin --version 13.5.3
# Copy necessary files from builder
COPY --from=builder /app/target/release/spinploy spinploy
COPY --from=frontend /app/dist ./app/dist

# NOTE: To enable container log streaming, mount the Docker socket when running:
#   docker run -v /var/run/docker.sock:/var/run/docker.sock ...
# Without the socket, the /containers/* endpoints will return 503.

ENTRYPOINT ["./spinploy"]

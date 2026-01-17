# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Spinploy is a lightweight HTTP API that bridges Azure DevOps pipelines with Dokploy to create and manage PR preview deployments. It consists of:

1. **Backend API** (Rust/Axum) - Webhook endpoints for Azure DevOps events and REST endpoints for preview lifecycle management
2. **Frontend Application** (React/TypeScript/Vite in `app/`) - Web UI for managing and monitoring preview deployments

## Commands

### Backend Development

```bash
# Run locally (loads .env.local in debug builds)
cargo run

# Run with specific log level
RUST_LOG=debug cargo run

# Build release binary
cargo build --release

# Run tests
cargo test

# Lint
cargo clippy
```

### Frontend Development

```bash
# Install dependencies
cd app && bun install

# Run dev server (port 3000)
bun run dev

# Build for production
bun run build

# Run tests
bun run test

# Lint and format
bun run tsc
bun run check
bun run lint
bun run format

# Fix linting errors and format
bun run check:fix
```

### Docker

```bash
# Build container
docker build -t spinploy .

# Run with Docker socket mounted (enables /containers/* endpoints)
docker run -v /var/run/docker.sock:/var/run/docker.sock \
  -e DOKPLOY_URL=... -e PROJECT_ID=... [other env vars] \
  spinploy
```

### Backend API Testing

Bruno API client collection is in `bruno/` directory for manual testing of backend REST and webhook endpoints.

## Architecture

### Core Components

**main.rs** (src/main.rs)

- Axum HTTP server with middleware stack
- Authentication: API key extraction from `x-api-key` header or HTTP Basic auth password
- Authentication caching: In-memory cache with separate TTLs for valid (60s) and invalid (10s) keys
- Routes: health check, preview CRUD, Azure webhooks, container logs (SSE)
- Optional static file serving with token-based auth (`/storage/*`)

**DokployClient** (src/dokploy_client.rs)

- Thin HTTP wrapper around Dokploy REST API
- Manages compose deployments: create, update, deploy, delete
- Domain management for frontend/backend services
- Uses `x-api-key` header for authentication

**AzureDevOpsClient** (src/azure_client.rs)

- Handles Azure DevOps API interactions
- Posts PR thread replies for slash commands
- Fetches build details, timelines, and commit info for failure notifications

**SlackWebhookClient** (src/slack_client.rs)

- Sends formatted messages via Slack Incoming Webhooks
- Used for E2E test failure alerts

**DockerClient** (src/docker_client.rs)

- Optional client using bollard crate
- Lists containers and streams logs via Server-Sent Events (SSE)
- Gracefully disabled if Docker socket unavailable

### Frontend Application (app/)

**Tech Stack**

- **Build Tool**: Vite with React plugin and React Compiler
- **Framework**: React 19 with TypeScript
- **Routing**: TanStack Router (file-based routing in `src/routes/`)
- **Data Fetching**: TanStack Query with Query devtools
- **Styling**: Tailwind CSS v4
- **Component Library**: Shadcn UI components
- **Linting/Formatting**: Biome (replaces ESLint + Prettier)
- **Testing**: Vitest with Testing Library
- **Package Manager**: Bun

**Project Structure**

- `src/routes/` - File-based routes, each file becomes a route automatically
- `src/components/` - Reusable React components
- `src/integrations/tanstack-query/` - TanStack Query setup and devtools
- `src/lib/` - Utility functions and helpers
- `src/data/` - Data models and constants
- `components.json` - Shadcn UI configuration
- `biome.json` - Biome linter/formatter configuration

**Key Features**

- **File-based Routing**: Routes are defined as files in `src/routes/`, with `__root.tsx` as the layout
- **Devtools Integration**: Unified TanStack devtools panel for Router, Query, and Store debugging
- **Path Aliases**: `@/` aliased to `src/` for cleaner imports
- **React Compiler**: Uses experimental Babel plugin for automatic React optimization

**Adding Components**

Use Shadcn CLI to add pre-built components:

```bash
pnpm dlx shadcn@latest add button
```

**React Components**

- Prefer inline props types for small component interfaces (2-3 fields). Avoid creating separate `interface` or `type` definitions that are only used once.
- Prefer using `props` directly instead of destructuring in the function signature. This makes it clear what comes from outside the component vs. local variables.

  ```tsx
  // Preferred: inline props type, using props directly
  function UserBadge(props: { name: string; isActive: boolean }) {
    return <span className={props.isActive ? "active" : ""}>{props.name}</span>;
  }

  // Avoid: destructuring in signature + separate interface for trivial props
  interface UserBadgeProps {
    name: string;
    isActive: boolean;
  }
  function UserBadge({ name, isActive }: UserBadgeProps) { ... }
  ```

### Preview Deployment Flow

1. **Identifier Generation**: PR number (`pr-{num}`) or sanitized branch name (`br-{branch}`)
2. **Compose Lookup**: Check if preview already exists by name
3. **Create or Update**:
   - New: Create compose, configure git source, set environment variables, create frontend/backend domains, deploy
   - Existing: Trigger redeploy with latest code
4. **Domain Pattern**: `{identifier}.{BASE_DOMAIN}` (frontend), `api-{identifier}.{BASE_DOMAIN}` (backend)
5. **Pruning**: After creation, automatically delete oldest previews when count exceeds `PREVIEW_LIMIT` (4)

### Environment Variables

The environment variable format for Dokploy project references uses `${{project.VAR_NAME}}` syntax. This is NOT standard bash or docker-compose syntax - it's Dokploy-specific interpolation that allows preview environments to inherit shared secrets from the parent project.

Dynamic environment variables (per-preview) are prefixed directly, while project-level secrets use the `${{project.*}}` pattern. See src/main.rs:376-394 for the full environment configuration.

### Webhook Handlers

**PR Comment** (`/webhooks/azure/pr-comment`)

- Parses slash commands: `/preview` creates/updates, `/delete` removes
- Posts reply in same PR thread with preview URL or deletion confirmation

**PR Updated** (`/webhooks/azure/pr-updated`)

- Push notifications: Redeploy existing preview if present
- Status change to `completed` with target=main: Delete preview

**Build Completed** (`/webhooks/azure/build-completed`)

- Detects E2E stage failures (`Run E2E tests`)
- Checks recent build history to suppress duplicate alerts
- Fetches commit author and posts Slack message with build link

### Authentication Strategy

API key validation uses cached decisions to avoid Dokploy API hammering:

1. Check in-memory cache first
2. On cache miss, validate with `dokploy_client.fetch_projects()`
3. Cache decision with appropriate TTL
4. Return 401 for invalid keys, 503 for connectivity issues

The cache uses simple eviction (clear all when full) since max_keys is typically large relative to expected key count.

## Key Implementation Details

- **Isolation**: All preview environments use `isolated_deployment: true` to prevent container name conflicts
- **Preview Limit**: Maximum 4 previews enforced via pruning (src/main.rs:36)
- **Pruning Logic**: Sorts by latest deployment timestamp (finishedAt > startedAt > createdAt), deletes oldest
- **Error Handling**: Webhook handlers return 204 No Content for no-op conditions, log warnings for non-critical failures
- **SSE Streaming**: Container logs use Server-Sent Events with keepalive for real-time tailing
- **Git Branch Refs**: Azure webhooks provide full refs (`refs/heads/main`) which are stripped via `strip_refs_heads()`

## Configuration Notes

- `.env.local` is loaded automatically in debug builds only
- All configuration via environment variables (see README.md for full list)
- Storage serving requires all three: `STORAGE_BASE_URL`, `STORAGE_DIR`, `STORAGE_TOKEN`
- Docker socket must be mounted for container log endpoints to function

## Testing Considerations

- Unit tests exist for identifier generation and ref stripping (src/lib.rs)
- Integration tests require live Dokploy/Azure credentials (use `.env.local`)
- Bruno collection provides manual API testing scenarios
- `test_init_env()` helper loads test credentials from `.env.local`

## Philosophy

This codebase will outlive you. Every shortcut becomes someone else's burden. Every hack compounds into technical debt that slows the whole team down.

You are not just writing code. You are shaping the future of this project. The patterns you establish will be copied. The corners you cut will be cut again.

Fight entropy. Leave the codebase better than you found it.

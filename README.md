## spinploy

HTTP API that builds Aspire preview artifacts on the Dokploy VM and reconciles one isolated Dokploy Compose application per Azure DevOps pull request.

### Status

Early work-in-progress. Current server provides a health check and preview endpoints plus Azure DevOps webhooks.

### Quick start

```bash
# Configure (env vars or .env.local at repo root)
export DOKPLOY_URL=https://dokploy.example.com

# Dokploy environment
export ENVIRONMENT_ID=your_dokploy_environment_id
export BASE_DOMAIN=preview.example.com
export FRONTEND_SERVICE_NAME=frontend
export FRONTEND_PORT=3000
export BACKEND_PORT=8080

# Azure DevOps (for posting PR thread replies)
export AZDO_ORG=your_org
export AZDO_PROJECT=your_project
export AZDO_REPOSITORY_ID=00000000-0000-0000-0000-000000000000
export AZDO_PAT=your_pat_with_code_write
export SLACK_WEBHOOK_URL=https://hooks.slack.com/services/XXX/YYY/ZZZ

# Paths as seen by Spinploy and by the host Docker daemon
export PREVIEW_WORK_DIR=/data/previews
export PREVIEW_HOST_WORK_DIR=/absolute/host/path/shared/previews
export PREVIEW_CACHE_DIR=/data/preview-cache
export PREVIEW_HOST_CACHE_DIR=/absolute/host/path/shared/preview-cache
export PREVIEW_BUILDER_IMAGE=your/spinploy:latest

# Optional
export BIND_ADDR=0.0.0.0:8080
export RUST_LOG=debug

# Run
cargo run
```

You can also place these in a `.env.local` at the repo root (loaded in debug builds).

### Authentication

All API endpoints (except `/healthz`) require an API key on each request. Provide either:

- `x-api-key: <DOKPLOY_API_KEY>` header, or
- HTTP Basic auth with the API key as the password (username can be empty).

Spinploy validates this key by making a lightweight request to the Dokploy API. To ensure performance, validation results are cached in memory for a short period (configurable via environment variables).

### Configuration

- DOKPLOY_URL: Base URL of your Dokploy instance
- ENVIRONMENT_ID: Dokploy environment ID
- BASE_DOMAIN: Base domain used to mint preview subdomains
- FRONTEND_SERVICE_NAME: Compose service name for the frontend
- FRONTEND_PORT: Service port exposed for the frontend
- BACKEND_PORT: Internal API port placed in the generated Compose environment
- AZDO_ORG: Azure DevOps organization
- AZDO_PROJECT: Azure DevOps project
- AZDO_REPOSITORY_ID: Azure DevOps repository ID
- AZDO_PAT: Azure DevOps Personal Access Token (Code Write to post comments)
- SLACK_WEBHOOK_URL: Slack Incoming Webhook URL (alerts destination channel configured in Slack)
- BIND_ADDR (optional): Server bind address (default `0.0.0.0:8080`)
- RUST_LOG (optional): Tracing filter (defaults internally to `debug,axum=info,reqwest=info,hyper_util=info`)
- AUTH_CACHE_TTL_SECS (optional): TTL for successful API key validations (default `60`)
- AUTH_CACHE_NEGATIVE_TTL_SECS (optional): TTL for failed API key validations (default `10`)
- PREVIEW_WORK_DIR (optional): source/artifact workspace inside Spinploy (default `/data/previews`)
- PREVIEW_HOST_WORK_DIR (optional): matching host path mounted into builder containers (default `/home/ponbac/shared/previews`)
- PREVIEW_CACHE_DIR (optional): NuGet/pnpm cache path inside Spinploy (default `/data/preview-cache`)
- PREVIEW_HOST_CACHE_DIR (optional): matching host cache path mounted into builders (default `/home/ponbac/shared/preview-cache`)
- PREVIEW_BUILDER_IMAGE (optional): image used for isolated build containers; defaults to the running Spinploy container image discovered through Docker
- PREVIEW_APPHOST_PATH (optional): AppHost project within the Azure repository archive
- PREVIEW_FRONTEND_PATH (optional): frontend directory within the Azure repository archive
- PREVIEW_BUILD_TIMEOUT_SECS (optional): artifact build timeout (default `2700`)
- PREVIEW_READINESS_TIMEOUT_SECS (optional): same-origin readiness timeout (default `600`)

#### Optional: Protected static storage

If you want the API to also serve static files (like a simple storage bucket) behind a header-based token, set:

- `STORAGE_DIR`: Absolute path to the directory to serve
- `STORAGE_TOKEN`: Shared secret token clients must send in the `x-storage-token` header

When configured, files are available under `/storage/*`. Requests must include:

```
x-storage-token: <STORAGE_TOKEN>
```

Example request:

```bash
curl -H "x-storage-token: $STORAGE_TOKEN" \
     https://your-spinploy.example.com/storage/path/to/file.txt -o file.txt
```

### API

- GET `/healthz` — service health probe
- POST `/api/previews` — create or update a preview environment
  - Request (JSON): `{ "gitBranch": "feature/foo", "prId": "123" }` (`prId` optional)
  - Response (202 JSON): `{ "identifier": "pr-123", "status": "queued", "domains": ["pr-123.preview.example.com", "dashboard-pr-123.preview.example.com"] }`
- DELETE `/api/previews` — delete a preview environment
  - Request (JSON): `{ "gitBranch": "feature/foo", "prId": "123" }`
  - Response: 204 No Content
- POST `/webhooks/azure/pr-comment` — handle PR comment slash commands (`/preview`, `/delete`)
  - `/preview`: queues a VM-local Aspire build and replies with the frontend URL
  - `/delete`: deletes preview and replies "Preview deleted"
- POST `/webhooks/azure/pr-updated` —
  - Active PR targeting `main` or `master`: rebuild the exact source commit only when a preview was previously requested with `/preview`
  - Status change to `completed` or `abandoned`: delete the preview and its volumes
- POST `/webhooks/azure/build-completed` —
  - Expects Azure DevOps `build.completed` service hook payloads
  - If the build failed because one or more tracked Playwright E2E runs failed (`Run main E2E tests`, `Run journal template E2E tests`; legacy `Run E2E tests` also supported), posts a Slack Incoming Webhook message including the commit author name and build link

All API calls must include the API key as described in Authentication.

When storage is enabled, static files are served at `GET /storage/*` and require the `x-storage-token` header.

### Docker volume example

Mount a host directory and expose it via `/storage/*`:

```bash
docker run --rm -p 8080:8080 \
  -e DOKPLOY_URL=... \
  -e ENVIRONMENT_ID=... \
  -e BASE_DOMAIN=preview.example.com \
  -e FRONTEND_SERVICE_NAME=frontend \
  -e FRONTEND_PORT=3000 \
  -e BACKEND_PORT=8080 \
  -e AZDO_ORG=... -e AZDO_PROJECT=... -e AZDO_REPOSITORY_ID=... -e AZDO_PAT=... \
  -e STORAGE_DIR=/data/storage -e STORAGE_TOKEN=supersecret \
  -e PREVIEW_HOST_WORK_DIR=/absolute/path/on/host/previews \
  -e PREVIEW_HOST_CACHE_DIR=/absolute/path/on/host/preview-cache \
  -e PREVIEW_BUILDER_IMAGE=your/spinploy:latest \
  -v /absolute/path/on/host:/data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  your/spinploy:latest
```

### Azure DevOps usage

Add a lightweight step in your pipeline to create/update a preview on each PR build:

```yaml
- task: Bash@3
  displayName: Create/Update preview
  env:
    PREVIEW_API: https://your-spinploy.example.com
    DOKPLOY_API_KEY: $(DOKPLOY_API_KEY)
  script: |
    curl -sS -X POST "$PREVIEW_API/previews" \
      -H "x-api-key: $DOKPLOY_API_KEY" \
      -H 'Content-Type: application/json' \
      -d '{
            "gitBranch": "$(Build.SourceBranchName)",
            "prId": "$(System.PullRequest.PullRequestNumber)"
          }'
```

Service hooks:

- Pull request commented on: send to `/webhooks/azure/pr-comment`.
  - Authentication: include `x-api-key` header (or Basic with password-only) with your Dokploy API key
  - Slash commands handled in the same PR thread:
    - `/preview`: queues the exact PR commit for a VM-local Aspire build
    - `/delete`: deletes preview and replies "Preview deleted"
- Pull request updated — create two subscriptions, both to `/webhooks/azure/pr-updated`:
  - Settings: `notificationType = PushNotification` — Queue the latest exact source commit only when that PR already has a preview
  - Settings: `notificationType = StatusUpdateNotification` — Delete previews when matching PRs are completed or abandoned

Preview creation is always manual through `/preview`. Before a new preview starts building,
Spinploy removes the oldest preview when all three preview slots are occupied. `/delete`
cancels queued or in-progress work and removes the preview.

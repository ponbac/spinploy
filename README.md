## spinploy

Lightweight HTTP API for Azure DevOps to create and manage PR preview deployments on Dokploy. It exposes simple endpoints designed for use from pipelines and service hooks with a minimal Dokploy client.

### Status

Early work-in-progress. Current server provides a health check and preview endpoints plus Azure DevOps webhooks.

### Quick start

```bash
# Configure (env vars or .env.local at repo root)
export DOKPLOY_URL=https://dokploy.example.com

# Dokploy environment and git settings
export PROJECT_ID=your_dokploy_project_id
export ENVIRONMENT_ID=your_dokploy_environment_id
export CUSTOM_GIT_URL=ssh://git@example.com/your/repo.git
export CUSTOM_GIT_SSH_KEY_ID=ssh_key_id_in_dokploy
export COMPOSE_PATH=./docker-compose.yml
export BASE_DOMAIN=preview.example.com
export FRONTEND_SERVICE_NAME=web
export FRONTEND_PORT=3000
export BACKEND_SERVICE_NAME=api
export BACKEND_PORT=8080

# Azure DevOps (for posting PR thread replies)
export AZDO_ORG=your_org
export AZDO_PROJECT=your_project
export AZDO_REPOSITORY_ID=00000000-0000-0000-0000-000000000000
export AZDO_PAT=your_pat_with_code_write
export SLACK_WEBHOOK_URL=https://hooks.slack.com/services/XXX/YYY/ZZZ

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
- PROJECT_ID: Dokploy project ID
- ENVIRONMENT_ID: Dokploy environment ID
- CUSTOM_GIT_URL: Git URL Dokploy should pull from
- CUSTOM_GIT_SSH_KEY_ID: Dokploy SSH key ID to use for the repo
- COMPOSE_PATH: Path to your compose file within the repo
- BASE_DOMAIN: Base domain used to mint preview subdomains
- FRONTEND_SERVICE_NAME: Compose service name for the frontend
- FRONTEND_PORT: Service port exposed for the frontend
- BACKEND_SERVICE_NAME: Compose service name for the backend
- BACKEND_PORT: Service port exposed for the backend
- AZDO_ORG: Azure DevOps organization
- AZDO_PROJECT: Azure DevOps project
- AZDO_REPOSITORY_ID: Azure DevOps repository ID
- AZDO_PAT: Azure DevOps Personal Access Token (Code Write to post comments)
- SLACK_WEBHOOK_URL: Slack Incoming Webhook URL (alerts destination channel configured in Slack)
- BIND_ADDR (optional): Server bind address (default `0.0.0.0:8080`)
- RUST_LOG (optional): Tracing filter (defaults internally to `debug,axum=info,reqwest=info,hyper_util=info`)
- AUTH_CACHE_TTL_SECS (optional): TTL for successful API key validations (default `60`)
- AUTH_CACHE_NEGATIVE_TTL_SECS (optional): TTL for failed API key validations (default `10`)

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
- POST `/previews` — create or update a preview environment
  - Request (JSON): `{ "gitBranch": "feature/foo", "prId": "123" }` (`prId` optional)
  - Response (200 JSON): `{ "composeId": "...", "domains": ["host1", "host2"] }`
- DELETE `/previews` — delete a preview environment
  - Request (JSON): `{ "gitBranch": "feature/foo", "prId": "123" }`
  - Response: 204 No Content
- POST `/webhooks/azure/pr-comment` — handle PR comment slash commands (`/preview`, `/delete`)
  - `/preview`: creates/updates preview and replies with the frontend URL
  - `/delete`: deletes preview and replies "Preview deleted"
- POST `/webhooks/azure/pr-updated` —
  - Push: redeploy existing preview if present (204 if none)
  - Status change to `completed`: if target branch is `main`, delete preview
- POST `/webhooks/azure/build-completed` —
  - Expects Azure DevOps `build.completed` service hook payloads
  - If the build failed because the stage named `Run E2E tests` failed, posts a Slack Incoming Webhook message including the commit author name and build link

All API calls must include the API key as described in Authentication.

When storage is enabled, static files are served at `GET /storage/*` and require the `x-storage-token` header.

### Docker volume example

Mount a host directory and expose it via `/storage/*`:

```bash
docker run --rm -p 8080:8080 \
  -e DOKPLOY_URL=... \
  -e PROJECT_ID=... \
  -e ENVIRONMENT_ID=... \
  -e CUSTOM_GIT_URL=... \
  -e CUSTOM_GIT_SSH_KEY_ID=... \
  -e COMPOSE_PATH=./docker-compose.yml \
  -e BASE_DOMAIN=preview.example.com \
  -e FRONTEND_SERVICE_NAME=web \
  -e FRONTEND_PORT=3000 \
  -e BACKEND_SERVICE_NAME=api \
  -e BACKEND_PORT=8080 \
  -e AZDO_ORG=... -e AZDO_PROJECT=... -e AZDO_REPOSITORY_ID=... -e AZDO_PAT=... \
  -e STORAGE_DIR=/data/storage -e STORAGE_TOKEN=supersecret \
  -v /absolute/path/on/host:/data/storage:ro \
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
    - `/preview`: creates/updates preview and replies with the frontend URL
    - `/delete`: deletes preview and replies "Preview deleted"
- Pull request updated — create two subscriptions, both to `/webhooks/azure/pr-updated`:
  - Settings: `notificationType = PushNotification` — Redeploy existing preview if present (204 if none)
  - Settings: `notificationType = StatusUpdateNotification` — On status change to `completed`, delete preview (only when target branch is `main`)

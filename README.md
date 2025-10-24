## spinploy

Lightweight HTTP API for Azure DevOps to create and manage PR preview deployments on Dokploy. It exposes simple endpoints designed for use from pipelines and service hooks with a hand-written minimal Dokploy client.

### Status

Early work-in-progress. Current server provides a health check and preview endpoints are being added next.

### Quick start

```bash
# Configure (env vars or .env.local at repo root)
export DOKPLOY_URL=https://dokploy.example.com
export DOKPLOY_API_KEY=your_api_key_here

# Optional
export BIND_ADDR=0.0.0.0:3000
export RUST_LOG=info

# Run
cargo run
```

### Configuration

- **DOKPLOY_URL**: Base URL of your Dokploy instance
- **DOKPLOY_API_KEY**: Dokploy API key (sent as `x-api-key`)
- **BIND_ADDR**: Server bind address (default `0.0.0.0:3000`)
- **RUST_LOG**: Tracing filter (e.g., `debug`, `info`)
- **AZDO_ORG**: Azure DevOps organization
- **AZDO_PROJECT**: Azure DevOps project
- **AZDO_REPOSITORY_ID**: Azure DevOps repository ID
- **AZDO_PAT**: Azure DevOps Personal Access Token (Code Write to post comments)

You can also place these in a `.env.local` at the repo root.

### API

- GET `/healthz` — service health probe
- POST `/previews` — create or update a preview environment
- DELETE `/previews` — delete a preview environment
- POST `/webhooks/azure/pr-comment` — handle PR comment slash commands (`/preview`, `/delete`)
- POST `/webhooks/azure/pr-updated` — handle PR updated (PushNotification) to redeploy existing preview only
- POST `/webhooks/azure/pr-merged` — handle PR merged; on successful merge to `main` deletes preview

### Azure DevOps usage

Add a lightweight step in your pipeline to create/update a preview on each PR build:

```yaml
- task: Bash@3
  displayName: Create/Update preview
  env:
    PREVIEW_API: https://your-spinploy.example.com
  script: |
    curl -sS -X POST "$PREVIEW_API/previews" \
      -H 'Content-Type: application/json' \
      -d '{
            "repo": "$(Build.Repository.Name)",
            "prNumber": "$(System.PullRequest.PullRequestNumber)",
            "sha": "$(Build.SourceVersion)"
          }'
```

Service hooks:

- Pull request commented on: send to `/webhooks/azure/pr-comment`.
  - Authentication: include `x-api-key` header (or Basic with password only) matching server config
  - Slash commands handled in the same PR thread:
    - `/preview`: creates/updates preview and replies with the frontend URL
    - `/delete`: deletes preview and replies "Preview deleted"
- Pull request updated — Settings: `notificationType = PushNotification` — send to `/webhooks/azure/pr-updated`.
  - This endpoint redeploys only if a preview already exists for the PR; otherwise it no-ops (204).
- Pull request merge attempted — Publisher `tfs`, Event `git.pullrequest.merged` — send to `/webhooks/azure/pr-merged`.
  - Optional filters: `branch = main`, `mergeResult = Succeeded`.

### Security

Run behind your preferred ingress/proxy. Add authentication/authorization at the edge (token, IP allowlist, or org SSO). Dokploy credentials are held server-side and never exposed to pipelines.

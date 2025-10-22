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

You can also place these in a `.env.local` at the repo root.

### API

- GET `/healthz` — service health probe
- Planned:
  - POST `/previews` — create/update a preview environment for a PR/commit
  - DELETE `/previews/{id}` — delete a preview environment
  - POST `/hooks/azure-devops` — handle PR open/update/merge events

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

Use a service hook (Pull request events) to call `POST /hooks/azure-devops` for create/update/cleanup based on PR lifecycle.

### Security

Run behind your preferred ingress/proxy. Add authentication/authorization at the edge (token, IP allowlist, or org SSO). Dokploy credentials are held server-side and never exposed to pipelines.


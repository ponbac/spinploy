#!/bin/bash

set -euo pipefail

# Function to find directory containing Cargo.toml by walking upwards
find_cargo_root() {
    local dir="$PWD"
    while [[ "$dir" != "/" ]]; do
        if [[ -f "$dir/Cargo.toml" ]]; then
            echo "$dir"
            return 0
        fi
        dir=$(dirname "$dir")
    done
    echo "Error: Could not find Cargo.toml in any parent directory" >&2
    return 1
}

# Function to load variables from .env.local file
load_env_file() {
    local env_file="$1"
    if [[ -f "$env_file" ]]; then
        echo "Loading environment from $env_file" >&2
        # shellcheck disable=SC2046
        export $(grep -v '^#' "$env_file" | grep -v '^$' | xargs)
    fi
}

# Find the project root with Cargo.toml
PROJECT_ROOT=$(find_cargo_root)
echo "Found project root: $PROJECT_ROOT" >&2

# Load from .env.local if variables aren't set
if [[ -z "${DOKPLOY_URL:-}" ]] || [[ -z "${DOKPLOY_API_KEY:-}" ]]; then
    ENV_FILE="$PROJECT_ROOT/.env.local"
    if [[ -f "$ENV_FILE" ]]; then
        load_env_file "$ENV_FILE"
    else
        echo "Warning: .env.local not found at $ENV_FILE" >&2
    fi
fi

# Validate required variables
if [[ -z "${DOKPLOY_URL:-}" ]]; then
    echo "Error: DOKPLOY_URL is not set" >&2
    exit 1
fi

if [[ -z "${DOKPLOY_API_KEY:-}" ]]; then
    echo "Error: DOKPLOY_API_KEY is not set" >&2
    exit 1
fi

# Construct API endpoint
API_ENDPOINT="${DOKPLOY_URL}/api/settings.getOpenApiDocument"
OUTPUT_FILE="$PROJECT_ROOT/openapi.json"

echo "Fetching OpenAPI spec from $API_ENDPOINT" >&2

# Fetch the OpenAPI document
curl -fsSL \
    -H "x-api-key: ${DOKPLOY_API_KEY}" \
    -H "Accept: application/json" \
    "$API_ENDPOINT" \
    -o "$OUTPUT_FILE"

echo "OpenAPI spec saved to $OUTPUT_FILE" >&2

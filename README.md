# signoz-cli

Auto-generated SigNoz CLI from the OpenAPI spec, plus query_range helpers for logs/traces/metrics.

## Install

### Install script (macOS arm64 + Linux x86_64)

```bash
curl -fsSL https://raw.githubusercontent.com/radjathaher/signoz-cli/main/scripts/install.sh | bash
```

### Homebrew (binary, macOS arm64 only)

```bash
brew tap radjathaher/tap
brew install signoz-cli
```

### Download binary

Grab the latest `signoz-cli-<version>-<os>-<arch>.tar.gz` from GitHub Releases, unpack, and place `signoz` on your PATH.

### Build from source

```bash
cargo build --release
./target/release/signoz --help
```

## Auth

SigNoz API base URL (default: http://localhost:3301):

```bash
export SIGNOZ_API_URL="http://localhost:3301"
```

API key (header `SIGNOZ-API-KEY`):

```bash
export SIGNOZ_API_KEY="signoz_api_key_here"
```

Optional bearer token:

```bash
export SIGNOZ_TOKEN="<token>"
```

## Discovery

```bash
signoz list --json
signoz describe users get-user --json
signoz tree --json
```

Human help:

```bash
signoz --help
signoz users --help
signoz users get-user --help
```

## Examples

List users (example endpoint):

```bash
signoz users get-users --pretty
```

Create alert (example):

```bash
signoz alerts create-alert \
  --body @alert.json \
  --pretty
```

Query range (logs):

```bash
signoz logs query-range \
  --body @query.json \
  --pretty
```

## Update schema + command tree

```bash
tools/fetch_openapi.py --out schemas/openapi.yml
tools/gen_command_tree.py --openapi schemas/openapi.yml --out schemas/command_tree.json
cargo build --release
```

## Notes

- `--body` accepts inline JSON, `@file.json`, or `@-` (stdin).
- Use `--raw` to include HTTP status and headers.

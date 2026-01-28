# signoz-cli

Auto-generated SigNoz CLI from the OpenAPI spec, plus curated alerting endpoints and a raw request mode.

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

Terraform-style alias (API key):

```bash
export SIGNOZ_ACCESS_TOKEN="signoz_api_key_here"
```

Optional bearer token:

```bash
export SIGNOZ_TOKEN="<token>"
```

Auth mode (default: auto, tries api-key then token on 401/403):

```bash
signoz --auth auto ...
signoz --auth api-key ...
signoz --auth token ...
```

Alternative base URL env (alias):

```bash
export SIGNOZ_ENDPOINT="http://localhost:3301"
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

Raw request (any endpoint):

```bash
signoz request \
  --method GET \
  --path /api/v1/version \
  --pretty
```

Query range (logs):

```bash
signoz logs query-range \
  --body @query.json \
  --pretty
```

Alert investigation workflow (starting from ruleId / traceID / spanID):

```bash
# 1) fetch rule definition (undocumented endpoint; may require token)
signoz rules get-rule --id 019bd4ca-be32-7795-a5bd-2c2c33275b77 --pretty

# 2) query traces for the traceID
cat > trace.json <<'JSON'
{
  "start": 1700000000000,
  "end": 1700000900000,
  "requestType": "raw",
  "variables": {},
  "compositeQuery": {
    "queries": [
      {
        "type": "builder_query",
        "spec": {
          "name": "A",
          "signal": "traces",
          "selectFields": [
            { "name": "serviceName" },
            { "name": "name" },
            { "name": "traceID" },
            { "name": "spanID" },
            { "name": "timestamp" },
            { "name": "durationNano" }
          ],
          "filter": {
            "expression": "traceID = 'a47f2c73aa0b2b5d8e864f253bb070f7'"
          },
          "order": [
            { "key": { "name": "timestamp" }, "direction": "desc" }
          ],
          "limit": 50,
          "offset": 0,
          "disabled": false
        }
      }
    ]
  }
}
JSON

signoz traces query-range --body @trace.json --pretty

# Optional: if your traces include code attributes, add them to selectFields:
# { "name": "code.filepath" }, { "name": "code.lineno" }, { "name": "code.function" }

# 3) query logs for the traceID / spanID
cat > logs.json <<'JSON'
{
  "start": 1700000000000,
  "end": 1700000900000,
  "requestType": "raw",
  "variables": {},
  "compositeQuery": {
    "queries": [
      {
        "type": "builder_query",
        "spec": {
          "name": "A",
          "signal": "logs",
          "filter": {
            "expression": "trace_id = 'a47f2c73aa0b2b5d8e864f253bb070f7' AND span_id = '03a2da795ab4c2eb'"
          },
          "order": [
            { "key": { "name": "timestamp" }, "direction": "desc" },
            { "key": { "name": "id" }, "direction": "desc" }
          ],
          "limit": 100,
          "offset": 0
        }
      }
    ]
  }
}
JSON

signoz logs query-range --body @logs.json --pretty
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
- Alerting endpoints (channels/rules/alerts) are curated; rules/alerts are undocumented and may require bearer tokens.
- Log/trace attribute keys can vary; adjust `traceID`/`trace_id` or custom keys to match your data.

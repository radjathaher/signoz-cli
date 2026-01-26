#!/usr/bin/env python3
import argparse
import os
import sys
import urllib.request

DEFAULT_URL = "https://raw.githubusercontent.com/SigNoz/signoz/main/docs/api/openapi.yml"


def read_source(url_or_path: str) -> bytes:
    if os.path.exists(url_or_path):
        with open(url_or_path, "rb") as f:
            return f.read()
    req = urllib.request.Request(url_or_path, headers={"user-agent": "signoz-cli"})
    with urllib.request.urlopen(req) as resp:
        return resp.read()


def main() -> int:
    parser = argparse.ArgumentParser(description="Fetch SigNoz OpenAPI spec.")
    parser.add_argument("--out", default="schemas/openapi.yml")
    parser.add_argument("--url", default=os.getenv("SIGNOZ_OPENAPI_URL", DEFAULT_URL))
    args = parser.parse_args()

    data = read_source(args.url)
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "wb") as f:
        f.write(data)
    print(args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

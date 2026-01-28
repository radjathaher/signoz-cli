#!/usr/bin/env python3
import argparse
import json
import os
import re
import sys
import subprocess
from typing import Dict, List, Tuple

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None

CAMEL_RE = re.compile(r"([a-z0-9])([A-Z])")


def camel_to_kebab(value: str) -> str:
    return CAMEL_RE.sub(r"\1-\2", value).replace("_", "-").lower()


def safe_kebab(value: str) -> str:
    if not value:
        return ""
    return camel_to_kebab(value).strip("-")


def load_yaml(path: str) -> Dict:
    if yaml is None:
        try:
            output = subprocess.check_output(
                ["ruby", "-ryaml", "-rjson", "-e", "puts JSON.generate(YAML.load(ARGF.read))", path],
                text=True,
            )
            return json.loads(output)
        except Exception as exc:
            print("error: PyYAML missing and ruby YAML fallback failed", file=sys.stderr)
            raise SystemExit(1) from exc
    with open(path, "r", encoding="utf-8") as f:
        return yaml.safe_load(f)


def resolve_ref(ref: str, components: Dict) -> Dict:
    if not ref.startswith("#/components/"):
        return {}
    parts = ref.split("/")
    cur = components
    for part in parts[2:]:
        cur = (cur or {}).get(part)
    return cur or {}


def schema_info(schema: Dict, components: Dict) -> Tuple[str, bool]:
    if not schema:
        return "string", False
    if "$ref" in schema:
        ref_name = schema["$ref"].split("/")[-1]
        resolved = resolve_ref(schema["$ref"], components)
        if resolved:
            return schema_info(resolved, components)
        return ref_name, False
    if "oneOf" in schema:
        return "oneOf", False
    if "anyOf" in schema:
        return "anyOf", False
    if "allOf" in schema:
        return "allOf", False
    t = schema.get("type")
    if t == "array":
        items = schema.get("items", {})
        item_type, _ = schema_info(items, components)
        return f"array<{item_type}>", True
    if t:
        return t, False
    if schema.get("properties"):
        return "object", False
    return "string", False


def param_key(param: Dict) -> Tuple[str, str]:
    return (param.get("name", ""), param.get("in", "query"))


def resolve_param(param: Dict, components: Dict) -> Dict:
    if "$ref" in param:
        resolved = resolve_ref(param["$ref"], components)
        if resolved:
            return resolved
    return param


def build_params(path_item: Dict, op: Dict, components: Dict) -> List[Dict]:
    params: Dict[Tuple[str, str], Dict] = {}
    for p in path_item.get("parameters", []) or []:
        rp = resolve_param(p, components)
        params[param_key(rp)] = rp
    for p in op.get("parameters", []) or []:
        rp = resolve_param(p, components)
        params[param_key(rp)] = rp

    used_flags = set()
    out = []
    for p in params.values():
        name = p.get("name", "")
        location = p.get("in", "query")
        required = bool(p.get("required"))
        schema = p.get("schema", {})
        schema_type, is_array = schema_info(schema, components)

        base_flag = safe_kebab(name)
        flag = base_flag
        if location == "header":
            flag = f"header-{base_flag}"
        if flag in used_flags:
            flag = f"{location}-{base_flag}"
        used_flags.add(flag)

        param_name = f"{location}__{base_flag}"
        out.append(
            {
                "param_name": name,
                "name": param_name,
                "flag": flag,
                "location": location,
                "required": required,
                "schema_type": schema_type,
                "is_array": is_array,
            }
        )
    return out


def request_body_info(op: Dict, components: Dict) -> Dict:
    body = op.get("requestBody")
    if not body:
        return {}
    content = body.get("content", {})
    if not content:
        return {}

    content_type = "application/json" if "application/json" in content else next(iter(content.keys()))
    schema = (content.get(content_type, {}) or {}).get("schema", {})
    schema_type, _ = schema_info(schema, components)

    return {
        "required": bool(body.get("required")),
        "content_type": content_type,
        "schema_type": schema_type,
    }


def resource_from_path(path: str, tags: List[str]) -> str:
    if tags:
        return safe_kebab(tags[0])
    parts = [p for p in path.split("/") if p]
    if len(parts) >= 3 and parts[0] == "api" and parts[1].startswith("v"):
        return safe_kebab(parts[2])
    if parts:
        return safe_kebab(parts[0])
    return "misc"


def op_name_from_operation(op: Dict, method: str, path: str) -> str:
    op_id = op.get("operationId")
    if op_id:
        return safe_kebab(op_id)
    cleaned = re.sub(r"[{}]", "", path)
    cleaned = cleaned.replace("/", "-").strip("-")
    return safe_kebab(f"{method}-{cleaned}")


def add_query_range_extras(resources: Dict[str, List[Dict]]) -> None:
    for res in ("logs", "traces", "metrics"):
        op = {
            "name": "query-range",
            "method": "POST",
            "path": "/api/v5/query_range",
            "summary": f"Query range for {res}",
            "description": "SigNoz query_range API",
            "tags": [res],
            "deprecated": False,
            "params": [],
            "request_body": {
                "required": True,
                "content_type": "application/json",
                "schema_type": "QueryRangeRequest",
            },
        }
        resources.setdefault(res, []).append(op)


def path_param(name: str) -> Dict:
    return {
        "param_name": name,
        "name": f"path__{safe_kebab(name)}",
        "flag": safe_kebab(name),
        "location": "path",
        "required": True,
        "schema_type": "string",
        "is_array": False,
    }


def add_alerting_extras(resources: Dict[str, List[Dict]]) -> None:
    channels = [
        {
            "name": "list-channels",
            "method": "GET",
            "path": "/api/v1/channels",
            "summary": "List notification channels",
            "description": "List notification channels (documented in SigNoz alerting docs).",
            "params": [],
            "request_body": None,
        },
        {
            "name": "create-channel",
            "method": "POST",
            "path": "/api/v1/channels",
            "summary": "Create notification channel",
            "description": "Create notification channel (documented in SigNoz alerting docs).",
            "params": [],
            "request_body": {
                "required": True,
                "content_type": "application/json",
                "schema_type": "object",
            },
        },
        {
            "name": "update-channel",
            "method": "PUT",
            "path": "/api/v1/channels/{id}",
            "summary": "Update notification channel",
            "description": "Update notification channel (documented in SigNoz alerting docs).",
            "params": [path_param("id")],
            "request_body": {
                "required": True,
                "content_type": "application/json",
                "schema_type": "object",
            },
        },
        {
            "name": "delete-channel",
            "method": "DELETE",
            "path": "/api/v1/channels/{id}",
            "summary": "Delete notification channel",
            "description": "Delete notification channel (documented in SigNoz alerting docs).",
            "params": [path_param("id")],
            "request_body": None,
        },
    ]
    rules = [
        {
            "name": "list-rules",
            "method": "GET",
            "path": "/api/v1/rules",
            "summary": "List alert rules",
            "description": "List alert rules (undocumented; verify against your SigNoz version).",
            "params": [],
            "request_body": None,
        },
        {
            "name": "get-rule",
            "method": "GET",
            "path": "/api/v1/rules/{id}",
            "summary": "Get alert rule",
            "description": "Get alert rule (undocumented; verify against your SigNoz version).",
            "params": [path_param("id")],
            "request_body": None,
        },
        {
            "name": "create-rule",
            "method": "POST",
            "path": "/api/v1/rules",
            "summary": "Create alert rule",
            "description": "Create alert rule (undocumented; verify against your SigNoz version).",
            "params": [],
            "request_body": {
                "required": True,
                "content_type": "application/json",
                "schema_type": "object",
            },
        },
        {
            "name": "update-rule",
            "method": "PUT",
            "path": "/api/v1/rules/{id}",
            "summary": "Update alert rule",
            "description": "Update alert rule (undocumented; verify against your SigNoz version).",
            "params": [path_param("id")],
            "request_body": {
                "required": True,
                "content_type": "application/json",
                "schema_type": "object",
            },
        },
        {
            "name": "delete-rule",
            "method": "DELETE",
            "path": "/api/v1/rules/{id}",
            "summary": "Delete alert rule",
            "description": "Delete alert rule (undocumented; verify against your SigNoz version).",
            "params": [path_param("id")],
            "request_body": None,
        },
    ]
    alerts = [
        {
            "name": "list-alerts",
            "method": "GET",
            "path": "/api/v1/alerts",
            "summary": "List alerts",
            "description": "List alerts (undocumented; verify against your SigNoz version).",
            "params": [],
            "request_body": None,
        },
        {
            "name": "get-alert",
            "method": "GET",
            "path": "/api/v1/alerts/{id}",
            "summary": "Get alert",
            "description": "Get alert (undocumented; verify against your SigNoz version).",
            "params": [path_param("id")],
            "request_body": None,
        },
    ]
    for op in channels:
        op["tags"] = ["channels"]
        op["deprecated"] = False
    for op in rules:
        op["tags"] = ["rules"]
        op["deprecated"] = False
    for op in alerts:
        op["tags"] = ["alerts"]
        op["deprecated"] = False

    resources.setdefault("channels", []).extend(channels)
    resources.setdefault("rules", []).extend(rules)
    resources.setdefault("alerts", []).extend(alerts)


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate CLI command tree from OpenAPI.")
    parser.add_argument("--openapi", default="schemas/openapi.yml")
    parser.add_argument("--out", default="schemas/command_tree.json")
    parser.add_argument("--base-url", default=os.getenv("SIGNOZ_API_URL", "http://localhost:3301"))
    args = parser.parse_args()

    spec = load_yaml(args.openapi)
    components = spec.get("components", {}) or {}

    resources: Dict[str, List[Dict]] = {}
    paths = spec.get("paths", {}) or {}

    for path, path_item in paths.items():
        if not isinstance(path_item, dict):
            continue
        for method in ("get", "post", "put", "patch", "delete", "head", "options"):
            if method not in path_item:
                continue
            op = path_item[method] or {}
            tags = op.get("tags", []) or []
            resource = resource_from_path(path, tags)
            op_name = op_name_from_operation(op, method, path)

            params = build_params(path_item, op, components)
            body_info = request_body_info(op, components)

            resources.setdefault(resource, []).append(
                {
                    "name": op_name,
                    "method": method.upper(),
                    "path": path,
                    "summary": op.get("summary"),
                    "description": op.get("description"),
                    "tags": [safe_kebab(t) for t in tags],
                    "deprecated": bool(op.get("deprecated")),
                    "params": params,
                    "request_body": body_info or None,
                }
            )

    add_query_range_extras(resources)
    add_alerting_extras(resources)

    resources_out = []
    for name in sorted(resources.keys()):
        ops = resources[name]
        seen = {}
        for op in ops:
            if op["name"] in seen:
                seen[op["name"]] += 1
                op["name"] = f"{op['name']}-{seen[op['name']]}"
            else:
                seen[op["name"]] = 1
        ops_sorted = sorted(ops, key=lambda o: o["name"])
        resources_out.append({"name": name, "ops": ops_sorted})

    tree = {
        "version": 1,
        "base_url": args.base_url,
        "resources": resources_out,
    }

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(tree, f, indent=2, sort_keys=True)
    print(args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

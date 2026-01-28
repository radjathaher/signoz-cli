mod command_tree;
mod http;

use anyhow::{anyhow, Context, Result};
use clap::{Arg, ArgAction, Command};
use command_tree::{CommandTree, Operation, ParamDef};
use http::{Body, HttpClient};
use serde_json::{json, Value};
use std::{env, fs, io::Read};
use urlencoding::encode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthMode {
    ApiKey,
    Token,
    Auto,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let tree = command_tree::load_command_tree();
    let cli = build_cli(&tree);
    let matches = cli.get_matches();

    if let Some(matches) = matches.subcommand_matches("list") {
        return handle_list(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("describe") {
        return handle_describe(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("tree") {
        return handle_tree(&tree, matches);
    }

    let base_url = matches
        .get_one::<String>("base-url")
        .cloned()
        .or_else(|| env::var("SIGNOZ_API_URL").ok())
        .or_else(|| env::var("SIGNOZ_ENDPOINT").ok())
        .unwrap_or_else(|| tree.base_url.clone());

    let api_key = matches
        .get_one::<String>("api-key")
        .cloned()
        .or_else(|| env::var("SIGNOZ_API_KEY").ok());
    let api_key = api_key.or_else(|| env::var("SIGNOZ_ACCESS_TOKEN").ok());

    let token = matches
        .get_one::<String>("token")
        .cloned()
        .or_else(|| env::var("SIGNOZ_TOKEN").ok());

    let headers = parse_header_args(matches.get_many::<String>("header"));
    let timeout = matches
        .get_one::<String>("timeout")
        .and_then(|v| v.parse::<u64>().ok());
    let auth_mode = parse_auth_mode(
        matches.get_one::<String>("auth"),
        api_key.as_ref(),
        token.as_ref(),
    );

    let pretty = matches.get_flag("pretty");
    let raw = matches.get_flag("raw");

    if let Some(matches) = matches.subcommand_matches("request") {
        return handle_request(
            matches, &base_url, api_key, token, auth_mode, headers, timeout, pretty, raw,
        );
    }

    let (res_name, res_matches) = matches
        .subcommand()
        .ok_or_else(|| anyhow!("resource required"))?;
    let (op_name, op_matches) = res_matches
        .subcommand()
        .ok_or_else(|| anyhow!("operation required"))?;

    let op = find_op(&tree, res_name, op_name)
        .ok_or_else(|| anyhow!("unknown command {res_name} {op_name}"))?;

    let (path, query, header_params) = build_request_parts(op, op_matches)?;
    let (body, content_type) = build_body(op, op_matches)?;

    let mut merged_headers = headers;
    merged_headers.extend(header_params);

    let mut response = execute_with_auth(
        &base_url,
        api_key.as_ref(),
        token.as_ref(),
        auth_mode,
        &merged_headers,
        timeout,
        &op.method,
        &path,
        &query,
        body.clone(),
        content_type.as_deref(),
    )?;
    if should_retry_v1(&path, &response) {
        let fallback_path = op.path.replacen("/api/v2/", "/api/v1/", 1);
        let fallback = execute_with_auth(
            &base_url,
            api_key.as_ref(),
            token.as_ref(),
            auth_mode,
            &merged_headers,
            timeout,
            &op.method,
            &fallback_path,
            &query,
            body,
            content_type.as_deref(),
        )?;
        if !is_html_response(&fallback) {
            response = fallback;
        }
    }

    ensure_api_response(&path, &response)?;

    let output = if raw {
        json!({
            "status": response.status,
            "headers": response.headers,
            "body": response.body,
        })
    } else {
        response.body
    };

    if pretty {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", serde_json::to_string(&output)?);
    }

    if response.status >= 400 {
        return Err(anyhow!("http {}", response.status));
    }

    Ok(())
}

fn should_retry_v1(path: &str, response: &http::HttpResponse) -> bool {
    if !path.starts_with("/api/v2/") {
        return false;
    }
    is_html_response(response)
}

fn is_html_response(response: &http::HttpResponse) -> bool {
    if response.content_type.contains("text/html") {
        return true;
    }
    match &response.body {
        Value::String(value) => {
            let trimmed = value.trim_start().to_ascii_lowercase();
            trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html")
        }
        _ => false,
    }
}

fn is_api_path(path: &str) -> bool {
    if path.starts_with("/api/") {
        return true;
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.contains("/api/");
    }
    false
}

fn ensure_api_response(path: &str, response: &http::HttpResponse) -> Result<()> {
    if is_api_path(path) && is_html_response(response) {
        return Err(anyhow!(
            "html response for {path}. base url likely points to UI/marketing or auth is missing"
        ));
    }
    Ok(())
}

fn build_cli(tree: &CommandTree) -> Command {
    let mut cmd = Command::new("signoz")
        .about("SigNoz CLI (auto-generated)")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("base-url")
                .long("base-url")
                .value_name("URL")
                .global(true)
                .help("SigNoz API base URL"),
        )
        .arg(
            Arg::new("api-key")
                .long("api-key")
                .value_name("KEY")
                .global(true)
                .help("SigNoz API key (SIGNOZ_API_KEY)"),
        )
        .arg(
            Arg::new("token")
                .long("token")
                .value_name("TOKEN")
                .global(true)
                .help("SigNoz bearer token (SIGNOZ_TOKEN)"),
        )
        .arg(
            Arg::new("auth")
                .long("auth")
                .value_name("MODE")
                .global(true)
                .value_parser(["api-key", "token", "auto"])
                .help("Auth mode: api-key, token, auto"),
        )
        .arg(
            Arg::new("header")
                .long("header")
                .value_name("NAME:VALUE")
                .global(true)
                .action(ArgAction::Append)
                .help("Extra header (repeatable)"),
        )
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .value_name("SECS")
                .global(true)
                .help("HTTP timeout in seconds"),
        )
        .arg(
            Arg::new("pretty")
                .long("pretty")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Pretty-print JSON output"),
        )
        .arg(
            Arg::new("raw")
                .long("raw")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Return status + headers + body"),
        );

    cmd = cmd.subcommand(
        Command::new("list")
            .about("List resources and operations")
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Emit machine-readable JSON"),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("describe")
            .about("Describe a specific operation")
            .arg(Arg::new("resource").required(true))
            .arg(Arg::new("op").required(true))
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Emit machine-readable JSON"),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("tree").about("Show full command tree").arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Emit machine-readable JSON"),
        ),
    );

    cmd = cmd.subcommand(
        Command::new("request")
            .about("Raw HTTP request to any SigNoz endpoint")
            .arg(
                Arg::new("method")
                    .long("method")
                    .value_name("HTTP")
                    .required(true),
            )
            .arg(Arg::new("path").long("path").value_name("PATH"))
            .arg(Arg::new("url").long("url").value_name("URL"))
            .arg(
                Arg::new("query")
                    .long("query")
                    .value_name("KEY=VALUE")
                    .action(ArgAction::Append)
                    .help("Query parameter (repeatable)"),
            )
            .arg(
                Arg::new("body")
                    .long("body")
                    .value_name("JSON|@file|@-")
                    .help("Request body payload"),
            )
            .arg(
                Arg::new("content-type")
                    .long("content-type")
                    .value_name("TYPE")
                    .help("Request Content-Type for --body"),
            ),
    );

    for resource in &tree.resources {
        let mut res_cmd = Command::new(resource.name.clone())
            .about(resource.name.clone())
            .subcommand_required(true)
            .arg_required_else_help(true);
        for op in &resource.ops {
            let mut op_cmd = Command::new(op.name.clone())
                .about(op.summary.clone().unwrap_or_else(|| op.path.clone()));
            for param in &op.params {
                op_cmd = op_cmd.arg(build_param_arg(param));
            }
            if op.request_body.is_some() {
                op_cmd = op_cmd.arg(
                    Arg::new("body")
                        .long("body")
                        .value_name("JSON|@file|@-")
                        .help("Request body payload"),
                );
            }
            res_cmd = res_cmd.subcommand(op_cmd);
        }
        cmd = cmd.subcommand(res_cmd);
    }

    cmd
}

fn handle_list(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    if matches.get_flag("json") {
        let mut out = Vec::new();
        for res in &tree.resources {
            let ops: Vec<String> = res.ops.iter().map(|op| op.name.clone()).collect();
            out.push(json!({"resource": res.name, "ops": ops}));
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    for res in &tree.resources {
        println!("{}", res.name);
        for op in &res.ops {
            println!("  {}", op.name);
        }
    }
    Ok(())
}

fn handle_describe(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    let resource = matches
        .get_one::<String>("resource")
        .ok_or_else(|| anyhow!("resource required"))?;
    let op_name = matches
        .get_one::<String>("op")
        .ok_or_else(|| anyhow!("operation required"))?;

    let op = find_op(tree, resource, op_name)
        .ok_or_else(|| anyhow!("unknown command {resource} {op_name}"))?;

    if matches.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(op)?);
        return Ok(());
    }

    println!("{} {}", resource, op.name);
    println!("  method: {}", op.method);
    println!("  path: {}", op.path);
    if let Some(summary) = &op.summary {
        println!("  summary: {}", summary);
    }
    if let Some(desc) = &op.description {
        println!("  description: {}", desc.trim());
    }
    if !op.params.is_empty() {
        println!("  params:");
        for param in &op.params {
            println!(
                "    --{}  {} ({})",
                param.flag, param.schema_type, param.location
            );
        }
    }
    if let Some(body) = &op.request_body {
        println!("  body: {} ({})", body.schema_type, body.content_type);
    }
    Ok(())
}

fn handle_tree(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    if matches.get_flag("json") {
        println!("{}", serde_json::to_string_pretty(tree)?);
        return Ok(());
    }
    println!("Run with --json for machine-readable output.");
    Ok(())
}

fn build_param_arg(param: &ParamDef) -> Arg {
    let mut arg_def = Arg::new(param.name.clone())
        .long(param.flag.clone())
        .value_name(param.schema_type.clone());
    if param.is_array {
        arg_def = arg_def.action(ArgAction::Append);
    }
    if param.required {
        arg_def = arg_def.required(false);
    }
    arg_def
}

fn find_op<'a>(tree: &'a CommandTree, res: &str, op: &str) -> Option<&'a Operation> {
    tree.resources
        .iter()
        .find(|r| r.name == res)
        .and_then(|r| r.ops.iter().find(|o| o.name == op))
}

fn build_request_parts(
    op: &Operation,
    matches: &clap::ArgMatches,
) -> Result<(String, Vec<(String, String)>, Vec<(String, String)>)> {
    let mut path = op.path.clone();
    let mut query = Vec::new();
    let mut headers = Vec::new();

    for param in &op.params {
        let values = if param.is_array {
            matches
                .get_many::<String>(&param.name)
                .map(|vals| vals.cloned().collect::<Vec<_>>())
        } else {
            matches
                .get_one::<String>(&param.name)
                .map(|v| vec![v.clone()])
        };

        if values.is_none() {
            if param.required {
                return Err(anyhow!("missing required argument --{}", param.flag));
            }
            continue;
        }

        let mut values = values.unwrap_or_default();
        if param.is_array && values.len() == 1 && values[0].trim_start().starts_with('[') {
            values = parse_json_list(&values[0])?;
        }

        match param.location.as_str() {
            "path" => {
                let value = values
                    .get(0)
                    .ok_or_else(|| anyhow!("missing value for --{}", param.flag))?;
                let encoded = encode(value).to_string();
                path = path.replace(&format!("{{{}}}", param.param_name), &encoded);
            }
            "query" => {
                for value in values {
                    query.push((param.param_name.clone(), value));
                }
            }
            "header" => {
                for value in values {
                    headers.push((param.param_name.clone(), value));
                }
            }
            _ => {}
        }
    }

    Ok((path, query, headers))
}

fn build_body(
    op: &Operation,
    matches: &clap::ArgMatches,
) -> Result<(Option<Body>, Option<String>)> {
    let Some(body_def) = &op.request_body else {
        return Ok((None, None));
    };

    let body_value = matches.get_one::<String>("body").cloned();
    if body_value.is_none() {
        if body_def.required {
            return Err(anyhow!("missing required --body"));
        }
        return Ok((None, Some(body_def.content_type.clone())));
    }

    let raw = read_body_input(&body_value.unwrap())?;
    if body_def.content_type.contains("json") {
        let parsed: Value = serde_json::from_str(&raw).context("invalid JSON body")?;
        return Ok((
            Some(Body::Json(parsed)),
            Some(body_def.content_type.clone()),
        ));
    }

    Ok((Some(Body::Text(raw)), Some(body_def.content_type.clone())))
}

fn handle_request(
    matches: &clap::ArgMatches,
    base_url: &str,
    api_key: Option<String>,
    token: Option<String>,
    auth_mode: AuthMode,
    headers: Vec<(String, String)>,
    timeout: Option<u64>,
    pretty: bool,
    raw: bool,
) -> Result<()> {
    let method = matches
        .get_one::<String>("method")
        .ok_or_else(|| anyhow!("missing --method"))?;
    let path = matches
        .get_one::<String>("url")
        .cloned()
        .or_else(|| matches.get_one::<String>("path").cloned())
        .ok_or_else(|| anyhow!("missing --path or --url"))?;
    let query = parse_kv_args(matches.get_many::<String>("query"), "query")?;
    let content_type = matches.get_one::<String>("content-type").cloned();
    let body = matches.get_one::<String>("body").cloned();
    let (body, content_type) = build_request_body(body, content_type)?;

    let response = execute_with_auth(
        base_url,
        api_key.as_ref(),
        token.as_ref(),
        auth_mode,
        &headers,
        timeout,
        method,
        &path,
        &query,
        body,
        content_type.as_deref(),
    )?;

    ensure_api_response(&path, &response)?;

    let output = if raw {
        json!({
            "status": response.status,
            "headers": response.headers,
            "body": response.body,
        })
    } else {
        response.body
    };

    if pretty {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", serde_json::to_string(&output)?);
    }

    if response.status >= 400 {
        return Err(anyhow!("http {}", response.status));
    }

    Ok(())
}

fn build_request_body(
    body_value: Option<String>,
    content_type: Option<String>,
) -> Result<(Option<Body>, Option<String>)> {
    let Some(body_value) = body_value else {
        return Ok((None, content_type));
    };
    let raw = read_body_input(&body_value)?;
    if content_type.is_some() {
        return Ok((Some(Body::Text(raw)), content_type));
    }
    let parsed: Result<Value> = serde_json::from_str(&raw).context("invalid JSON body");
    if let Ok(parsed) = parsed {
        return Ok((
            Some(Body::Json(parsed)),
            Some("application/json".to_string()),
        ));
    }
    Ok((Some(Body::Text(raw)), None))
}

fn parse_auth_mode(
    raw: Option<&String>,
    api_key: Option<&String>,
    token: Option<&String>,
) -> AuthMode {
    match raw.map(|v| v.as_str()) {
        Some("api-key") => AuthMode::ApiKey,
        Some("token") => AuthMode::Token,
        Some("auto") => AuthMode::Auto,
        _ => {
            if api_key.is_none() && token.is_some() {
                AuthMode::Token
            } else {
                AuthMode::Auto
            }
        }
    }
}

fn execute_with_auth(
    base_url: &str,
    api_key: Option<&String>,
    token: Option<&String>,
    auth_mode: AuthMode,
    headers: &[(String, String)],
    timeout: Option<u64>,
    method: &str,
    path: &str,
    query: &[(String, String)],
    body: Option<Body>,
    content_type: Option<&str>,
) -> Result<http::HttpResponse> {
    match auth_mode {
        AuthMode::ApiKey => {
            let client = HttpClient::new(
                base_url.to_string(),
                api_key.cloned(),
                None,
                headers.to_vec(),
                timeout,
            )?;
            client.execute(method, path, query, body, content_type)
        }
        AuthMode::Token => {
            let client = HttpClient::new(
                base_url.to_string(),
                None,
                token.cloned(),
                headers.to_vec(),
                timeout,
            )?;
            client.execute(method, path, query, body, content_type)
        }
        AuthMode::Auto => {
            if api_key.is_some() {
                let client = HttpClient::new(
                    base_url.to_string(),
                    api_key.cloned(),
                    None,
                    headers.to_vec(),
                    timeout,
                )?;
                let response = client.execute(method, path, query, body.clone(), content_type)?;
                if matches!(response.status, 401 | 403) && token.is_some() {
                    let client = HttpClient::new(
                        base_url.to_string(),
                        None,
                        token.cloned(),
                        headers.to_vec(),
                        timeout,
                    )?;
                    return client.execute(method, path, query, body, content_type);
                }
                return Ok(response);
            }
            let client = HttpClient::new(
                base_url.to_string(),
                None,
                token.cloned(),
                headers.to_vec(),
                timeout,
            )?;
            client.execute(method, path, query, body, content_type)
        }
    }
}

fn read_body_input(value: &str) -> Result<String> {
    if value == "@-" || value == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return Ok(buf);
    }
    if let Some(path) = value.strip_prefix('@') {
        return Ok(fs::read_to_string(path).context("read body file")?);
    }
    Ok(value.to_string())
}

fn parse_json_list(raw: &str) -> Result<Vec<String>> {
    let parsed: Value = serde_json::from_str(raw).context("invalid JSON list")?;
    let arr = parsed
        .as_array()
        .ok_or_else(|| anyhow!("expected JSON array"))?;
    let mut out = Vec::new();
    for value in arr {
        match value {
            Value::String(s) => out.push(s.clone()),
            Value::Number(n) => out.push(n.to_string()),
            Value::Bool(b) => out.push(b.to_string()),
            _ => out.push(value.to_string()),
        }
    }
    Ok(out)
}

fn parse_header_args(values: Option<clap::parser::ValuesRef<'_, String>>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(values) = values else {
        return out;
    };
    for raw in values {
        if let Some((k, v)) = split_header(raw) {
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

fn parse_kv_args(
    values: Option<clap::parser::ValuesRef<'_, String>>,
    label: &str,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let Some(values) = values else {
        return Ok(out);
    };
    for raw in values {
        if let Some((k, v)) = split_header(raw) {
            out.push((k.to_string(), v.to_string()));
        } else {
            return Err(anyhow!("invalid {label} param: {raw}"));
        }
    }
    Ok(out)
}

fn split_header(value: &str) -> Option<(&str, &str)> {
    if let Some((k, v)) = value.split_once(':') {
        return Some((k.trim(), v.trim()));
    }
    if let Some((k, v)) = value.split_once('=') {
        return Some((k.trim(), v.trim()));
    }
    None
}

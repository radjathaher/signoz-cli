mod command_tree;
mod http;

use anyhow::{Context, Result, anyhow};
use clap::{Arg, ArgAction, Command};
use command_tree::{CommandTree, Operation, ParamDef};
use http::{Body, HttpClient};
use serde_json::{Value, json};
use std::{env, fs, io::Read};
use urlencoding::encode;

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
        .unwrap_or_else(|| tree.base_url.clone());

    let api_key = matches
        .get_one::<String>("api-key")
        .cloned()
        .or_else(|| env::var("SIGNOZ_API_KEY").ok());

    let token = matches
        .get_one::<String>("token")
        .cloned()
        .or_else(|| env::var("SIGNOZ_TOKEN").ok());

    let headers = parse_header_args(matches.get_many::<String>("header"));
    let timeout = matches.get_one::<String>("timeout").and_then(|v| v.parse::<u64>().ok());

    let pretty = matches.get_flag("pretty");
    let raw = matches.get_flag("raw");

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

    let client = HttpClient::new(base_url, api_key, token, merged_headers, timeout)?;
    let response = client.execute(&op.method, &path, &query, body, content_type.as_deref())?;

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
            println!("    --{}  {} ({})", param.flag, param.schema_type, param.location);
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
            matches.get_many::<String>(&param.name).map(|vals| vals.cloned().collect::<Vec<_>>())
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

fn build_body(op: &Operation, matches: &clap::ArgMatches) -> Result<(Option<Body>, Option<String>)> {
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
        return Ok((Some(Body::Json(parsed)), Some(body_def.content_type.clone())));
    }

    Ok((Some(Body::Text(raw)), Some(body_def.content_type.clone())))
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
    let arr = parsed.as_array().ok_or_else(|| anyhow!("expected JSON array"))?;
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

fn split_header(value: &str) -> Option<(&str, &str)> {
    if let Some((k, v)) = value.split_once(':') {
        return Some((k.trim(), v.trim()));
    }
    if let Some((k, v)) = value.split_once('=') {
        return Some((k.trim(), v.trim()));
    }
    None
}

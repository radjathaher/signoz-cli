use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(dead_code)]
pub struct CommandTree {
    pub version: u32,
    pub base_url: String,
    pub resources: Vec<Resource>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(dead_code)]
pub struct Resource {
    pub name: String,
    pub ops: Vec<Operation>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(dead_code)]
pub struct Operation {
    pub name: String,
    pub method: String,
    pub path: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    pub params: Vec<ParamDef>,
    pub request_body: Option<RequestBodyDef>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(dead_code)]
pub struct ParamDef {
    pub param_name: String,
    pub name: String,
    pub flag: String,
    pub location: String,
    pub required: bool,
    pub schema_type: String,
    pub is_array: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(dead_code)]
pub struct RequestBodyDef {
    pub required: bool,
    pub content_type: String,
    pub schema_type: String,
}

pub fn load_command_tree() -> CommandTree {
    let raw = include_str!("../schemas/command_tree.json");
    serde_json::from_str(raw).expect("invalid command_tree.json")
}

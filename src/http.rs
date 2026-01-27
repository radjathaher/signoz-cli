use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::time::Duration;
use url::Url;

pub struct HttpClient {
    base_url: String,
    api_key: Option<String>,
    token: Option<String>,
    headers: Vec<(String, String)>,
    client: Client,
}

pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub content_type: String,
}

#[derive(Clone)]
pub enum Body {
    Json(Value),
    Text(String),
}

impl HttpClient {
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        token: Option<String>,
        headers: Vec<(String, String)>,
        timeout_secs: Option<u64>,
    ) -> Result<Self> {
        let mut builder = Client::builder().user_agent("signoz-cli");
        if let Some(secs) = timeout_secs {
            builder = builder.timeout(Duration::from_secs(secs));
        }
        let client = builder.build().context("build http client")?;
        Ok(Self {
            base_url,
            api_key,
            token,
            headers,
            client,
        })
    }

    pub fn execute(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<Body>,
        content_type: Option<&str>,
    ) -> Result<HttpResponse> {
        let url = build_url(&self.base_url, path, query)?;
        let mut headers = HeaderMap::new();

        if let Some(key) = &self.api_key {
            headers.insert(
                HeaderName::from_static("signoz-api-key"),
                HeaderValue::from_str(key).context("invalid api key header")?,
            );
        }
        if let Some(token) = &self.token {
            let mut value = token.clone();
            if !value.to_ascii_lowercase().starts_with("bearer ") {
                value = format!("Bearer {}", value);
            }
            headers.insert(
                HeaderName::from_static("authorization"),
                HeaderValue::from_str(&value).context("invalid token header")?,
            );
        }
        for (name, value) in &self.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).context("invalid header name")?;
            let header_value = HeaderValue::from_str(value).context("invalid header value")?;
            headers.insert(header_name, header_value);
        }

        let mut req = self
            .client
            .request(method.parse()?, url)
            .headers(headers);

        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }

        if let Some(body) = body {
            req = match body {
                Body::Json(value) => req.json(&value),
                Body::Text(value) => req.body(value),
            };
        }

        let resp = req.send().context("send request")?;
        let status = resp.status().as_u16();
        let headers_out = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<Vec<_>>();

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let text = resp.text().unwrap_or_default();
        let body = if content_type.contains("json") {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        } else {
            Value::String(text)
        };

        Ok(HttpResponse {
            status,
            headers: headers_out,
            body,
            content_type,
        })
    }
}

fn build_url(base_url: &str, path: &str, query: &[(String, String)]) -> Result<Url> {
    let mut base = base_url.trim_end_matches('/').to_string();
    let path = if path.starts_with('/') { path } else { &format!("/{path}") };
    base.push_str(path);
    let mut url = Url::parse(&base).context("invalid base url")?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    Ok(url)
}

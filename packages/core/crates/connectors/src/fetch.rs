//! Requests to the network: the catalog, a pack's archive, and the HTTP
//! connector's requests. Every call blocks, so none is made on an async task.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

/// Reads the bytes behind a URL, refusing more than `limit`, and saying how
/// far it got as `(received, total)` (`total` 0 when the server did not say).
pub trait Fetch: Send + Sync {
    fn get(
        &self,
        url: &str,
        timeout: Duration,
        limit: u64,
        progress: &dyn Fn(u64, u64),
    ) -> Result<Vec<u8>, String>;
}

/// One HTTP request, as the HTTP connector sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// What came back: the status, the headers by lower-case name, the body as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// The network, through `ureq` with the web's roots of trust.
#[derive(Debug, Clone, Default)]
pub struct Http;

fn agent(timeout: Duration, redirects: u32) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(redirects)
        .http_status_as_error(false)
        .build()
        .new_agent()
}

impl Fetch for Http {
    fn get(
        &self,
        url: &str,
        timeout: Duration,
        limit: u64,
        progress: &dyn Fn(u64, u64),
    ) -> Result<Vec<u8>, String> {
        let mut response = agent(timeout, 5)
            .get(url)
            .call()
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(format!("Downloading the pack failed with HTTP {status}"));
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let mut reader = response.body_mut().as_reader();
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(bytes);
            }
            bytes.extend_from_slice(&chunk[..n]);
            if bytes.len() as u64 > limit {
                return Err(format!(
                    "The pack is {} KB; Vorn installs at most {} MB",
                    (bytes.len() as f64 / 1024.0).round(),
                    limit / 1024 / 1024
                ));
            }
            progress(bytes.len() as u64, total);
        }
    }
}

impl Http {
    /// Sends `request` without following a redirect, which could carry a
    /// profile's secret to another host.
    pub fn send(&self, request: &Request, timeout: Duration) -> Result<Response, String> {
        let agent = agent(timeout, 0);
        let mut builder = ureq::http::Request::builder()
            .method(request.method.as_str())
            .uri(request.url.as_str());
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let sent = match &request.body {
            Some(body) => {
                let req = builder.body(body.clone()).map_err(|e| e.to_string())?;
                agent.run(req)
            }
            None => {
                let req = builder.body(()).map_err(|e| e.to_string())?;
                agent.run(req)
            }
        };
        let mut response = sent.map_err(|e| e.to_string())?;
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_lowercase(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .fold(BTreeMap::new(), |mut all, (k, v)| {
                all.entry(k)
                    .and_modify(|e: &mut String| {
                        e.push_str(", ");
                        e.push_str(&v);
                    })
                    .or_insert(v);
                all
            });
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok(Response {
            status: response.status().as_u16(),
            headers,
            body,
        })
    }
}

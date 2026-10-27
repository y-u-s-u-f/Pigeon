//! HTTP: the `Transport` the backend sends requests through, a fake one for
//! tests, and Google's HTTP batch format (up to 50 calls in one request).
//! The real transport (ureq), auth headers, retries, backoff and the quota
//! limiter come with W2.

use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    /// Absolute, with any query string.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Quota units the call costs (2.6).
    pub cost: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum TransportError {
    /// No connection, DNS, TLS or a timeout: worth retrying.
    Network(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Network(e) => write!(f, "network error: {e}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Sends one request and blocks for its response. Called off the main
/// thread.
pub trait Transport: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;
}

impl Request {
    pub fn get(url: impl Into<String>, cost: u32) -> Self {
        Self {
            method: "GET",
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            cost,
        }
    }

    pub fn json(
        method: &'static str,
        url: impl Into<String>,
        body: &serde_json::Value,
        cost: u32,
    ) -> Self {
        Self {
            method,
            url: url.into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string().into_bytes(),
            cost,
        }
    }

    /// The URL's path, without host or query.
    pub fn path(&self) -> &str {
        let rest = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, rest)| rest);
        let path = rest.find('/').map_or("/", |ix| &rest[ix..]);
        path.split('?').next().unwrap_or(path)
    }

    /// The query's pairs, undecoded.
    pub fn query(&self) -> Vec<(&str, &str)> {
        self.url
            .split_once('?')
            .map(|(_, q)| q)
            .unwrap_or_default()
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').unwrap_or((p, "")))
            .collect()
    }

    /// The path and query, as a batch part's request line wants them.
    fn path_and_query(&self) -> &str {
        let rest = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, rest)| rest);
        rest.find('/').map_or("/", |ix| &rest[ix..])
    }
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_slice(&self.body)
    }
}

// Tests' transport.

/// Answers requests from a script: each route matches a method, a path and
/// some query pairs, and replies with its responses in turn, the last one
/// again once the others are used. Records every request.
#[derive(Default)]
pub struct FakeTransport {
    routes: Mutex<Vec<Route>>,
    log: Mutex<Vec<Request>>,
}

struct Route {
    method: &'static str,
    path: String,
    query: Vec<(String, String)>,
    responses: VecDeque<(u16, String)>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reply to `method path` (with every pair in `query`) with
    /// `responses`, as (status, body), in order.
    pub fn on(
        self,
        method: &'static str,
        path: &str,
        query: &[(&str, &str)],
        responses: &[(u16, &str)],
    ) -> Self {
        self.routes.lock().unwrap().push(Route {
            method,
            path: path.into(),
            query: query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            responses: responses
                .iter()
                .map(|&(status, body)| (status, body.to_string()))
                .collect(),
        });
        self
    }

    /// Every request sent so far.
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
}

impl Transport for FakeTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.log.lock().unwrap().push(request.clone());
        let mut routes = self.routes.lock().unwrap();
        let query = request.query();
        let route = routes.iter_mut().find(|r| {
            r.method == request.method
                && r.path == request.path()
                && r.query
                    .iter()
                    .all(|(k, v)| query.contains(&(k.as_str(), v.as_str())))
        });
        let Some(route) = route else {
            return Ok(Response {
                status: 404,
                headers: Vec::new(),
                body: b"{\"error\":{\"code\":404,\"message\":\"no fake route\"}}".to_vec(),
            });
        };
        let (status, body) = if route.responses.len() > 1 {
            route.responses.pop_front().expect("checked")
        } else {
            route
                .responses
                .front()
                .cloned()
                .unwrap_or((200, String::new()))
        };
        if status == 0 {
            return Err(TransportError::Network("scripted failure".into()));
        }
        Ok(Response {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.into_bytes(),
        })
    }
}

// Google's HTTP batch: https://developers.google.com/gmail/api/guides/batch

/// Most calls Google takes in one batch is 100; it advises 50.
pub const BATCH_LIMIT: usize = 50;

/// One request carrying `requests` to `endpoint` (Gmail's is
/// `https://gmail.googleapis.com/batch/gmail/v1`). Parts are numbered from
/// 1 in order; costs add up.
pub fn encode_batch(endpoint: &str, requests: &[Request], boundary: &str) -> Request {
    let mut body = String::new();
    for (ix, request) in requests.iter().enumerate() {
        body.push_str(&format!(
            "--{boundary}\r\nContent-Type: application/http\r\nContent-ID: <item-{}>\r\n\r\n",
            ix + 1
        ));
        body.push_str(&format!(
            "{} {} HTTP/1.1\r\n",
            request.method,
            request.path_and_query()
        ));
        for (name, value) in &request.headers {
            body.push_str(&format!("{name}: {value}\r\n"));
        }
        if !request.body.is_empty() {
            body.push_str(&format!("Content-Length: {}\r\n", request.body.len()));
        }
        body.push_str("\r\n");
        body.push_str(&String::from_utf8_lossy(&request.body));
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    Request {
        method: "POST",
        url: endpoint.into(),
        headers: vec![(
            "Content-Type".into(),
            format!("multipart/mixed; boundary={boundary}"),
        )],
        body: body.into_bytes(),
        cost: requests.iter().map(|r| r.cost).sum(),
    }
}

/// The parts of a batch response, put back in the order of the requests
/// (by their `Content-ID: <response-item-N>`). A part missing from the
/// response, or one that doesn't parse, is `None`.
pub fn decode_batch(response: &Response, count: usize) -> Result<Vec<Option<Response>>, String> {
    let content_type = response.header("Content-Type").unwrap_or_default();
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
        .map(|b| b.trim_matches('"'))
        .ok_or("batch response without a boundary")?;
    let text = String::from_utf8_lossy(&response.body).replace("\r\n", "\n");
    let mut out: Vec<Option<Response>> = vec![None; count];
    let delimiter = format!("--{boundary}");
    for part in text.split(&delimiter).skip(1) {
        if part.starts_with("--") {
            break;
        }
        let Some((part_headers, http)) = part.trim_start_matches('\n').split_once("\n\n") else {
            continue;
        };
        let index = part_headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if !name.trim().eq_ignore_ascii_case("content-id") {
                return None;
            }
            let id = value.trim().trim_start_matches('<').trim_end_matches('>');
            id.rsplit('-').next()?.parse::<usize>().ok()
        });
        let Some(slot) = index.and_then(|n| n.checked_sub(1)).filter(|&n| n < count) else {
            continue;
        };
        out[slot] = parse_http(http);
    }
    Ok(out)
}

/// "HTTP/1.1 200 OK", headers, a blank line, the body.
fn parse_http(text: &str) -> Option<Response> {
    let (head, body) = text.split_once("\n\n").unwrap_or((text, ""));
    let mut lines = head.lines();
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Some(Response {
        status,
        headers,
        body: body.trim_end_matches('\n').as_bytes().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GMAIL_BATCH: &str = "https://gmail.googleapis.com/batch/gmail/v1";

    #[test]
    fn batch_request_encodes_each_call() {
        let requests = [
            Request::get(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/t1?format=metadata",
                40,
            ),
            Request::json(
                "POST",
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/t2/modify",
                &serde_json::json!({ "removeLabelIds": ["INBOX"] }),
                10,
            ),
        ];
        let batch = encode_batch(GMAIL_BATCH, &requests, "batch_pigeon");
        assert_eq!(batch.method, "POST");
        assert_eq!(batch.url, GMAIL_BATCH);
        assert_eq!(batch.cost, 50);
        assert_eq!(
            batch.headers,
            [(
                "Content-Type".to_string(),
                "multipart/mixed; boundary=batch_pigeon".to_string()
            )]
        );
        let body = String::from_utf8(batch.body).unwrap();
        assert_eq!(
            body,
            "--batch_pigeon\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <item-1>\r\n\
             \r\n\
             GET /gmail/v1/users/me/threads/t1?format=metadata HTTP/1.1\r\n\
             \r\n\
             \r\n\
             --batch_pigeon\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <item-2>\r\n\
             \r\n\
             POST /gmail/v1/users/me/threads/t2/modify HTTP/1.1\r\n\
             Content-Type: application/json\r\n\
             Content-Length: 28\r\n\
             \r\n\
             {\"removeLabelIds\":[\"INBOX\"]}\r\n\
             --batch_pigeon--\r\n"
        );
    }

    #[test]
    fn batch_response_decodes_in_request_order() {
        let fixture = include_str!("../../tests/fixtures/google/gmail/batch_response.txt");
        let response = Response {
            status: 200,
            headers: vec![(
                "Content-Type".into(),
                "multipart/mixed; boundary=batch_fixture".into(),
            )],
            body: fixture.as_bytes().to_vec(),
        };
        let parts = decode_batch(&response, 3).unwrap();
        // The fixture answers item 2 first.
        let first = parts[0].as_ref().unwrap();
        assert_eq!(first.status, 200);
        assert_eq!(
            first.header("content-type"),
            Some("application/json; charset=UTF-8")
        );
        let thread: serde_json::Value = first.json().unwrap();
        assert_eq!(thread["id"], "thread-a1");
        let second = parts[1].as_ref().unwrap();
        assert_eq!(second.status, 404);
        // Item 3 is missing from the response.
        assert!(parts[2].is_none());
    }

    #[test]
    fn fake_transport_scripts_and_records() {
        let fake = FakeTransport::new().on(
            "GET",
            "/gmail/v1/users/me/history",
            &[("startHistoryId", "7")],
            &[(503, ""), (200, "{\"historyId\":\"9\"}")],
        );
        let request = Request::get(
            "https://gmail.googleapis.com/gmail/v1/users/me/history?startHistoryId=7",
            2,
        );
        assert_eq!(fake.send(&request).unwrap().status, 503);
        assert_eq!(fake.send(&request).unwrap().status, 200);
        assert_eq!(fake.send(&request).unwrap().status, 200);
        let other = Request::get("https://gmail.googleapis.com/gmail/v1/users/me/history", 2);
        assert_eq!(fake.send(&other).unwrap().status, 404);
        assert_eq!(fake.requests().len(), 4);
        assert_eq!(request.path(), "/gmail/v1/users/me/history");
    }
}

//! The writing model: an OpenAI-compatible `chat/completions`, always
//! streamed (SSE). A streamed request answered 400 is sent once more
//! without streaming, and streaming stays off until restart. JSON answers
//! come from a plain prompt, checked by the request's `validate`, with one
//! retry. An answer cut off by the token limit is a failure.

use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};

use serde_json::{Value, json};

use super::{CallError, Feature, HttpRequest, REASONING_ROOM, Transport, config::Endpoint};

/// Reasoning effort, which the endpoint reads from a model-name suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn suffix(self) -> &'static str {
        match self {
            Effort::Low => "-low",
            Effort::Medium => "-medium",
            Effort::High => "-high",
        }
    }
}

/// Every suffix the endpoint knows, longest first where one ends another.
const SUFFIXES: [&str; 5] = ["-xhigh", "-max", "-low", "-medium", "-high"];

/// The one table of efforts. Anything else (Test Connection, the check
/// script) goes low.
pub fn effort(feature: Feature) -> Effort {
    match feature {
        Feature::Line | Feature::Replies | Feature::AskTerms => Effort::Low,
        Feature::Summary | Feature::CatchUp | Feature::Clean | Feature::DoIt => Effort::Medium,
        Feature::Ask => Effort::High,
        _ => Effort::Low,
    }
}

/// `PIGEON_AI_MODEL` without a known effort suffix: code picks the effort.
pub fn base_model(name: &str) -> &str {
    SUFFIXES
        .iter()
        .find_map(|s| name.strip_suffix(s).filter(|base| !base.is_empty()))
        .unwrap_or(name)
}

/// The model name a request for `feature` sends.
pub fn model_for(base: &str, feature: Feature) -> String {
    format!("{base}{}", effort(feature).suffix())
}

#[derive(Clone, Debug)]
pub struct Request {
    pub system: String,
    pub user: String,
    /// Tokens the answer may take; `REASONING_ROOM` is added when sent.
    pub max_output: u32,
    /// For JSON answers: the answer as it should be kept, or `None` when
    /// it doesn't pass.
    pub validate: Option<fn(&str) -> Option<String>>,
}

impl Request {
    pub fn new(system: impl Into<String>, user: impl Into<String>, max_output: u32) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
            max_output,
            validate: None,
        }
    }

    pub fn body(&self, model: &str, stream: bool) -> Value {
        let mut body = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": self.system },
                { "role": "user", "content": self.user },
            ],
            "max_tokens": self.max_output + REASONING_ROOM,
            "stream": stream,
        });
        if stream {
            body["stream_options"] = json!({ "include_usage": true });
        }
        body
    }
}

/// A whole answer, with what it cost.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    pub finish: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
}

/// One thing a stream said.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    Finish(String),
    Usage { tokens_in: u32, tokens_out: u32 },
    Done,
}

/// Server-sent events, fed in chunks as they arrive. Lines may split
/// anywhere, even inside a UTF-8 character.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                events.push(Event::Done);
            } else if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                events.extend(chunk_events(&chunk));
            }
        }
        events
    }
}

fn chunk_events(chunk: &Value) -> Vec<Event> {
    let mut events = Vec::new();
    let choice = &chunk["choices"][0];
    if let Some(text) = choice["delta"]["content"].as_str()
        && !text.is_empty()
    {
        events.push(Event::Text(text.to_string()));
    }
    if let Some(reason) = choice["finish_reason"].as_str() {
        events.push(Event::Finish(reason.to_string()));
    }
    if let Some((tokens_in, tokens_out)) = usage(&chunk["usage"]) {
        events.push(Event::Usage {
            tokens_in,
            tokens_out,
        });
    }
    events
}

fn usage(usage: &Value) -> Option<(u32, u32)> {
    Some((
        usage["prompt_tokens"].as_u64()? as u32,
        usage["completion_tokens"].as_u64()? as u32,
    ))
}

/// A non-streamed answer.
pub fn parse_plain(body: &[u8]) -> Result<Reply, CallError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| CallError::Invalid("not JSON".into()))?;
    let choice = &value["choices"][0];
    let text = choice["message"]["content"]
        .as_str()
        .ok_or_else(|| CallError::Invalid("no answer".into()))?;
    let (tokens_in, tokens_out) = usage(&value["usage"]).unwrap_or_default();
    Ok(Reply {
        text: text.to_string(),
        finish: choice["finish_reason"].as_str().map(str::to_string),
        tokens_in,
        tokens_out,
    })
}

/// Send `request` to `model` (with its effort suffix), streaming while
/// `streaming` holds, and validate a JSON answer. `stop` is checked before sending and per chunk; `on_text` gets
/// each piece of a streamed answer.
pub fn run(
    transport: &dyn Transport,
    endpoint: &Endpoint,
    model: &str,
    request: &Request,
    streaming: &AtomicBool,
    stop: &dyn Fn() -> bool,
    on_text: &mut dyn FnMut(&str),
) -> Result<Reply, CallError> {
    let mut reply = call(
        transport, endpoint, model, request, streaming, stop, on_text,
    )?;
    let Some(validate) = request.validate else {
        return Ok(reply);
    };
    if let Some(text) = validate(&reply.text) {
        reply.text = text;
        return Ok(reply);
    }
    let (spent_in, spent_out) = (reply.tokens_in, reply.tokens_out);
    let mut again = call(
        transport, endpoint, model, request, streaming, stop, on_text,
    )?;
    again.tokens_in += spent_in;
    again.tokens_out += spent_out;
    match validate(&again.text) {
        Some(text) => {
            again.text = text;
            Ok(again)
        }
        None => Err(CallError::Invalid("the answer didn't parse".into())),
    }
}

fn call(
    transport: &dyn Transport,
    endpoint: &Endpoint,
    model: &str,
    request: &Request,
    streaming: &AtomicBool,
    stop: &dyn Fn() -> bool,
    on_text: &mut dyn FnMut(&str),
) -> Result<Reply, CallError> {
    if stop() {
        return Err(CallError::Cancelled);
    }
    let stream = streaming.load(Ordering::Relaxed);
    let response = transport.post(&HttpRequest {
        url: endpoint.url("chat/completions"),
        key: &endpoint.key,
        body: request.body(model, stream).to_string().into_bytes(),
        total: None,
    })?;
    if response.status == 400 && stream {
        streaming.store(false, Ordering::Relaxed);
        crate::logging::info("ai: streaming refused; sending without it until restart");
        return call(
            transport, endpoint, model, request, streaming, stop, on_text,
        );
    }
    if !(200..300).contains(&response.status) {
        return Err(CallError::Status(response.status));
    }
    let reply = if stream {
        read_stream(response.body, stop, on_text)?
    } else {
        let mut body = Vec::new();
        let mut reader = response.body;
        reader.read_to_end(&mut body).map_err(CallError::from_io)?;
        parse_plain(&body)?
    };
    if reply.finish.as_deref() == Some("length") {
        return Err(CallError::CutOff);
    }
    Ok(reply)
}

fn read_stream(
    mut body: Box<dyn Read>,
    stop: &dyn Fn() -> bool,
    on_text: &mut dyn FnMut(&str),
) -> Result<Reply, CallError> {
    let mut parser = SseParser::default();
    let mut reply = Reply::default();
    let mut done = false;
    let mut chunk = [0u8; 4096];
    while !done {
        if stop() {
            return Err(CallError::Cancelled);
        }
        let n = body.read(&mut chunk).map_err(CallError::from_io)?;
        if n == 0 {
            break;
        }
        for event in parser.feed(&chunk[..n]) {
            match event {
                Event::Text(text) => {
                    on_text(&text);
                    reply.text.push_str(&text);
                }
                Event::Finish(reason) => reply.finish = Some(reason),
                Event::Usage {
                    tokens_in,
                    tokens_out,
                } => (reply.tokens_in, reply.tokens_out) = (tokens_in, tokens_out),
                Event::Done => done = true,
            }
        }
    }
    if reply.finish.is_none() {
        return Err(CallError::Network("the stream ended early".into()));
    }
    Ok(reply)
}

/// The first `{…}` in an answer, as JSON: models wrap it in prose or
/// code fences now and then.
pub fn json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let value: Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    value.is_object().then_some(value)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ai::FakeTransport;

    const PLAIN: &[u8] = include_bytes!("testdata/llm_plain.json");
    const STREAM: &[u8] = include_bytes!("testdata/llm_stream.txt");

    fn endpoint() -> Endpoint {
        Endpoint {
            base_url: "https://llm.example/v1/".into(),
            key: "fake-key".into(),
            model: "test-model".into(),
        }
    }

    #[test]
    fn the_stream_fixture_parses_in_any_chunking() {
        for size in [1, 7, 64, STREAM.len()] {
            let mut parser = SseParser::default();
            let events: Vec<Event> = STREAM.chunks(size).flat_map(|c| parser.feed(c)).collect();
            let text: String = events
                .iter()
                .filter_map(|e| match e {
                    Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(text, "1, 2, 3, 4, 5, 6, 7, 8, 9, 10");
            assert!(events.contains(&Event::Finish("stop".into())));
            assert!(events.contains(&Event::Usage {
                tokens_in: 28,
                tokens_out: 32
            }));
            assert_eq!(events.last(), Some(&Event::Done));
        }
    }

    #[test]
    fn the_plain_fixture_parses() {
        let reply = parse_plain(PLAIN).unwrap();
        assert_eq!(reply.text, "OK");
        assert_eq!((reply.tokens_in, reply.tokens_out), (22, 5));
        assert_eq!(reply.finish.as_deref(), Some("stop"));
    }

    #[test]
    fn streams_with_usage_and_reasoning_room() {
        let fake = FakeTransport::new(vec![Ok((200, STREAM.to_vec()))]);
        let streaming = AtomicBool::new(true);
        let seen = Mutex::new(String::new());
        let reply = run(
            &fake,
            &endpoint(),
            "test-model",
            &Request::new("s", "u", 60),
            &streaming,
            &|| false,
            &mut |t| seen.lock().unwrap().push_str(t),
        )
        .unwrap();
        assert_eq!(reply.text, *seen.lock().unwrap());
        assert_eq!(reply.tokens_out, 32);
        let sent = fake.sent();
        assert_eq!(sent[0].0, "https://llm.example/v1/chat/completions");
        let body: Value = serde_json::from_str(&sent[0].1).unwrap();
        assert_eq!(body["max_tokens"], 60 + REASONING_ROOM);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn a_400_retries_once_without_streaming_and_stays_off() {
        let fake = FakeTransport::new(vec![
            Ok((400, b"{}".to_vec())),
            Ok((200, PLAIN.to_vec())),
            Ok((200, PLAIN.to_vec())),
        ]);
        let streaming = AtomicBool::new(true);
        let req = Request::new("s", "u", 5);
        let reply = run(
            &fake,
            &endpoint(),
            "test-model",
            &req,
            &streaming,
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(reply.text, "OK");
        assert!(!streaming.load(Ordering::Relaxed));
        run(
            &fake,
            &endpoint(),
            "test-model",
            &req,
            &streaming,
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        let bodies: Vec<Value> = fake
            .sent()
            .iter()
            .map(|(_, b)| serde_json::from_str(b).unwrap())
            .collect();
        let streamed: Vec<bool> = bodies.iter().map(|b| b["stream"] == true).collect();
        assert_eq!(streamed, [true, false, false]);
        // Without streaming, a 400 is the job's failure.
        let fake = FakeTransport::new(vec![Ok((400, b"{}".to_vec()))]);
        let err = run(
            &fake,
            &endpoint(),
            "test-model",
            &req,
            &streaming,
            &|| false,
            &mut |_| {},
        );
        assert_eq!(err.unwrap_err(), CallError::Status(400));
    }

    #[test]
    fn cut_off_answers_fail() {
        let cut = String::from_utf8(PLAIN.to_vec())
            .unwrap()
            .replace("\"stop\"", "\"length\"");
        let fake = FakeTransport::new(vec![Ok((200, cut.into_bytes()))]);
        let streaming = AtomicBool::new(false);
        let req = Request::new("s", "u", 5);
        let err = run(
            &fake,
            &endpoint(),
            "test-model",
            &req,
            &streaming,
            &|| false,
            &mut |_| {},
        );
        assert_eq!(err.unwrap_err(), CallError::CutOff);
    }

    #[test]
    fn json_answers_get_one_retry() {
        let answer = |text: &str| {
            json!({"choices": [{"finish_reason": "stop", "message": {"content": text}}]})
                .to_string()
                .into_bytes()
        };
        let mut req = Request::new("s", "u", 50);
        req.validate = Some(|t| json_object(t).map(|v| v.to_string()));
        let streaming = AtomicBool::new(false);
        let fake = FakeTransport::new(vec![
            Ok((200, answer("Sure!"))),
            Ok((200, answer("```json\n{\"a\": 1}\n```"))),
        ]);
        let reply = run(
            &fake,
            &endpoint(),
            "test-model",
            &req,
            &streaming,
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(reply.text, r#"{"a":1}"#);
        let fake = FakeTransport::new(vec![Ok((200, answer("no"))), Ok((200, answer("still no")))]);
        assert!(matches!(
            run(
                &fake,
                &endpoint(),
                "test-model",
                &req,
                &streaming,
                &|| false,
                &mut |_| {}
            ),
            Err(CallError::Invalid(_))
        ));
    }

    #[test]
    fn known_effort_suffixes_are_stripped() {
        assert_eq!(base_model("qwen3"), "qwen3");
        assert_eq!(base_model("qwen3-low"), "qwen3");
        assert_eq!(base_model("qwen3-xhigh"), "qwen3");
        assert_eq!(base_model("qwen3-max"), "qwen3");
        assert_eq!(base_model("qwen3-high-low"), "qwen3-high");
        assert_eq!(base_model("my-lowkey-model"), "my-lowkey-model");
        assert_eq!(base_model("glow"), "glow");
        assert_eq!(base_model("-low"), "-low");
    }

    #[test]
    fn each_feature_sends_its_effort() {
        let cases = [
            (Feature::Line, "test-model-low"),
            (Feature::Replies, "test-model-low"),
            (Feature::AskTerms, "test-model-low"),
            (Feature::Summary, "test-model-medium"),
            (Feature::CatchUp, "test-model-medium"),
            (Feature::Clean, "test-model-medium"),
            (Feature::DoIt, "test-model-medium"),
            (Feature::Ask, "test-model-high"),
            (Feature::Test, "test-model-low"),
        ];
        for (feature, want) in cases {
            let fake = FakeTransport::new(vec![Ok((200, PLAIN.to_vec()))]);
            let model = model_for("test-model", feature);
            let req = Request::new("s", "u", 5);
            run(
                &fake,
                &endpoint(),
                &model,
                &req,
                &AtomicBool::new(false),
                &|| false,
                &mut |_| {},
            )
            .unwrap();
            let body: Value = serde_json::from_str(&fake.sent()[0].1).unwrap();
            assert_eq!(body["model"], want, "{feature:?}");
        }
    }

    #[test]
    fn a_stop_closes_the_stream() {
        let fake = FakeTransport::new(vec![Ok((200, STREAM.to_vec()))]);
        let streaming = AtomicBool::new(true);
        let req = Request::new("s", "u", 5);
        let err = run(
            &fake,
            &endpoint(),
            "test-model",
            &req,
            &streaming,
            &|| true,
            &mut |_| {},
        );
        assert_eq!(err.unwrap_err(), CallError::Cancelled);
        assert!(fake.sent().is_empty());
    }
}

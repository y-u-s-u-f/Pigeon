//! Jev, through OpenRouter's System One API (`POST {base}/v1/systemone`):
//! typed questions about one `state`, answered with probabilities. A
//! missing or invalid answer is `None`: no result, never "no".

use std::{collections::BTreeMap, io::Read, time::Duration};

use serde::{Deserialize, Serialize, ser::SerializeMap};
use serde_json::Value;

use super::{CallError, HttpRequest, Transport, config::Endpoint};

/// The whole call, connecting included.
pub const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    Noul {
        instructions: String,
    },
    /// Options as (id, description), in the order sent: Jev leans to the
    /// first one, so a "none" option goes there.
    Choice {
        instructions: String,
        options: Vec<(String, String)>,
    },
    /// Levels from 0 up.
    Score {
        instructions: String,
        levels: Vec<String>,
    },
}

impl Question {
    pub fn noul(instructions: impl Into<String>) -> Self {
        Question::Noul {
            instructions: instructions.into(),
        }
    }
}

/// One state and its questions, by stable id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub state: String,
    pub questions: Vec<(String, Question)>,
}

impl Request {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            questions: Vec::new(),
        }
    }

    pub fn ask(mut self, id: impl Into<String>, question: Question) -> Self {
        self.questions.push((id.into(), question));
        self
    }

    /// What the API would refuse, caught before sending.
    pub fn check(&self) -> Result<(), String> {
        if self.questions.is_empty() {
            return Err("no questions".into());
        }
        let mut ids = std::collections::HashSet::new();
        for (id, question) in &self.questions {
            if id.is_empty() || !ids.insert(id.as_str()) {
                return Err(format!("question id {id:?} is empty or repeated"));
            }
            match question {
                Question::Noul { .. } => {}
                Question::Choice { options, .. } => {
                    let unique: std::collections::HashSet<_> =
                        options.iter().map(|(id, _)| id).collect();
                    if !(2..=255).contains(&options.len()) || unique.len() != options.len() {
                        return Err(format!("{id}: a choice needs 2–255 distinct options"));
                    }
                }
                Question::Score { levels, .. } => {
                    if !(2..=10).contains(&levels.len()) {
                        return Err(format!("{id}: a score needs 2–10 levels"));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn body(&self, model: &str) -> Vec<u8> {
        #[derive(Serialize)]
        struct Body<'a> {
            model: &'a str,
            state: &'a str,
            questions: Questions<'a>,
        }
        serde_json::to_vec(&Body {
            model,
            state: &self.state,
            questions: Questions(&self.questions),
        })
        .expect("questions serialize")
    }

    /// The answers that fit a question asked: the right type, a choice
    /// among its options, a score in range, probabilities in 0–1.
    pub fn keep_valid(&self, answers: BTreeMap<String, Answer>) -> BTreeMap<String, Answer> {
        answers
            .into_iter()
            .filter(|(id, answer)| {
                let Some((_, question)) = self.questions.iter().find(|(q, _)| q == id) else {
                    return false;
                };
                match (question, answer) {
                    (Question::Noul { .. }, Answer::Noul { .. }) => answer.yes().is_some(),
                    (Question::Choice { options, .. }, Answer::Choice { .. }) => answer
                        .choice()
                        .is_some_and(|(c, _)| options.iter().any(|(o, _)| o == c)),
                    (Question::Score { levels, .. }, Answer::Score { .. }) => answer
                        .score()
                        .is_some_and(|s| s <= (levels.len() - 1) as f64),
                    _ => false,
                }
            })
            .collect()
    }
}

/// Questions as a JSON object in the order given, and choice options too.
struct Questions<'a>(&'a [(String, Question)]);

impl Serialize for Questions<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (id, question) in self.0 {
            let body = match question {
                Question::Noul { instructions } => QuestionBody {
                    kind: "noul",
                    instructions,
                    criteria: None,
                },
                Question::Choice {
                    instructions,
                    options,
                } => QuestionBody {
                    kind: "choice",
                    instructions,
                    criteria: Some(Criteria::Options(Options(options))),
                },
                Question::Score {
                    instructions,
                    levels,
                } => QuestionBody {
                    kind: "score",
                    instructions,
                    criteria: Some(Criteria::Levels(levels)),
                },
            };
            map.serialize_entry(id, &body)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct QuestionBody<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    instructions: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    criteria: Option<Criteria<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Criteria<'a> {
    Options(Options<'a>),
    Levels(&'a [String]),
}

struct Options<'a>(&'a [(String, String)]);

impl Serialize for Options<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (id, description) in self.0 {
            map.serialize_entry(id, description)?;
        }
        map.end()
    }
}

/// One answer, as the API gives it. Every field is optional so a bad one
/// reads as missing rather than failing the rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        #[serde(default)]
        noul: Option<f64>,
    },
    Choice {
        #[serde(default)]
        choice: Option<String>,
        #[serde(default)]
        confidence: Option<f64>,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
    },
    Score {
        #[serde(default)]
        score: Option<f64>,
        #[serde(default)]
        confidence: Option<f64>,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
    },
    #[serde(other)]
    Other,
}

fn probability(p: Option<f64>) -> Option<f64> {
    p.filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
}

impl Answer {
    /// The probability of "yes" for a noul.
    pub fn yes(&self) -> Option<f64> {
        match self {
            Answer::Noul { noul } => probability(*noul),
            _ => None,
        }
    }

    /// The option picked and the confidence in it.
    pub fn choice(&self) -> Option<(&str, f64)> {
        match self {
            Answer::Choice {
                choice: Some(choice),
                confidence,
                probabilities,
            } => {
                let confidence = probability(*confidence)
                    .or_else(|| probability(probabilities.get(choice).copied()))?;
                Some((choice.as_str(), confidence))
            }
            _ => None,
        }
    }

    /// The expected level of a score.
    pub fn score(&self) -> Option<f64> {
        match self {
            Answer::Score { score, .. } => score.filter(|s| s.is_finite() && *s >= 0.0),
            _ => None,
        }
    }

    /// One option's or level's probability.
    pub fn probability(&self, key: &str) -> Option<f64> {
        match self {
            Answer::Choice { probabilities, .. } | Answer::Score { probabilities, .. } => {
                probability(probabilities.get(key).copied())
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub answers: BTreeMap<String, Answer>,
    pub tokens_in: u32,
    pub tokens_out: u32,
}

/// The API's answer, with each answer read on its own.
pub fn parse(body: &[u8]) -> Result<Response, CallError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| CallError::Invalid("not JSON".into()))?;
    let answers = value["answers"]
        .as_object()
        .ok_or_else(|| CallError::Invalid("no answers".into()))?;
    Ok(Response {
        answers: answers
            .iter()
            .filter_map(|(id, a)| Some((id.clone(), serde_json::from_value(a.clone()).ok()?)))
            .collect(),
        tokens_in: value["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
        tokens_out: value["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
    })
}

/// Stored answers (`ai_results.value`) back; nothing when they don't read.
pub fn answers_from(value: &str) -> BTreeMap<String, Answer> {
    serde_json::from_str(value).unwrap_or_default()
}

pub fn run(
    transport: &dyn Transport,
    endpoint: &Endpoint,
    request: &Request,
) -> Result<Response, CallError> {
    request.check().map_err(CallError::Invalid)?;
    let response = transport.post(&HttpRequest {
        url: endpoint.url("v1/systemone"),
        key: &endpoint.key,
        body: request.body(&endpoint.model),
        total: Some(TIMEOUT),
    })?;
    if !(200..300).contains(&response.status) {
        return Err(CallError::Status(response.status));
    }
    let mut body = Vec::new();
    let mut reader = response.body;
    reader.read_to_end(&mut body).map_err(CallError::from_io)?;
    let mut parsed = parse(&body)?;
    parsed.answers = request.keep_valid(parsed.answers);
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::FakeTransport;

    const NOUL: &[u8] = include_bytes!("testdata/jev_noul.json");
    const CHOICE: &[u8] = include_bytes!("testdata/jev_choice.json");
    const SCORE: &[u8] = include_bytes!("testdata/jev_score.json");
    const MULTI: &[u8] = include_bytes!("testdata/jev_multi.json");

    fn options(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter()
            .map(|i| (i.to_string(), format!("{i}.")))
            .collect()
    }

    #[test]
    fn fixtures_parse() {
        let noul = parse(NOUL).unwrap();
        assert_eq!(noul.answers["needs_reply"].yes(), Some(0.98));
        assert_eq!((noul.tokens_in, noul.tokens_out), (317, 21));
        let choice = parse(CHOICE).unwrap();
        assert_eq!(choice.answers["label"].choice(), Some(("social", 1.0)));
        assert_eq!(choice.answers["label"].probability("work"), Some(0.0));
        let score = parse(SCORE).unwrap();
        assert_eq!(score.answers["urgency"].score(), Some(1.19));
        assert_eq!(score.answers["urgency"].probability("1"), Some(0.79));
        let multi = parse(MULTI).unwrap();
        assert_eq!(multi.answers.len(), 3);
        assert_eq!(multi.answers["important"].yes(), Some(0.82));
    }

    #[test]
    fn yes_is_none_when_missing_or_invalid() {
        let read = |json: &str| serde_json::from_str::<Answer>(json).unwrap();
        assert_eq!(read(r#"{"type":"noul"}"#).yes(), None);
        assert_eq!(read(r#"{"type":"noul","noul":null}"#).yes(), None);
        assert_eq!(read(r#"{"type":"noul","noul":1.5}"#).yes(), None);
        assert_eq!(read(r#"{"type":"noul","noul":0}"#).yes(), Some(0.0));
        assert_eq!(read(r#"{"type":"choice","choice":"a"}"#).yes(), None);
        assert_eq!(read(r#"{"type":"box"}"#), Answer::Other);
        // A broken answer is missing; the others stay.
        let body =
            br#"{"answers":{"a":{"type":"noul","noul":"high"},"b":{"type":"noul","noul":0.4}}}"#;
        let parsed = parse(body).unwrap();
        assert!(!parsed.answers.contains_key("a"));
        assert_eq!(parsed.answers["b"].yes(), Some(0.4));
    }

    #[test]
    fn questions_are_checked_before_sending() {
        let choice = |n: usize| Question::Choice {
            instructions: "Which?".into(),
            options: (0..n).map(|i| (format!("o{i}"), String::new())).collect(),
        };
        let score = |n: usize| Question::Score {
            instructions: "How much?".into(),
            levels: (0..n).map(|i| i.to_string()).collect(),
        };
        assert!(Request::new("s").check().is_err());
        assert!(Request::new("s").ask("c", choice(1)).check().is_err());
        assert!(Request::new("s").ask("c", choice(2)).check().is_ok());
        assert!(Request::new("s").ask("c", choice(255)).check().is_ok());
        assert!(Request::new("s").ask("c", choice(256)).check().is_err());
        assert!(Request::new("s").ask("s", score(1)).check().is_err());
        assert!(Request::new("s").ask("s", score(10)).check().is_ok());
        assert!(Request::new("s").ask("s", score(11)).check().is_err());
        let twice = Request::new("s")
            .ask("a", Question::noul("x"))
            .ask("a", Question::noul("y"));
        assert!(twice.check().is_err());
    }

    #[test]
    fn options_keep_their_order() {
        let request = Request::new("state").ask(
            "label",
            Question::Choice {
                instructions: "Which label?".into(),
                options: options(&["none", "work", "receipts", "a-social"]),
            },
        );
        let body = String::from_utf8(request.body("m")).unwrap();
        let none = body.find("\"none\"").unwrap();
        let work = body.find("\"work\"").unwrap();
        let social = body.find("\"a-social\"").unwrap();
        assert!(none < work && work < social);
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["questions"]["label"]["type"], "choice");
        assert_eq!(value["state"], "state");
    }

    #[test]
    fn only_answers_to_questions_asked_are_kept() {
        let request = Request::new("s")
            .ask(
                "label",
                Question::Choice {
                    instructions: "?".into(),
                    options: options(&["none", "work"]),
                },
            )
            .ask(
                "urgency",
                Question::Score {
                    instructions: "?".into(),
                    levels: vec!["a".into(), "b".into()],
                },
            );
        // "social" isn't an option; 1.19 is past the last level; "x" wasn't asked.
        let mut answers = parse(CHOICE).unwrap().answers;
        answers.extend(parse(SCORE).unwrap().answers);
        answers.extend(parse(NOUL).unwrap().answers);
        assert!(request.keep_valid(answers).is_empty());
    }

    #[test]
    fn runs_against_the_url_with_a_total_timeout() {
        let fake = FakeTransport::new(vec![Ok((200, NOUL.to_vec())), Ok((401, Vec::new()))]);
        let endpoint = Endpoint {
            base_url: "https://openrouter.ai/api/".into(),
            key: "fake".into(),
            model: "typesafe/jev-1.14".into(),
        };
        let request = Request::new("s").ask("needs_reply", Question::noul("Asks me something?"));
        let response = run(&fake, &endpoint, &request).unwrap();
        assert_eq!(response.answers["needs_reply"].yes(), Some(0.98));
        assert_eq!(fake.sent()[0].0, "https://openrouter.ai/api/v1/systemone");
        assert_eq!(fake.totals(), [Some(TIMEOUT)]);
        assert_eq!(
            run(&fake, &endpoint, &request).unwrap_err(),
            CallError::Status(401)
        );
    }
}

//! The check script: `cargo test ai_check -- --ignored --nocapture`.
//! Runs each feature over the eval set and the demo data against the real
//! models (keys from `.env`), and writes Markdown per feature to
//! `CHECK_DIR`. With `PIGEON_AI_CHECK_REAL=1`, also the Jev questions over
//! the latest 200 cached Inbox messages of the real store. Prints paths
//! and counts only, never a key.
use std::{fmt::Write as _, path::Path};

use super::*;

const CHECK_DIR: &str =
    "/private/var/folders/sc/26v_9jsx0s9_fkjttv6jzjdh0000gn/T/opencode/ai-check";

#[derive(Deserialize)]
struct Eval {
    examples: Vec<Example>,
}

#[derive(Deserialize)]
struct Example {
    id: String,
    feature: String,
    from: String,
    subject: String,
    body: String,
    #[serde(default)]
    options: Vec<(String, String)>,
    expect: serde_json::Value,
    #[serde(default)]
    adversarial: Option<String>,
}

fn message(e: &Example) -> text::Message {
    text::Message {
        id: e.id.clone(),
        thread_id: String::new(),
        from: e.from.clone(),
        subject: e.subject.clone(),
        date: chrono::DateTime::parse_from_rfc3339("2026-10-20T09:00:00Z").unwrap(),
        body: e.body.clone(),
    }
}

fn write(dir: &Path, name: &str, text: &str) {
    let path = dir.join(format!("{name}.md"));
    std::fs::write(&path, text).unwrap();
    println!("ai_check: wrote {}", path.display());
}

/// Both Jev requests the pass makes about a message, as it makes them:
/// request A from what a notification shows, request B from the body.
/// Both ask `newsletter`, so they're kept apart.
struct Nouls {
    a: BTreeMap<String, jev::Answer>,
    b: BTreeMap<String, jev::Answer>,
}

impl Nouls {
    fn yes(&self, request: char, id: &str) -> Option<f64> {
        let answers = if request == 'a' { &self.a } else { &self.b };
        answers.get(id).and_then(|a| a.yes())
    }

    /// Request A or B's answer, as the report shows it.
    fn cell(&self, request: char, id: &str) -> String {
        fmt(self.yes(request, id))
    }

    /// AI-4: the Needs Reply marker, from request B's answers alone.
    fn marker(&self) -> &'static str {
        let shows = self
            .yes('b', "needs_reply")
            .is_some_and(|p| p >= pass::NEEDS_REPLY)
            && self
                .yes('b', "newsletter")
                .is_none_or(|p| p < pass::NEEDS_REPLY_NEWSLETTER);
        if shows { "**yes**" } else { "" }
    }

    /// AI-5: whether Important Only keeps the notification silent.
    fn silent(&self) -> &'static str {
        let answer = notice::Answer {
            important: self.yes('a', "important"),
            newsletter: self.yes('a', "newsletter"),
        };
        if notice::silent(answer) {
            "**yes**"
        } else {
            ""
        }
    }
}

fn nouls(transport: &dyn Transport, endpoint: &config::Endpoint, m: &text::Message) -> Nouls {
    let snippet: String = m.body.chars().take(200).collect();
    let a = pass::important_request(&m.from, &m.subject, &snippet);
    let email = m
        .from
        .rsplit_once('<')
        .map_or(m.from.as_str(), |(_, e)| e.trim_end_matches('>'));
    let b = pass::signals_request(
        m,
        crate::contacts::is_automated(email),
        &events::for_message(m),
    );
    let ask = |request| {
        jev::run(transport, endpoint, &request)
            .map(|r| r.answers)
            .unwrap_or_default()
    };
    Nouls {
        a: ask(a),
        b: ask(b),
    }
}

/// AI-7's line for these messages, or why there's none.
fn line(
    transport: &dyn Transport,
    endpoint: &config::Endpoint,
    messages: &[text::Message],
) -> Result<String, String> {
    llm::run(
        transport,
        endpoint,
        &llm::model_for(&endpoint.model, Feature::Line),
        &pass::line_request(messages),
        &AtomicBool::new(true),
        &|| false,
        &mut |_| {},
    )
    .map(|r| r.text)
    .map_err(|e| e.to_string())
}

fn fmt(yes: Option<f64>) -> String {
    yes.map_or("–".into(), |p| format!("{p:.2}"))
}

/// The columns both Jev reports share, after the first ones.
const NOUL_HEADER: &str =
    "important (A) | newsletter (A) | silent | needs_reply (B) | newsletter (B) | marker |";

fn noul_cells(n: &Nouls) -> String {
    format!(
        "{} | {} | {} | {} | {} | {} |",
        n.cell('a', "important"),
        n.cell('a', "newsletter"),
        n.silent(),
        n.cell('b', "needs_reply"),
        n.cell('b', "newsletter"),
        n.marker(),
    )
}

/// How the reports explain the marker and silent columns.
fn noul_legend() -> String {
    format!(
        "A is request A (sender, subject, snippet), B request B (body). \
         **silent**: Important Only leaves the notification out (newsletter (A) ≥ {} or important < {}; \
         a missing answer keeps it). **marker**: Needs Reply shows (needs_reply ≥ {} and newsletter (B) < {}, \
         missing newsletter counts as not one).\n\n",
        notice::NEWSLETTER,
        notice::UNIMPORTANT,
        pass::NEEDS_REPLY,
        pass::NEEDS_REPLY_NEWSLETTER
    )
}

#[test]
fn eval_set_reads() {
    let eval: Eval = serde_json::from_str(include_str!("testdata/eval.json")).unwrap();
    assert!(eval.examples.len() >= 10);
    let ids: std::collections::HashSet<&str> =
        eval.examples.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), eval.examples.len());
    for e in eval.examples.iter().filter(|e| e.feature == "label") {
        assert_eq!(e.options[0].0, "none", "{}", e.id);
    }
}

#[test]
#[ignore]
fn ai_check() {
    let config = AiConfig::load();
    let dir = Path::new(CHECK_DIR);
    std::fs::create_dir_all(dir).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let transport = UreqTransport::new();
    let eval: Eval =
        serde_json::from_str(include_str!("testdata/eval.json")).expect("eval.json reads");

    // The writing model: a plain and a streamed answer.
    let mut md = String::from("# Writing model\n\n");
    match &config.llm {
        None => md.push_str("Not set in .env.\n"),
        Some(endpoint) => {
            let _ = writeln!(
                md,
                "Host `{}`, model `{}`.\n",
                endpoint.host(),
                endpoint.model
            );
            for stream in [false, true] {
                let started = Instant::now();
                let result = llm::run(
                    &transport,
                    endpoint,
                    &llm::model_for(&endpoint.model, Feature::Test),
                    &llm::Request::new("Answer with the single word OK.", "Test", 5),
                    &AtomicBool::new(stream),
                    &|| false,
                    &mut |_| {},
                );
                let _ = writeln!(
                    md,
                    "- {}: {} in {} ms",
                    if stream { "streamed" } else { "plain" },
                    match &result {
                        Ok(r) => format!(
                            "{:?} ({} in, {} out)",
                            r.text.trim(),
                            r.tokens_in,
                            r.tokens_out
                        ),
                        Err(e) => e.to_string(),
                    },
                    started.elapsed().as_millis()
                );
            }
        }
    }
    write(dir, "writing-model", &md);
    if let Some(endpoint) = &config.llm {
        lines(dir, &transport, endpoint, &eval);
        reader(dir, &transport, endpoint, &eval);
    }

    let Some(jev_endpoint) = &config.jev else {
        write(dir, "jev", "# Jev\n\nNot set in .env.\n");
        return;
    };

    // Eval set: nouls and choices.
    let mut by_feature: BTreeMap<String, String> = BTreeMap::new();
    for e in &eval.examples {
        let row = match e.feature.as_str() {
            "important" | "needs_reply" | "newsletter" => {
                let answers = nouls(&transport, jev_endpoint, &message(e));
                let request = if e.feature == "important" { 'a' } else { 'b' };
                let got = answers.yes(request, &e.feature);
                let ok = match (e.expect.as_bool(), got) {
                    (Some(want), Some(p)) => {
                        if (p >= 0.5) == want {
                            "yes"
                        } else {
                            "**no**"
                        }
                    }
                    _ => "–",
                };
                format!(
                    "| {} | {} | {} | {} | {} |",
                    e.id,
                    e.expect,
                    got.map_or("–".into(), |p| format!("{p:.2}")),
                    ok,
                    e.adversarial.as_deref().unwrap_or("")
                )
            }
            // The writing model's; done in `lines` and `reader`.
            "line" | "clean" | "summary" | "catch_up" | "replies" => continue,
            "label" => {
                let ask = |options: Vec<(String, String)>| {
                    let request = reading::label_request(&message(e), options);
                    jev::run(&transport, jev_endpoint, &request)
                        .ok()
                        .and_then(|r| {
                            r.answers
                                .get("label")
                                .and_then(|a| a.choice().map(|(c, p)| (c.to_string(), p)))
                        })
                };
                let forward = ask(e.options.clone());
                let reversed = ask(e.options.iter().rev().cloned().collect());
                let pick = |p: &Option<(String, f64)>| {
                    p.as_ref()
                        .map_or("–".into(), |(c, p)| format!("{c} ({p:.2})"))
                };
                let agree = match (&forward, &reversed) {
                    (Some(a), Some(b)) if a.0 == b.0 => "agree",
                    (Some(_), Some(_)) => "**differ**",
                    _ => "–",
                };
                format!(
                    "| {} | {} | {} / reversed {} ({agree}) | {} | {} |",
                    e.id,
                    e.expect,
                    pick(&forward),
                    pick(&reversed),
                    if forward
                        .as_ref()
                        .map(|f| serde_json::Value::from(f.0.as_str()))
                        == Some(e.expect.clone())
                    {
                        "yes"
                    } else {
                        "**no**"
                    },
                    e.adversarial.as_deref().unwrap_or("")
                )
            }
            _ => format!(
                "| {} | {} | not checked yet | – | {} |",
                e.id,
                e.expect,
                e.adversarial.as_deref().unwrap_or("")
            ),
        };
        by_feature
            .entry(e.feature.clone())
            .or_insert_with(|| {
                format!(
                    "# Eval: {}\n\n| Example | Expected | Got | OK | Adversarial |\n| --- | --- | --- | --- | --- |\n",
                    e.feature
                )
            })
            .push_str(&(row + "\n"));
    }
    for (feature, md) in &by_feature {
        write(dir, &format!("eval-{feature}"), md);
    }

    // Demo data: each Inbox conversation's latest message.
    let store = Store::open_in_memory().unwrap();
    let account = crate::backend::demo::seed(&store).unwrap();
    let heads = store.load_inbox_heads(&account.email, &[]).unwrap();
    let ids: Vec<String> = heads.iter().map(|t| t.id.clone()).collect();
    let threads = store.load_threads(&account.email, &ids).unwrap();
    let mut md = format!(
        "# Demo data: Jev nouls on each Inbox conversation's latest message\n\n{}| Subject | From | {NOUL_HEADER}\n|{}\n",
        noul_legend(),
        " --- |".repeat(8)
    );
    for t in &threads {
        let Some(m) = text::Message::from_thread(t, t.messages.len().saturating_sub(1)) else {
            continue;
        };
        let answers = nouls(&transport, jev_endpoint, &m);
        let _ = writeln!(
            md,
            "| {} | {} | {}",
            m.subject.replace('|', "/"),
            m.from.replace('|', "/"),
            noul_cells(&answers),
        );
    }
    write(dir, "demo-jev", &md);

    if std::env::var("PIGEON_AI_CHECK_REAL").is_ok_and(|v| v == "1") {
        real_inbox(dir, &transport, jev_endpoint);
    }
}

/// AI-7: the eval set's lines, then each demo Inbox conversation's, as
/// the pass asks for them.
fn lines(dir: &Path, transport: &UreqTransport, endpoint: &config::Endpoint, eval: &Eval) {
    let mut md = String::from(
        "# Eval: line\n\n| Example | About | Line | Chars | OK |\n| --- | --- | --- | --- | --- |\n",
    );
    for e in eval.examples.iter().filter(|e| e.feature == "line") {
        let got = line(transport, endpoint, &[message(e)]);
        let (text, ok) = match &got {
            Ok(l) => (
                l.clone(),
                if l.chars().count() <= pass::LINE_MAX {
                    "yes"
                } else {
                    "**no**"
                },
            ),
            Err(why) => (why.clone(), "**no**"),
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {ok} |",
            e.id,
            e.expect["about"].as_str().unwrap_or(""),
            text.replace('|', "/"),
            text.chars().count(),
        );
    }
    write(dir, "eval-line", &md);

    let store = Store::open_in_memory().unwrap();
    let account = crate::backend::demo::seed(&store).unwrap();
    let heads = store.load_inbox_heads(&account.email, &[]).unwrap();
    let ids: Vec<String> = heads.iter().map(|t| t.id.clone()).collect();
    let threads = store.load_threads(&account.email, &ids).unwrap();
    let only_lines = pass::Allowed {
        signals: false,
        line: true,
        clean: false,
    };
    // As of the newest demo mail, however old it is.
    let now = threads
        .iter()
        .filter_map(|t| t.messages.last())
        .map(|m| m.internal_date)
        .max()
        .unwrap_or(0);
    let wants = pass::work_list(
        &threads,
        &HashMap::new(),
        &account,
        now,
        only_lines,
        &|_, _| false,
    );
    let mut md = String::from(
        "# Demo data: one-line summaries\n\n| Subject | Line | Chars |\n| --- | --- | --- |\n",
    );
    for want in &wants {
        let Some(thread) = threads.iter().find(|t| t.id == want.thread_id) else {
            continue;
        };
        let Some((
            Job {
                work: Work::Llm(request),
                ..
            },
            _,
        )) = pass::job(thread, want)
        else {
            continue;
        };
        let text = llm::run(
            transport,
            endpoint,
            &llm::model_for(&endpoint.model, Feature::Line),
            &request,
            &AtomicBool::new(true),
            &|| false,
            &mut |_| {},
        )
        .map_or_else(|e| e.to_string(), |r| r.text);
        let _ = writeln!(
            md,
            "| {} | {} | {} |",
            thread.subject.replace('|', "/"),
            text.replace('|', "/"),
            text.chars().count(),
        );
    }
    write(dir, "demo-line", &md);
}

/// One writing-model answer for the check, or why there's none.
fn ask_llm(
    transport: &dyn Transport,
    endpoint: &config::Endpoint,
    feature: Feature,
    request: &llm::Request,
) -> Result<String, String> {
    llm::run(
        transport,
        endpoint,
        &llm::model_for(&endpoint.model, feature),
        request,
        &AtomicBool::new(true),
        &|| false,
        &mut |_| {},
    )
    .map(|r| r.text)
    .map_err(|e| e.to_string())
}

/// W3: Clean View (links kept and dropped), summaries, catch-ups and
/// quick replies over the eval set.
fn reader(dir: &Path, transport: &UreqTransport, endpoint: &config::Endpoint, eval: &Eval) {
    let cell = |s: &str| s.replace('|', "/").replace('\n', " ");
    let mut md = String::from(
        "# Eval: clean\n\n| Example | Expected links | Links kept | OK | Adversarial |\n| --- | --- | --- | --- | --- |\n",
    );
    for e in eval.examples.iter().filter(|e| e.feature == "clean") {
        let mut m = message(e);
        m.body = reading::source(&e.body);
        let want: Vec<&str> = e.expect["links"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let (kept, ok) = match reading::request(&m) {
            None => ("over budget".to_string(), "–"),
            Some(request) => match ask_llm(transport, endpoint, Feature::Clean, &request) {
                Err(why) => (why, "**no**"),
                Ok(answer) => {
                    let html = reading::to_html(&reading::clean(&answer, &e.body));
                    let links: Vec<String> = crate::html::links(&crate::html::parse(&html).blocks)
                        .iter()
                        .map(|l| l.to_string())
                        .collect();
                    let ok = links.iter().all(|l| want.contains(&l.as_str()));
                    (links.join(", "), if ok { "yes" } else { "**no**" })
                }
            },
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {ok} | {} |",
            e.id,
            want.join(", "),
            cell(&kept),
            e.adversarial.as_deref().unwrap_or("")
        );
    }
    write(dir, "eval-clean", &md);

    let mut md = String::from(
        "# Eval: summary, catch_up, replies\n\n| Example | Feature | About | Answer | OK | Adversarial |\n| --- | --- | --- | --- | --- | --- |\n",
    );
    for e in &eval.examples {
        let mut t = crate::data::Thread::local(
            "t-check".into(),
            e.subject.clone(),
            crate::data::Folder::Inbox,
            Vec::new(),
            None,
        );
        let mut m = crate::data::Message::new(crate::data::Contact {
            name: String::new(),
            email: e.from.clone(),
        });
        m.id = e.id.clone();
        m.body = crate::data::Body::Loaded {
            html: crate::html::from_plain_text(&e.body),
        };
        t.messages.push(m);
        let (feature, job) = match e.feature.as_str() {
            "summary" => (Feature::Summary, reading::summary_job(&t)),
            "catch_up" => (
                Feature::CatchUp,
                reading::catch_up_job(&t, std::slice::from_ref(&e.id)),
            ),
            "replies" => (Feature::Replies, reading::replies_job(&t)),
            _ => continue,
        };
        let Some(Job {
            work: Work::Llm(request),
            ..
        }) = job
        else {
            continue;
        };
        let got = ask_llm(transport, endpoint, feature, &request);
        let ok = match (&got, feature) {
            (Err(_), _) => "**no**",
            (Ok(text), Feature::CatchUp) => {
                let asks = e.expect["asked"].as_bool().unwrap_or(false);
                if text.contains(reading::ASKED) == asks {
                    "yes"
                } else {
                    "**no**"
                }
            }
            (Ok(_), _) => "yes",
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {ok} | {} |",
            e.id,
            e.feature,
            e.expect["about"].as_str().unwrap_or(""),
            cell(&got.unwrap_or_else(|why| why)),
            e.adversarial.as_deref().unwrap_or("")
        );
    }
    write(dir, "eval-reader", &md);
}

/// The latest 200 cached Inbox messages of the real store, each
/// subject with its probabilities.
fn real_inbox(dir: &Path, transport: &UreqTransport, endpoint: &config::Endpoint) {
    let data = crate::backend::data_dir();
    let Some(record) = crate::backend::auth::load_accounts(&data)
        .into_iter()
        .next()
    else {
        println!("ai_check: no account in the data folder");
        return;
    };
    let store = Store::open(&data.join("pigeon.sqlite")).unwrap();
    let heads = store.load_inbox_heads(&record.email, &[]).unwrap();
    let ids: Vec<String> = heads.iter().map(|t| t.id.clone()).collect();
    let threads = store.load_threads(&record.email, &ids).unwrap();
    let mut messages: Vec<text::Message> = threads
        .iter()
        .flat_map(|t| (0..t.messages.len()).filter_map(|ix| text::Message::from_thread(t, ix)))
        .filter(|m| !m.from.to_lowercase().contains(&record.email.to_lowercase()))
        .collect();
    messages.sort_by_key(|m| std::cmp::Reverse(m.date));
    messages.truncate(200);
    let rows: Vec<String> = std::thread::scope(|scope| {
        let chunks: Vec<_> = messages
            .chunks(messages.len().div_ceil(6).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|m| {
                            let answers = nouls(transport, endpoint, m);
                            format!(
                                "| {} | {} | {}",
                                m.date.format("%Y-%m-%d"),
                                m.subject.replace('|', "/"),
                                noul_cells(&answers),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        chunks.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let mut md = format!(
        "# Real Inbox: Jev nouls, latest 200 cached messages\n\n{}| Date | Subject | {NOUL_HEADER}\n|{}\n",
        noul_legend(),
        " --- |".repeat(8)
    );
    for row in rows {
        md.push_str(&row);
        md.push('\n');
    }
    write(dir, "real-inbox-jev", &md);
    println!("ai_check: {} real messages", messages.len());

    // AI-10: the Clean Views reported as bad.
    let mut md = String::from(
        "# Real store: Clean Views reported as bad\n\n| Subject | Message |\n| --- | --- |\n",
    );
    for (thread_id, ai) in store.ai_results(&record.email, None).unwrap_or_default() {
        for row in ai
            .rows
            .iter()
            .filter(|r| r.feature == Feature::CleanRejected)
        {
            let subject = threads
                .iter()
                .find(|t| t.id == thread_id)
                .map_or("(not in the Inbox)", |t| t.subject.as_str());
            let _ = writeln!(md, "| {} | {} |", subject.replace('|', "/"), row.key);
        }
    }
    write(dir, "real-clean-rejected", &md);
}

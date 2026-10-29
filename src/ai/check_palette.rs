//! W4's check script: `cargo test ai_check_palette -- --ignored --nocapture`.
//! Runs ⌘K plain words, Best Matches, Do It for Me and Ask's search terms
//! over `testdata/eval_palette.json` against the real models (keys from
//! `.env`) and writes `palette.md` to `CHECK_DIR`. Prints paths only,
//! never a key.

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
    text: String,
    #[serde(default)]
    records: Vec<RecordIn>,
    #[serde(default)]
    labels: Vec<(String, String)>,
    expect: serde_json::Value,
    #[serde(default)]
    adversarial: Option<String>,
}

#[derive(Deserialize)]
struct RecordIn {
    subject: String,
    sender: String,
    date: String,
    snippet: String,
}

fn eval() -> Eval {
    serde_json::from_str(include_str!("testdata/eval_palette.json"))
        .expect("eval_palette.json reads")
}

fn commands() -> Vec<(&'static str, &'static str)> {
    crate::actions::COMMANDS
        .iter()
        .filter(|c| !c.help_only)
        .map(|c| (c.name, c.group.title()))
        .take(254)
        .collect()
}

fn ok(good: bool) -> &'static str {
    if good { "yes" } else { "**no**" }
}

#[test]
fn eval_palette_reads() {
    let eval = eval();
    assert!((10..=15).contains(&eval.examples.len()));
    let names: Vec<&str> = commands().iter().map(|(n, _)| *n).collect();
    for e in &eval.examples {
        match e.feature.as_str() {
            "palette" => {
                let want = e.expect.as_str().unwrap();
                assert!(want == "none" || names.contains(&want), "{}", e.id);
            }
            "best_matches" => assert!(!e.records.is_empty(), "{}", e.id),
            "do_it" | "ask" => {}
            other => panic!("unknown feature {other}"),
        }
    }
}

#[test]
#[ignore]
fn ai_check_palette() {
    let config = AiConfig::load();
    let dir = Path::new(CHECK_DIR);
    std::fs::create_dir_all(dir).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let transport = UreqTransport::new();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
    let mut md = String::from("# ⌘K and search\n\n");

    let commands = commands();
    md.push_str("## Plain words (Jev)\n\n| id | expect | forward | reversed | right | adversarial |\n| --- | --- | --- | --- | --- | --- |\n");
    for e in eval().examples.iter().filter(|e| e.feature == "palette") {
        let Some(jev_endpoint) = &config.jev else {
            md.push_str("Jev isn't set in .env.\n");
            break;
        };
        let ask = |list: &[(&str, &str)]| {
            let request = search::palette_request(&e.text, list);
            jev::run(&transport, jev_endpoint, &request)
                .ok()
                .and_then(|r| {
                    r.answers
                        .get("command")
                        .and_then(|a| a.choice().map(|(c, p)| (c.to_string(), p)))
                })
                .map(|(c, p)| {
                    let name = c
                        .strip_prefix('c')
                        .and_then(|n| n.parse::<usize>().ok())
                        .and_then(|n| list.get(n - 1))
                        .map_or("none", |(name, _)| name);
                    (name.to_string(), p)
                })
        };
        let forward = ask(&commands);
        let reversed: Vec<(&str, &str)> = commands.iter().rev().copied().collect();
        let backward = ask(&reversed);
        let want = e.expect.as_str().unwrap_or("none");
        let shown = |p: &Option<(String, f64)>| {
            p.as_ref()
                .map_or("–".into(), |(c, p)| format!("{c} ({p:.2})"))
        };
        // What the palette would show: a pick at 0.6 or more.
        let picked = forward
            .as_ref()
            .filter(|(_, p)| *p >= 0.6)
            .map_or("none", |(c, _)| c.as_str());
        let _ = writeln!(
            md,
            "| {} | {want} | {} | {} | {} | {} |",
            e.id,
            shown(&forward),
            shown(&backward),
            ok(picked == want),
            e.adversarial.as_deref().unwrap_or("")
        );
    }

    md.push_str("\n## Best Matches (Jev scores)\n\n| id | scores | want ≥ 3 | right | adversarial |\n| --- | --- | --- | --- | --- |\n");
    for e in eval()
        .examples
        .iter()
        .filter(|e| e.feature == "best_matches")
    {
        let Some(jev_endpoint) = &config.jev else {
            break;
        };
        let records: Vec<search::Record> = e
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| search::Record {
                id: format!("c{}", i + 1),
                subject: r.subject.clone(),
                sender: r.sender.clone(),
                date: r.date.clone(),
                snippet: r.snippet.clone(),
            })
            .collect();
        let answers = jev::run(&transport, jev_endpoint, &search::rerank(&e.text, &records))
            .map(|r| r.answers)
            .unwrap_or_default();
        let scores = search::read_scores(&records, &answers);
        let mut got: Vec<usize> = scores
            .iter()
            .filter(|(_, s)| *s >= search::MIN_SCORE)
            .map(|(i, _)| *i)
            .collect();
        got.sort_unstable();
        let want: Vec<usize> = serde_json::from_value(e.expect.clone()).unwrap_or_default();
        let _ = writeln!(
            md,
            "| {} | {} | {want:?} | {} | {} |",
            e.id,
            scores
                .iter()
                .map(|(i, s)| format!("{i}: {s:.2}"))
                .collect::<Vec<_>>()
                .join(", "),
            ok(got == want),
            e.adversarial.as_deref().unwrap_or("")
        );
    }

    let run = |feature: Feature, request: &llm::Request| -> Result<String, String> {
        let endpoint = config.llm.as_ref().ok_or("not set in .env")?;
        llm::run(
            &transport,
            endpoint,
            &llm::model_for(&endpoint.model, feature),
            request,
            &AtomicBool::new(true),
            &|| false,
            &mut |_| {},
        )
        .map(|r| r.text)
        .map_err(|e| e.to_string())
    };

    md.push_str("\n## Do It for Me (writing model)\n\n| id | expect | plan or refusal | right | adversarial |\n| --- | --- | --- | --- | --- |\n");
    for e in eval().examples.iter().filter(|e| e.feature == "do_it") {
        let labels: Vec<crate::data::Label> = e
            .labels
            .iter()
            .map(|(id, name)| crate::data::Label::new(id.as_str(), name.as_str(), gpui::black()))
            .collect();
        let got = run(Feature::DoIt, &do_it::request(&e.text, &labels, today))
            .and_then(|answer| do_it::validate(&answer, &labels));
        let want = e.expect.as_str().unwrap_or("");
        let (shown, right) = match &got {
            Ok(plan) => (
                format!("{} · {:?}", do_it::header(plan, 0, &labels), plan.action),
                want != "refused"
                    && format!("{:?}", plan.action).to_lowercase() == want.replace('_', ""),
            ),
            Err(why) => (format!("refused: {why}"), want == "refused"),
        };
        let _ = writeln!(
            md,
            "| {} | {want} | {} | {} | {} |",
            e.id,
            shown.replace('|', "/"),
            ok(right),
            e.adversarial.as_deref().unwrap_or("")
        );
    }

    md.push_str("\n## Ask Pigeon search terms (writing model)\n\n| id | terms | calendar | right | adversarial |\n| --- | --- | --- | --- | --- |\n");
    for e in eval().examples.iter().filter(|e| e.feature == "ask") {
        let got = run(Feature::AskTerms, &search::terms_request(&e.text, today));
        let terms = got.as_deref().ok().and_then(search::read_terms);
        let want_calendar = e.expect["calendar"].as_bool().unwrap_or(false);
        let want_words: Vec<String> =
            serde_json::from_value(e.expect["terms"].clone()).unwrap_or_default();
        let (shown, right) = match &terms {
            Some(t) => {
                let joined = t.terms.join(", ").to_lowercase();
                (
                    (t.terms.join(", "), t.calendar.to_string()),
                    t.calendar == want_calendar
                        && want_words.iter().any(|w| joined.contains(w.as_str())),
                )
            }
            None => (
                (
                    format!("unreadable: {}", got.unwrap_or_else(|e| e)),
                    "–".into(),
                ),
                false,
            ),
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} |",
            e.id,
            shown.0.replace(['|', '\n'], " "),
            shown.1,
            ok(right),
            e.adversarial.as_deref().unwrap_or("")
        );
    }

    let path = dir.join("palette.md");
    std::fs::write(&path, md).unwrap();
    println!("ai_check_palette: wrote {}", path.display());
}

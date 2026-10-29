//! Where the two models are, from `.env` files and the environment, read
//! once at startup. Keys never leave this module except as the bearer
//! header; `Debug` shows them as `<set>`.

use std::{collections::HashMap, path::Path};

/// The repo's `.env`, fixed at build time.
const REPO_ENV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/.env");

const NAMES: [&str; 6] = [
    "PIGEON_AI_BASE_URL",
    "PIGEON_AI_API_KEY",
    "PIGEON_AI_MODEL",
    "PIGEON_JEV_BASE_URL",
    "PIGEON_JEV_API_KEY",
    "PIGEON_JEV_MODEL",
];

/// One provider: base URL, key and model, all set.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    pub key: String,
    pub model: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("base_url", &self.base_url)
            .field("key", &"<set>")
            .field("model", &self.model)
            .finish()
    }
}

impl Endpoint {
    /// `path` under the base URL, with the base's trailing `/` trimmed.
    pub fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// The host, for Settings and the note.
    pub fn host(&self) -> String {
        let rest = self
            .base_url
            .split_once("://")
            .map_or(self.base_url.as_str(), |(_, r)| r);
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        // No user info, should a URL carry one.
        host.rsplit('@').next().unwrap_or(host).to_string()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiConfig {
    /// The writing model (OpenAI-compatible).
    pub llm: Option<Endpoint>,
    /// Jev, through OpenRouter's System One API.
    pub jev: Option<Endpoint>,
}

impl AiConfig {
    /// The repo `.env`, then `<data dir>/.env`, then real environment
    /// variables; later ones win, and an empty value unsets.
    pub fn load() -> Self {
        let read = |path: &Path| {
            std::fs::read_to_string(path)
                .map(|text| parse_env(&text))
                .unwrap_or_default()
        };
        let env: Vec<(String, String)> = NAMES
            .iter()
            .filter_map(|n| Some((n.to_string(), std::env::var(n).ok()?)))
            .collect();
        Self::from_layers(&[
            read(Path::new(REPO_ENV)),
            read(&crate::backend::data_dir().join(".env")),
            env,
        ])
    }

    pub fn from_layers(layers: &[Vec<(String, String)>]) -> Self {
        let mut vars: HashMap<&str, &str> = HashMap::new();
        for (key, value) in layers.iter().flatten() {
            vars.insert(key, value);
        }
        let get = |name: &str| {
            vars.get(name)
                .map(|v| v.to_string())
                .filter(|v| !v.is_empty())
        };
        let endpoint = |prefix: &str| {
            Some(Endpoint {
                base_url: get(&format!("PIGEON_{prefix}_BASE_URL"))?,
                key: get(&format!("PIGEON_{prefix}_API_KEY"))?,
                model: get(&format!("PIGEON_{prefix}_MODEL"))?,
            })
        };
        // The effort suffix is code's to pick (`llm::effort`).
        let llm = endpoint("AI").map(|e| Endpoint {
            model: super::llm::base_model(&e.model).to_string(),
            ..e
        });
        Self {
            llm,
            jev: endpoint("JEV"),
        }
    }
}

/// `KEY=VALUE` lines. Blank lines and `#` comment lines are skipped; an
/// optional `export ` goes; the value is everything after the first `=`,
/// trimmed, with one pair of matching quotes stripped (no escapes); an
/// unquoted value ends at ` #`. Empty values are kept: they unset.
pub fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        let quoted = ['"', '\'']
            .iter()
            .find(|q| value.len() >= 2 && value.starts_with(**q) && value.ends_with(**q));
        let value = match quoted {
            Some(_) => &value[1..value.len() - 1],
            None => value.split(" #").next().unwrap_or_default().trim_end(),
        };
        vars.push((key.to_string(), value.to_string()));
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn env_lines_follow_the_rules() {
        let text = "# comment\n\nexport A=1\nB = two words # note\nC=\"quoted # kept\"\nD='single'\nE=\nF=a=b\nG=\"unmatched\nH=x#y\n  # indented comment\nnot a pair\n";
        assert_eq!(
            parse_env(text),
            pairs(&[
                ("A", "1"),
                ("B", "two words"),
                ("C", "quoted # kept"),
                ("D", "single"),
                ("E", ""),
                ("F", "a=b"),
                ("G", "\"unmatched"),
                ("H", "x#y"),
            ])
        );
    }

    #[test]
    fn later_layers_win_and_empty_unsets() {
        let repo = pairs(&[
            ("PIGEON_AI_BASE_URL", "http://repo/v1"),
            ("PIGEON_AI_API_KEY", "fake-repo"),
            ("PIGEON_AI_MODEL", "m1"),
            ("PIGEON_JEV_BASE_URL", "https://openrouter.ai/api"),
            ("PIGEON_JEV_API_KEY", "fake-jev"),
            ("PIGEON_JEV_MODEL", "typesafe/jev-1.14"),
        ]);
        let data = pairs(&[("PIGEON_AI_MODEL", "m2"), ("PIGEON_JEV_API_KEY", "")]);
        let env = pairs(&[("PIGEON_AI_MODEL", "m3-high")]);
        let config = AiConfig::from_layers(&[repo.clone(), data.clone()]);
        assert_eq!(config.llm.as_ref().unwrap().model, "m2");
        assert!(config.jev.is_none());
        let config = AiConfig::from_layers(&[repo.clone(), data, env]);
        assert_eq!(config.llm.unwrap().model, "m3");
        let back = pairs(&[("PIGEON_JEV_API_KEY", "fake-again")]);
        assert!(AiConfig::from_layers(&[repo, back]).jev.is_some());
    }

    #[test]
    fn a_temp_file_loads() {
        let dir = std::env::temp_dir().join(format!("pigeon-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        std::fs::write(&path, "PIGEON_AI_API_KEY=fake\n").unwrap();
        let vars = parse_env(&std::fs::read_to_string(&path).unwrap());
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(vars, pairs(&[("PIGEON_AI_API_KEY", "fake")]));
    }

    #[test]
    fn urls_join_and_keys_stay_hidden() {
        let e = Endpoint {
            base_url: "https://ai.example.ts.net/v1/".into(),
            key: "fake-secret".into(),
            model: "m".into(),
        };
        assert_eq!(
            e.url("chat/completions"),
            "https://ai.example.ts.net/v1/chat/completions"
        );
        assert_eq!(e.url("/x"), "https://ai.example.ts.net/v1/x");
        let jev = Endpoint {
            base_url: "https://openrouter.ai/api".into(),
            ..e.clone()
        };
        assert_eq!(
            jev.url("v1/systemone"),
            "https://openrouter.ai/api/v1/systemone"
        );
        assert_eq!(e.host(), "ai.example.ts.net");
        let shown = format!(
            "{:?}",
            AiConfig {
                llm: Some(e),
                jev: None
            }
        );
        assert!(!shown.contains("fake-secret") && shown.contains("<set>"));
    }
}

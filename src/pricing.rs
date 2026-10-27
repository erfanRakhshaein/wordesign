//! Token counting and input-cost estimates.
//!
//! * OpenAI models are counted exactly with `tiktoken-rs` (the same BPE tables as OpenAI's cookbook).
//! * Anthropic and Google do not publish an offline tokenizer, so their counts are estimates
//!   (counted with the o200k table and flagged with "≈" in the UI).
//! * Prices come from the community-maintained LiteLLM table, refreshed in the background at every
//!   launch (the providers' own pricing pages are JavaScript pages that cannot be read reliably).
//!   A bundled snapshot and the last downloaded copy are used when offline.

use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};

pub const PRICE_TABLE_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const SNAPSHOT: &str = include_str!("../assets/prices_snapshot.json");
const CACHE_FILE: &str = ".prices.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provider {
    Anthropic,
    OpenAi,
    Google,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "OpenAI",
            Provider::Google => "Google",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::Google => "google",
        }
    }
    fn from_key(k: &str) -> Option<Self> {
        match k {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAi),
            "google" => Some(Provider::Google),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Price {
    pub model: String,
    pub provider: Provider,
    /// US dollars per input token.
    pub usd_per_token: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tokenizer {
    O200k,
    Cl100k,
    /// No offline tokenizer exists: o200k is used as a stand-in.
    Estimate,
}

pub fn tokenizer_for(p: &Price) -> Tokenizer {
    if p.provider != Provider::OpenAi {
        return Tokenizer::Estimate;
    }
    let m = p.model.as_str();
    let legacy = m.starts_with("gpt-3.5") || (m.starts_with("gpt-4") && !m.starts_with("gpt-4o") && !m.starts_with("gpt-4.1"));
    if legacy {
        Tokenizer::Cl100k
    } else {
        Tokenizer::O200k
    }
}

impl Tokenizer {
    pub fn is_exact(self) -> bool {
        self != Tokenizer::Estimate
    }
}

pub fn count_tokens(text: &str, t: Tokenizer) -> usize {
    match t {
        Tokenizer::Cl100k => tiktoken_rs::cl100k_base_singleton().encode_ordinary(text).len(),
        Tokenizer::O200k | Tokenizer::Estimate => tiktoken_rs::o200k_base_singleton().encode_ordinary(text).len(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    pub model: String,
    pub tokens: usize,
    pub usd: f64,
    pub exact: bool,
}

pub fn quote(text: &str, p: &Price) -> Quote {
    let t = tokenizer_for(p);
    let tokens = count_tokens(text, t);
    Quote { model: p.model.clone(), tokens, usd: tokens as f64 * p.usd_per_token, exact: t.is_exact() }
}

/// `$0.0241`, `$0.000042`, `$12.40` — enough digits to be meaningful for tiny amounts.
pub fn format_usd(usd: f64) -> String {
    if usd == 0.0 {
        "$0".to_owned()
    } else if usd >= 1.0 {
        format!("${usd:.2}")
    } else if usd >= 0.01 {
        format!("${usd:.3}")
    } else if usd >= 0.00001 {
        format!("${usd:.6}")
    } else {
        format!("${usd:.8}")
    }
}

// ---------------------------------------------------------------------
// the table
// ---------------------------------------------------------------------

/// Chat models of the three providers we can count for. Dated duplicates, audio/search/realtime/image
/// variants and the like are left out.
pub fn filter_table(table: &Value) -> Vec<Price> {
    let dated = Regex::new(r"-(\d{8}|\d{4}-\d{2}-\d{2}|\d{2}-\d{4}|\d{4}(-preview)?)$").expect("static regex");
    const SKIP: [&str; 19] = [
        "audio", "search", "tts", "realtime", "transcribe", "image", "live", "computer-use", "customtools",
        "cyber", "embedding", "moderation", "codex", "deep-research", "robotics", "gemma", "exp-", "daybreak", "omni",
    ];
    let Some(map) = table.as_object() else { return Vec::new() };
    let mut out: Vec<Price> = Vec::new();
    for (key, v) in map {
        if v.get("mode").and_then(Value::as_str) != Some("chat") {
            continue;
        }
        let provider = match v.get("litellm_provider").and_then(Value::as_str) {
            Some("anthropic") => Provider::Anthropic,
            Some("openai") => Provider::OpenAi,
            Some("gemini") => Provider::Google,
            _ => continue,
        };
        let Some(per_token) = v.get("input_cost_per_token").and_then(Value::as_f64).filter(|c| *c > 0.0) else {
            continue;
        };
        let name = key.strip_prefix("gemini/").unwrap_or(key);
        if name.contains('/') || name.starts_with("ft:") || dated.is_match(name) || SKIP.iter().any(|s| name.contains(s)) {
            continue;
        }
        out.push(Price { model: name.to_owned(), provider, usd_per_token: per_token });
    }
    sort(&mut out);
    out.dedup_by(|a, b| a.model == b.model);
    out
}

fn sort(v: &mut [Price]) {
    // provider, then newest-looking name first
    v.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| natural_cmp(&b.model, &a.model)));
}

/// Compare names with numbers ordered numerically (gpt-5.10 after gpt-5.9).
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let d = c.is_ascii_digit();
            match out.last_mut() {
                Some((ld, t)) if *ld == d => t.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(&cb) {
        let o = if x.0 && y.0 {
            x.1.parse::<u64>().unwrap_or(0).cmp(&y.1.parse::<u64>().unwrap_or(0))
        } else {
            x.1.cmp(&y.1)
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

fn to_json(prices: &[Price]) -> Value {
    Value::Array(
        prices
            .iter()
            .map(|p| json!({ "m": p.model, "p": p.provider.key(), "c": p.usd_per_token }))
            .collect(),
    )
}

fn from_json(v: &Value) -> Vec<Price> {
    let mut out: Vec<Price> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(Price {
                        model: e.get("m")?.as_str()?.to_owned(),
                        provider: Provider::from_key(e.get("p")?.as_str()?)?,
                        usd_per_token: e.get("c")?.as_f64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    sort(&mut out);
    out
}

pub fn snapshot() -> Vec<Price> {
    serde_json::from_str::<Value>(SNAPSHOT).map(|v| from_json(&v)).unwrap_or_default()
}

/// The last downloaded table, if any.
pub fn load_cache(dir: &Path) -> Option<Vec<Price>> {
    let text = std::fs::read_to_string(dir.join(CACHE_FILE)).ok()?;
    let v = from_json(&serde_json::from_str::<Value>(&text).ok()?);
    (!v.is_empty()).then_some(v)
}

fn save_cache(dir: &Path, prices: &[Price]) {
    let _ = std::fs::write(dir.join(CACHE_FILE), to_json(prices).to_string());
}

/// Download the price table on a background thread. The result arrives on the channel.
pub fn spawn_refresh(dir: &Path) -> Receiver<Result<Vec<Price>, String>> {
    let (tx, rx) = mpsc::channel();
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let res = (|| -> Result<Vec<Price>, String> {
            let agent = ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(25))
                .build();
            let text = agent
                .get(PRICE_TABLE_URL)
                .call()
                .map_err(|e| e.to_string())?
                .into_string()
                .map_err(|e| e.to_string())?;
            let table: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let prices = filter_table(&table);
            if prices.len() < 5 {
                return Err("the price table had an unexpected shape".to_owned());
            }
            save_cache(&dir, &prices);
            Ok(prices)
        })();
        let _ = tx.send(res);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Value {
        json!({
            "sample_spec": { "mode": "chat" },
            "gpt-5": { "mode": "chat", "litellm_provider": "openai", "input_cost_per_token": 1.25e-6 },
            "gpt-5-2025-08-07": { "mode": "chat", "litellm_provider": "openai", "input_cost_per_token": 1.25e-6 },
            "gpt-4o-audio-preview": { "mode": "chat", "litellm_provider": "openai", "input_cost_per_token": 2.5e-6 },
            "gpt-4": { "mode": "chat", "litellm_provider": "openai", "input_cost_per_token": 30e-6 },
            "text-embedding-3-small": { "mode": "embedding", "litellm_provider": "openai", "input_cost_per_token": 2e-8 },
            "claude-sonnet-5": { "mode": "chat", "litellm_provider": "anthropic", "input_cost_per_token": 2e-6 },
            "claude-sonnet-4-5-20250929": { "mode": "chat", "litellm_provider": "anthropic", "input_cost_per_token": 3e-6 },
            "gemini/gemini-2.5-pro": { "mode": "chat", "litellm_provider": "gemini", "input_cost_per_token": 1.25e-6 },
            "gemini/gemini-2.5-pro-preview-tts": { "mode": "chat", "litellm_provider": "gemini", "input_cost_per_token": 1e-6 },
            "bedrock/claude-sonnet-5": { "mode": "chat", "litellm_provider": "bedrock", "input_cost_per_token": 2e-6 },
            "free-model": { "mode": "chat", "litellm_provider": "openai", "input_cost_per_token": 0.0 }
        })
    }

    #[test]
    fn filters_to_current_chat_models_of_the_three_providers() {
        let names: Vec<(String, Provider)> = filter_table(&table()).into_iter().map(|p| (p.model, p.provider)).collect();
        assert_eq!(
            names,
            vec![
                ("claude-sonnet-5".to_owned(), Provider::Anthropic),
                ("gpt-5".to_owned(), Provider::OpenAi),
                ("gpt-4".to_owned(), Provider::OpenAi),
                ("gemini-2.5-pro".to_owned(), Provider::Google),
            ]
        );
    }

    #[test]
    fn natural_ordering_puts_newest_first() {
        let mut v = vec!["gpt-5.2", "gpt-5.10", "gpt-5.9", "gpt-5"];
        v.sort_by(|a, b| natural_cmp(b, a));
        assert_eq!(v, ["gpt-5.10", "gpt-5.9", "gpt-5.2", "gpt-5"]);
    }

    #[test]
    fn json_round_trip_and_bundled_snapshot() {
        let p = filter_table(&table());
        assert_eq!(from_json(&to_json(&p)), p);
        let snap = snapshot();
        assert!(snap.len() > 20, "snapshot has {} models", snap.len());
        for prov in [Provider::Anthropic, Provider::OpenAi, Provider::Google] {
            assert!(snap.iter().any(|m| m.provider == prov), "{prov:?} missing from the snapshot");
        }
    }

    #[test]
    fn openai_counts_are_exact_and_cost_follows() {
        // "hello world" is 2 tokens in both cl100k_base and o200k_base.
        assert_eq!(count_tokens("hello world", Tokenizer::Cl100k), 2);
        assert_eq!(count_tokens("hello world", Tokenizer::O200k), 2);
        let gpt = Price { model: "gpt-5".into(), provider: Provider::OpenAi, usd_per_token: 1.25e-6 };
        let q = quote("hello world", &gpt);
        assert_eq!((q.tokens, q.exact), (2, true));
        assert!((q.usd - 2.5e-6).abs() < 1e-12);
        let claude = Price { model: "claude-sonnet-5".into(), provider: Provider::Anthropic, usd_per_token: 2e-6 };
        assert!(!quote("hello world", &claude).exact);
        assert_eq!(tokenizer_for(&Price { model: "gpt-4".into(), ..gpt.clone() }), Tokenizer::Cl100k);
        assert_eq!(tokenizer_for(&Price { model: "o3".into(), ..gpt }), Tokenizer::O200k);
    }

    #[test]
    fn money_formatting() {
        assert_eq!(format_usd(12.4), "$12.40");
        assert_eq!(format_usd(0.0241), "$0.024");
        assert_eq!(format_usd(0.000042), "$0.000042");
        assert_eq!(format_usd(0.0), "$0");
    }
}

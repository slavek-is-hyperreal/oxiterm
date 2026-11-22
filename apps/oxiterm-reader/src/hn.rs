//! Hacker News via the official Firebase JSON API.

use crate::fetch::{Budget, FetchError, SafeFetcher};
use serde_json::Value;
use url::Url;

/// A story (or Ask/Show HN post).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story {
    /// Item id.
    pub id: u64,
    /// Title.
    pub title: String,
    /// External URL, if any.
    pub url: Option<String>,
    /// Points.
    pub score: u64,
    /// Author.
    pub by: String,
    /// Comment count.
    pub descendants: u64,
    /// Self-post body (HTML), if any.
    pub text: Option<String>,
}

const BASE: &str = "https://hacker-news.firebaseio.com/v0";

/// Top story ids.
pub fn parse_ids(json: &str, n: usize) -> Vec<u64> {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(Value::as_u64).take(n).collect()))
        .unwrap_or_default()
}

/// Parses one item; `None` for deleted/dead/non-story items.
pub fn parse_story(json: &str) -> Option<Story> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v.get("deleted").and_then(Value::as_bool) == Some(true) || v.get("dead").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    Some(Story {
        id: v.get("id")?.as_u64()?,
        title: v.get("title")?.as_str()?.to_string(),
        url: v.get("url").and_then(Value::as_str).map(str::to_string),
        score: v.get("score").and_then(Value::as_u64).unwrap_or(0),
        by: v.get("by").and_then(Value::as_str).unwrap_or("").to_string(),
        descendants: v.get("descendants").and_then(Value::as_u64).unwrap_or(0),
        text: v.get("text").and_then(Value::as_str).map(str::to_string),
    })
}

fn get(f: &SafeFetcher, url: &str, budget: &Budget) -> Result<String, FetchError> {
    let u = Url::parse(url).map_err(|_| FetchError::BadRedirect)?;
    f.fetch_text(&u, budget).map(|(_, t)| t)
}

/// Fetches one story.
pub fn fetch_story(f: &SafeFetcher, id: u64, budget: &Budget) -> Result<Story, FetchError> {
    let body = get(f, &format!("{BASE}/item/{id}.json"), budget)?;
    parse_story(&body).ok_or(FetchError::Status(404))
}

/// Fetches the top `n` stories (item requests run 6 at a time).
pub fn fetch_top(f: &SafeFetcher, n: usize, budget: &Budget) -> Result<Vec<Story>, FetchError> {
    let ids = parse_ids(&get(f, &format!("{BASE}/topstories.json"), budget)?, n);
    let mut out: Vec<Option<Story>> = vec![None; ids.len()];
    for (ci, chunk) in ids.chunks(6).enumerate() {
        let res: Vec<Option<Story>> = std::thread::scope(|s| {
            let hs: Vec<_> = chunk.iter().map(|&id| s.spawn(move || fetch_story(f, id, budget).ok())).collect();
            hs.into_iter().map(|h| h.join().ok().flatten()).collect()
        });
        for (i, r) in res.into_iter().enumerate() {
            out[ci * 6 + i] = r;
        }
    }
    Ok(out.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids_and_stories() {
        assert_eq!(parse_ids("[5,4,3,2]", 3), vec![5, 4, 3]);
        assert!(parse_ids("nope", 3).is_empty());
        let s = parse_story(r#"{"id":1,"title":"Hi <b>","url":"https://e.com/x","score":10,"by":"u","descendants":3,"type":"story"}"#).unwrap();
        assert_eq!((s.id, s.score, s.descendants), (1, 10, 3));
        assert!(parse_story(r#"{"id":1,"deleted":true}"#).is_none());
        assert!(parse_story(r#"{"id":1}"#).is_none());
        assert!(parse_story("[]").is_none());
    }
}

//! Wikipedia via the REST APIs (PL and EN only).

use serde_json::Value;

/// Supported language editions.
pub const LANGS: &[&str] = &["pl", "en"];

/// Validates a language code.
pub fn valid_lang(l: &str) -> Option<&'static str> {
    LANGS.iter().copied().find(|x| *x == l)
}

/// Percent-encodes one path/query component.
pub fn encode_component(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Title search endpoint.
pub fn search_url(lang: &str, q: &str) -> String {
    format!("https://{lang}.wikipedia.org/w/rest.php/v1/search/title?q={}&limit=12", encode_component(q))
}

/// Random page summary endpoint (redirects to a concrete summary).
pub fn random_url(lang: &str) -> String {
    format!("https://{lang}.wikipedia.org/api/rest_v1/page/random/summary")
}

/// Parsoid HTML of an article.
pub fn article_url(lang: &str, title: &str) -> String {
    format!("https://{lang}.wikipedia.org/api/rest_v1/page/html/{}", encode_component(&title.replace(' ', "_")))
}

/// (title, description) pairs from a search response.
pub fn parse_search(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    v.get("pages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let t = p.get("title")?.as_str()?.to_string();
                    let d = p.get("description").and_then(Value::as_str).unwrap_or("").to_string();
                    Some((t, d))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Title of a summary document.
pub fn parse_summary_title(json: &str) -> Option<String> {
    serde_json::from_str::<Value>(json).ok()?.get("title")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_encoded_and_langs_validated() {
        assert_eq!(article_url("pl", "Zielona Góra"), "https://pl.wikipedia.org/api/rest_v1/page/html/Zielona_G%C3%B3ra");
        assert!(article_url("en", "a/b?c#d").ends_with("a%2Fb%3Fc%23d"));
        assert_eq!(search_url("en", "rust lang&x=1"), "https://en.wikipedia.org/w/rest.php/v1/search/title?q=rust%20lang%26x%3D1&limit=12");
        assert_eq!(valid_lang("pl"), Some("pl"));
        assert_eq!(valid_lang("evil.com/#"), None);
    }

    #[test]
    fn parses_search_and_summary() {
        let r = parse_search(r#"{"pages":[{"title":"Rust","description":"language"},{"title":"Rust (film)"}]}"#);
        assert_eq!(r, vec![("Rust".into(), "language".into()), ("Rust (film)".into(), "".into())]);
        assert_eq!(parse_summary_title(r#"{"title":"X"}"#).as_deref(), Some("X"));
        assert!(parse_search("{").is_empty());
    }
}

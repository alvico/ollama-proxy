use std::collections::HashMap;

/// The value whose prefix key is the longest match for `model`, so a rule for
/// `kimi-linear` covers every tag of it without an entry per tag.
pub fn longest_prefix<'a, V>(rules: &'a HashMap<String, V>, model: &str) -> Option<&'a V> {
    rules
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, value)| value)
}

/// Maps a model name to a backend key, falling back to `default`.
pub fn resolve<'a>(routes: &'a HashMap<String, String>, default: &'a str, model: &str) -> &'a str {
    longest_prefix(routes, model).map_or(default, String::as_str)
}

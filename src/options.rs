use serde_json::{Map, Value};
use std::collections::HashSet;

pub struct Merged {
    pub body: Vec<u8>,
    pub applied: Vec<String>,
}

/// Groups of field names where the client setting any one of them means the
/// proxy must not default any of the others. `think` and `reasoning_effort`
/// are the same logical knob expressed differently: ollama's native /api/chat
/// honors `think` and ignores `reasoning_effort`, while the OpenAI-compatible
/// /v1 endpoint does the reverse. A client that pins either is stating its
/// intent, so injecting the other would contradict it.
fn suppressed_keys<'a>(
    present: &Map<String, Value>,
    exclusive: &'a [Vec<String>],
) -> HashSet<&'a str> {
    let mut suppressed = HashSet::new();
    for group in exclusive {
        if group.iter().any(|k| present.contains_key(k)) {
            suppressed.extend(group.iter().map(String::as_str));
        }
    }
    suppressed
}

/// Fills in per-model request settings the client left out, so a model that
/// needs a smaller context does not depend on every client knowing to ask for
/// it. A value the client set is never overwritten, and a field in the same
/// `exclusive` group as something the client set is not injected either.
///
/// `fields` land at the top level of the request and `options` inside its
/// `options` object; both are needed because ollama splits request settings
/// across the two, with `think` on one side and `num_ctx` on the other.
///
/// Returns `None` when nothing was added, so an untouched body can be forwarded
/// byte for byte rather than re-serialised.
pub fn merge(
    body: &[u8],
    fields: &Map<String, Value>,
    options: &Map<String, Value>,
    exclusive: &[Vec<String>],
) -> Option<Merged> {
    if fields.is_empty() && options.is_empty() {
        return None;
    }

    let mut root: Value = serde_json::from_slice(body).ok()?;
    let object = root.as_object_mut()?;

    let mut applied = Vec::new();
    let fields_suppressed = suppressed_keys(object, exclusive);
    for (key, value) in fields {
        if !object.contains_key(key) && !fields_suppressed.contains(key.as_str()) {
            object.insert(key.clone(), value.clone());
            applied.push(key.clone());
        }
    }

    // Left alone when there is nothing to add, so a request whose `options` is
    // not an object still gets its top-level fields rather than being skipped.
    if !options.is_empty() {
        let target = object
            .entry("options")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()?;

        let options_suppressed = suppressed_keys(target, exclusive);
        for (key, value) in options {
            if !target.contains_key(key) && !options_suppressed.contains(key.as_str()) {
                target.insert(key.clone(), value.clone());
                applied.push(key.clone());
            }
        }
    }

    if applied.is_empty() {
        return None;
    }

    Some(Merged {
        body: serde_json::to_vec(&root).ok()?,
        applied,
    })
}

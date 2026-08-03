use serde_json::{Map, Value};

pub struct Merged {
    pub body: Vec<u8>,
    pub applied: Vec<String>,
}

/// Fills in per-model request settings the client left out, so a model that
/// needs a smaller context does not depend on every client knowing to ask for
/// it. A value the client set is never overwritten.
///
/// `fields` land at the top level of the request and `options` inside its
/// `options` object; both are needed because ollama splits request settings
/// across the two, with `think` on one side and `num_ctx` on the other.
///
/// Returns `None` when nothing was added, so an untouched body can be forwarded
/// byte for byte rather than re-serialised.
pub fn merge(body: &[u8], fields: &Map<String, Value>, options: &Map<String, Value>) -> Option<Merged> {
    if fields.is_empty() && options.is_empty() {
        return None;
    }

    let mut root: Value = serde_json::from_slice(body).ok()?;
    let object = root.as_object_mut()?;

    let mut applied = Vec::new();
    for (key, value) in fields {
        if !object.contains_key(key) {
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

        for (key, value) in options {
            if !target.contains_key(key) {
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

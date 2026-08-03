# ollama-proxy

A reverse proxy for [Ollama](https://ollama.com) that fronts several Ollama backends on one port, routing each model to a backend tuned for it.

## Why

`OLLAMA_KV_CACHE_TYPE` is read once when the Ollama server starts, so different KV cache formats need separate servers. The KV cache is where most of a model's VRAM goes on long contexts, and `q8_0` shrinks it to a quarter of `f16` at a small quality cost. For VRAM-bound models that pays off; for models that already fit in VRAM it just loses quality. A proxy lets you keep one backend per cache type and send each model to whichever is faster.

## How it works

- Listens on one port (default `0.0.0.0:11434`), so existing clients need no reconfiguration. Backends bind to loopback only.
- Routes a model-bearing request to a backend by **longest-prefix match** on the model name, so `gemma4` covers `gemma4:12b`. Unmatched models go to `default_backend`.
- Only one backend holds a model at a time: when a request routes to a different backend, the others are drained (`keep_alive: 0`). This matters when VRAM can't hold models from two backends at once.
- Injects per-model request defaults (both inside the `options` object and top-level fields like `think`) when the client omits them; an explicit client value always wins.
- Aggregates `/api/tags` and `/api/ps` across all backends, deduplicating by model name.
- Streams responses through untouched (Ollama emits NDJSON token by token; nothing is buffered).
- Uses a generous upstream timeout (1 h) because model loads on a partially offloaded 28 GB model can take minutes.

## Build & run

```sh
cargo build --release
./target/release/ollama-proxy config.json
```

## Configuration

See `config.json` for the annotated example.

| Key               | Description |
|-------------------|-------------|
| `listen`          | Address the proxy binds. |
| `backends`        | Map of backend name → base URL. |
| `default_backend` | Backend for models with no matching route. |
| `routes`          | Model prefix → backend name. Longest prefix wins. |
| `options`         | Per-model request options injected into the `options` object when the client omits them, matched by the same longest prefix. |
| `defaults`        | Same, but for top-level request fields beside `options` (e.g. `think`). |

## Windows stack

`start-stack.ps1` launches the two tuned backends (one `q8_0`, one `f16`) plus the proxy, keeps the machine awake while it runs, and stops everything on exit. The default ports are `11434` (proxy), `11435` (`q8_0`), `11436` (`f16`). Each backend is started by `ollama-serve-awake.ps1`, which applies the tuning environment variables (`OLLAMA_CONTEXT_LENGTH`, `OLLAMA_NUM_PARALLEL`, `OLLAMA_KV_CACHE_TYPE`, ...), redirects server logs to `%LOCALAPPDATA%\Ollama\serve-logs`, and holds a keep-awake request while the server runs.

```powershell
.\start-stack.ps1
```

## Tests

```sh
cargo test
```

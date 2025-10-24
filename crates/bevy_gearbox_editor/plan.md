### Editor architecture and critiques (bin-only; pluginized UI)

Progress:
- Completed: UI pluginization, JSON-RPC client unification + basic error propagation, Bevy tasks + messages for networking, bounded per-frame event processing, and UI clone reductions.
- Next up: centralize URL config, normalize IDs, wasm networking gating, constants for type names, structured errors + logging, tests.

- **Make UI a Plugin (bin-only)**
  - Create a `plugin` module exposing `EditorPlugin` that registers resources, events, and systems (setup, networking, UI). Keep `main.rs` minimal: build `App` and `.add_plugins(EditorPlugin)`.
  - Group systems into named `SystemSet`s and define ordering/run conditions (e.g., Connecting/Connected/Errored states).

- **Deduplicate HTTP/JSON-RPC glue**
  - Consolidate `call`/`http_call` into a single client module; remove duplicate `extract_components_map`.
  - Properly parse JSON-RPC 2.0 responses: propagate `error` vs `result`; avoid treating non-result payloads as success.

- **Move concurrency into Bevy’s flow**
  - Prefer Bevy task pools (`IoTaskPool`/`AsyncComputeTaskPool`) + `Events`/resources over a manual thread.
  - If using channels, drop `Arc<Mutex<Sender<_>>>`; store `Sender` directly (it’s `Clone + Send`). Consider `crossbeam-channel`/`flume` for `Sync` receivers and better ergonomics.
  - Add lifecycle controls: bounded queues, shutdown on app exit, and basic debouncing/batching for high-frequency actions.

- **Limit per-frame work and locks**
  - Bound event draining per frame (e.g., process N messages per update) to avoid stalls.
  - In UI, iterate by reference (avoid cloning collections) and clone the sender once per frame/UI pass, not per button.

- **Centralize configuration**
  - Store URL and related config in a `Resource`; stop passing URL in every `Command`. Provide one command/system to update config when the user edits it.

- **Normalize entity IDs**
  - Pick one width across the crate (recommend `u64`). Update UI state (e.g., `graphs: HashMap<u64, String>`) and command/event types accordingly. Convert at the very edge only if necessary.

- **Improve error types and logging**
  - Replace `String` errors with a structured error type (`thiserror` or `anyhow` with context). Add targeted logging for connect, refresh, select, save, and graph fetch paths.

- **Wasm path**
  - Either implement a wasm-friendly client (fetch-based) or gate networking off on wasm so `spawn()` is not called there. Keep Egui scheduling consistent if possible.

- **Graph building performance and reliability**
  - Reduce chattiness: prefer a batch endpoint (server-side) for fetching a machine’s graph in one call, or at least batch component queries.
  - Cache names/edges; invalidate on refresh. Clear stale `graphs` if machines disappear.
  - Extract ID/label parsing into pure helpers and add unit tests.

- **Remove magic strings**
  - Centralize component type-name constants (e.g., `bevy_ecs::name::Name`, `bevy_gearbox_core::transitions::Transitions`, etc.) in one place.

- **Visibility and naming**
  - Use `pub(crate)` for internal types (e.g., commands/events) and re-export a minimal surface from the plugin module.
  - Prefer a single clear type name over aliases (e.g., `NetClient` instead of both `Connection` and `NetCtx`).

- **Camera usage**
  - Keep `Camera2d` only if rendering world content (viewport, scene). For Egui-only, omit it to reduce scene clutter; this is orthogonal to using Bevy state everywhere.

- **Testing**
  - Unit-test entity ID parsing and edge label selection. Add an integration test for graph text generation against captured fixtures.

- **Immediate steps (checklist)**
  - [x] Add `plugin` module with `EditorPlugin`; move UI systems/resources there. Keep `main.rs` minimal.
  - [x] Unify JSON-RPC client and `extract_components_map`; add JSON-RPC error handling.
  - [x] Replace manual thread with Bevy tasks + `Messages` (or upgrade channels and drop unnecessary locks).
  - [ ] Centralize URL in a config `Resource`; stop passing it in commands.
  - [ ] Normalize IDs to `u64` across UI state, commands, and RPCs.
  - [x] Bound per-frame event processing and reduce UI clones/lock scope.
  - [ ] Gate wasm networking or provide a wasm client; align Egui scheduling.
  - [ ] De-stringify component type names into shared constants.
  - [ ] Add structured errors and logging.
  - [ ] Add tests for ID/label parsing and graph output.



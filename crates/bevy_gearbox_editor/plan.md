## Bevy Gearbox Editor — Refactor Plan

- **Tighten visibility (pub(crate))**: Restrict visibility for items not used outside this crate. Default to `pub(crate)`; expose only minimal surface the app actually needs.

- **Order systems via a dedicated schedule**: Create a `SystemSet` grouping for network send/receive and UI interaction. Explicitly order: UI → command enqueue → network task spawn → task drain → state/UI updates. Add clear `.in_set(...)` and `.after(...)` constraints to remove nondeterminism.

- **Improve error handling**: Replace `Result<_, String>` with a small error enum using `thiserror`. Keep HTTP, JSON-RPC, and domain errors distinct, add context at boundaries, and map to user-facing messages only at the UI layer.

- **Task polling and async (non-blocking)**:
  - **Current pattern**: Spawning on `IoTaskPool` and draining with `poll_once` is non-blocking and acceptable, but using `block_on(poll_once(...))` is unusual in systems.
  - **Options**:
    - Keep Bevy `Task<T>` pattern; replace `block_on(poll_once(..))` with a non-blocking poll helper and drain a bounded number per frame. Maintain backpressure via a tunable limit.
    - Switch transport to an async client (`reqwest`) and keep the same task-spawn + drain model; this removes sync I/O in tasks and simplifies retries/timeouts.
    - Use a runtime bridge (e.g., a tokio tasks plugin) only if we truly need tokio features; otherwise prefer Bevy’s pools to avoid scheduler complexity.
    - For streaming updates, consider `mpsc` channels or a dedicated event stream task feeding Bevy `Events`.
  - **Recommendation**: Keep Bevy `Task<T>` + per-frame bounded drain, move HTTP to an async client, and make the drain limit a resource so we can tune it at runtime. Instrument with basic metrics/logging.

- **JSON-RPC ID**: If the server does not rely on `id`, drop it entirely to avoid implying request correlation. If correlation becomes necessary, generate unique IDs per call; do not reuse constants.

- **Transport vs domain separation**: 
  - `client.rs`: only HTTP/JSON-RPC transport (request building, error mapping, retries/timeouts). No domain assembly.
  - `graph.rs`: domain logic for building the machine graph text and related helpers. No HTTP.
  - `connection.rs`: messages (commands/events) and task orchestration that ties transport to domain.

## Clarification: App vs Plugin

- You are correct: `bevy_gearbox_core` is a plugin; `bevy_gearbox_editor` is an app. The earlier note about “app and plugin” was a forward-looking option: if you want parts of the editor UI/logic reusable by other apps, you could extract a library plugin from this crate. Not required for your current structure.



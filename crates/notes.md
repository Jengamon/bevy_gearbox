## BRP +watch quickstart: initial list + streaming updates

This is the minimal pattern to get an initial dataset, then receive live updates using BRP over HTTP with SSE (+watch).

1) Server: register a +watch method
- Add a watching handler via `with_watching_method` (or `RemoteMethodSystemId::Watching`). The handler should return `Some(Value)` only when changes occurred this tick, otherwise `None`.
- For discovery, emit events on `Added<StateMachine>`, `RemovedComponents<StateMachine>`, `Changed<Name>`, and `Changed<StateMachineId>`.

2) Client: fetch initial snapshot
- On connect, request a one-time snapshot (e.g., `editor.snapshot_discovery` or your existing `world.query` + `world.get_components` flow) to populate the UI.

3) Client: start the +watch stream
- POST a JSON-RPC request with `method` ending in `+watch`. The HTTP response will be `text/event-stream` where each pushed message is a JSON-RPC response prefixed by `data: `.

Example request (client → server):
```json
{ "jsonrpc": "2.0", "id": 1, "method": "editor.discovery+watch", "params": null }
```

Example pushed frame (server → client, repeated as changes occur):
```text
data: {"jsonrpc":"2.0","id":1,"result":{"events":[{"kind":"machine_created","machine":123,"name":"Combat"}]}}

```

Parsing client-side:
- Treat the body as SSE, split by lines, and only process lines starting with `data: `.
- Parse the JSON payload, then unwrap to your method-specific shape (e.g., `result.events: [...]`).

4) Apply updates incrementally
- For discovery: insert or update entries on `machine_created`/`machine_renamed`/`machine_id_set`; remove on `machine_removed`.
- Keep the snapshot logic intact; the +watch stream only delivers changes, not the initial list.

5) Runtime considerations
- Ensure all HTTP (reqwest) work runs inside a Tokio 1.x runtime (reactor) if you’re using reqwest (execute via a shared `tokio::Runtime`).
- Avoid dropping your own connection/session events before you trigger the snapshot/start-watch actions; set any session stamps prior to spawning network tasks.


### Design notes: reliable server→client streaming

- Event-driven vs sampled updates:
  - Prefer event-driven append points when data changes (e.g., observers like `On<Add, T>` / `On<Remove, T>` or domain events) over sampling with `Changed<T>` systems. This guarantees ordering relative to the mutation.
  - If you do sample, ensure the sampling system runs after the mutation logic; a one-frame lag may be acceptable for some UIs.

- Sequencing and buffering:
  - Use ring buffers with monotonically increasing `seq` for each stream (runtime activity, structure changes, metrics, etc.).
  - Keep per-stream watermarks (e.g., `last_seq`) in the +watch handler and emit only entries with `seq` greater than the watermark.
  - Choose bounded `capacity` appropriate for expected burstiness; evict oldest on overflow and signal clients to resnapshot if needed.

- Client loop pattern:
  - For SSE, emit one app-level event as soon as a non-empty frame arrives, then immediately re-arm the watch (don’t wait for the HTTP stream to end).
  - Apply deltas atomically per batch and request another frame; this minimizes latency and complexity.

- Identity and normalization:
  - Ensure entity/record identifiers in streamed deltas match identifiers used by the client model. Either emit canonical ids from the server, or canonicalize client-side before applying.

- Example (state machine editor):
  - Transitions: append `TransitionEdge { seq, edge }` to a ring when a transition fires; clients stream via +watch using a watermark.
  - Active states: append a snapshot of the authoritative active/leaves to a ring on `On<Add, Active>` / `On<Remove, Active>` for states; clients stream using a separate watermark. This removes frame-ordering races while keeping updates compact.

- Backpressure and consolidation:
  - Prefer one consolidated response that may include multiple streams (e.g., transitions + active snapshots) per frame to reduce client wakeups.
  - Debounce very high-frequency sources if the UI doesn’t need every intermediate step; sequences still preserve order for those delivered.
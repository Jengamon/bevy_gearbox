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


### Additional notes: reliable active state syncing

- Prefer event-driven updates for actives to avoid frame ordering races:
  - Use observers tied to component triggers: `On<Add, Active>` and `On<Remove, Active>` on state entities.
  - In those observers, resolve the root via relationship queries and append a snapshot of the authoritative active/leaves to a ring on the machine (e.g., `ActiveChangedFeed { next_seq, ring }`).
  - Keep a per-machine `last_active_seq` watermark in the +watch handler and emit only entries with `seq > last_active_seq`.

- Keep transitions and actives symmetric:
  - Both should use ring buffers with monotonically increasing `seq` and bounded `capacity`.
  - The +watch handler should emit new entries for both feeds in the same response.

- Client streaming loop best practices:
  - Emit one `MachineDeltas` batch as soon as a non-empty SSE frame arrives, then immediately re-arm the watch (don’t wait for the stream to end).
  - Canonicalize entity ids consistently. Either:
    - Have the server emit canonical ids, or
    - Canonicalize all ids client-side before applying (machine, active/leaves, edge/source/target).

- Tolerating one-frame lag:
  - If a one-frame lag is acceptable, running the active snapshot system strictly after the state machine update also works; however, event-driven observers remove the need for explicit ordering.
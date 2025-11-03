## Editor UX Storyboard

### Edge Delay
- Display: Edge pill shows a small chip on the right: "Delay: <t>" (e.g., "Delay: 1.5s"). In the Edge Inspector, a row labeled "Delay" shows a toggle (Off/On) and a numeric field (seconds, f32) with +/- stepper and a slider.
- Change: Right-click edge → "Set Delay…" opens a compact popover with a number input and unit selector (ms/s). Toggling Off removes `Delay`. Editing in the inspector live-updates the chip and persists via RPC.

### Edge Kind (Internal / External)
- Display: Internal edges use a dotted pill outline (dashed stroke) instead of a solid outline; External uses the normal solid outline. Inspector row "Kind" with radio: External (default), Internal. Tooltip explains re-entry semantics.
- Change: Right-click edge → "Mark Internal"/"Mark External". Or toggle in the inspector. Persist immediately.

### Guards (runtime/dynamic)
- Display: Pill shows a shield icon with count (e.g., "🛡 2") when any `Guards` are present. Hovering the icon shows a tooltip listing current guard keys (read-only), e.g. one per line.
- Change: None in editor. Guards are produced/managed by systems (e.g., parameters). The inspector shows a read-only list of current guards for the selected edge; there is no add/remove UI.

### Reset on Transition (ResetEdge)
- Display: Pill shows a small refresh icon with scope tooltip. Inspector row "Reset on Transition" with dropdown: Off, Source, Target, Both.
- Change: Right-click edge → "Reset Scope…" → submenu to choose scope. Or set in inspector dropdown. Off removes `ResetEdge`.

### Event Validator (per EventEdge<E>)
- Display: Pill shows a tiny filter icon when a validator is set. Inspector section "Validator" appears for Event edges:
  - Mode: "Accept all" (default) or "Custom".
  - When Custom: show a type picker (if available via protocol), or a raw JSON/reflect form editor for the validator fields.
- Change: Switch mode in inspector; when Custom, edit fields and save. Right-click edge → "Set Validator…" opens a compact editor for quick changes.

### Defer Event on State (DeferEvent<E>)
- Display: State header shows a mail/queue icon with count if any deferrals configured. Inspector section "Defers Events" lists event types with remove X and "+ Add" to include another type. Tooltip explains replay on exit.
- Change: In state inspector, click "+ Add" → choose an event type (from known variants) or type a path. Removing deletes the component instance for that `E`.

### State History (Shallow / Deep)
- Display: State header shows an "H" chip: "H: S" or "H: D" when set. Inspector row "History" with options: Off (no component), Shallow, Deep. Tooltip explains behavior.
- Change: Toggle in inspector. Off removes `History`; Shallow/Deep sets enum accordingly.

### Edge Type Editing (Always ↔ Event)
- Display: Pill label is the event name for Event edges or "Always" for Always edges. Inspector top row "Edge Type": Always or Event.
- Change: Right-click edge → "Change Edge Type…" opens picker:
  - Choose "Always" to convert (remove EventEdge component, add AlwaysEdge).
  - Choose an Event type from the searchable list (`workspace.available_event_edges`) to convert the edge (remove Always, add EventEdge<E>). Keep target/other components.

### Edge Creation Flow
- Display: When dragging from the source state’s plus handle to a target, a chooser pops near the cursor listing "Always" and the available Event types.
- Change: Selecting an item creates the edge and closes the chooser. A transient preview line is removed when created. The new edge appears with default properties and is selected to reveal the inspector.

### Edge/State Inspectors (Panel)
- Display: A right-side Inspector panel appears when an entity is selected.
  - For edges: sections for Label (Name), Edge Type, Delay, Kind, Guards (read-only list with hover tooltip), Reset on Transition, Validator.
  - For states: sections for Label (Name), History, Defers Events.
- Change: All controls auto-apply on edit; dirty controls show a subtle saving spinner until the RPC confirms.

### Watches and Live Display
- Display: Editor subscribes to edge components: Name, Target, Source (read-only in UI), Delay, EdgeKind, Guards (display-only), ResetEdge, and type-specific EventEdge presence; for states: Name, InitialState, StateMachineId, History, DeferEvent<E>. UI badges/chips update live on watch deltas.
- Change: All edits publish RPC mutations; on success, watches echo the new component values keeping the model in sync. Guards are never mutated by the editor.



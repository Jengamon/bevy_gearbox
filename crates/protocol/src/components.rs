
// Stable component keys used on the wire (Phase 1 minimal)
pub const NAME: &str = "bevy_ecs::name::Name";

// Phase 1: reflect type-path strings used by BRP world.* queries
// State machine domain types in bevy_gearbox
pub const STATE_MACHINE: &str = "bevy_gearbox_core::components::StateMachine";
pub const STATE_CHILDREN: &str = "bevy_gearbox_core::components::Substates";
pub const TRANSITIONS: &str = "bevy_gearbox_core::components::Transitions";
pub const TARGET: &str = "bevy_gearbox_core::components::Target";
pub const ALWAYS_EDGE: &str = "bevy_gearbox_core::components::AlwaysEdge";
pub const DELAY: &str = "bevy_gearbox_core::components::Delay";
pub const EDGE_KIND: &str = "bevy_gearbox_core::components::EdgeKind";
pub const INITIAL_STATE: &str = "bevy_gearbox_core::components::InitialState";
pub const STATE_MACHINE_ID: &str = "bevy_gearbox_protocol::server::StateMachineId";
// Kept for editor compatibility; no longer populated by the server (TargetedBy was removed from core)
pub const TARGETED_BY: &str = "bevy_gearbox_core::components::TargetedBy";

// Substring used to detect generic message edge component types like ...MessageEdge<...>
pub const MESSAGE_EDGE_SUBSTR: &str = "MessageEdge";
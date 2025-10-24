#![allow(dead_code)]
// Shared component type-name constants for server-side reflection queries

pub const STATE_MACHINE: &str = "bevy_gearbox_core::StateMachine";
pub const STATE_CHILDREN: &str = "bevy_gearbox_core::StateChildren";

pub const NAME: &str = "bevy_ecs::name::Name";

pub const TRANSITIONS: &str = "bevy_gearbox_core::transitions::Transitions";
pub const TARGET: &str = "bevy_gearbox_core::transitions::Target";
pub const ALWAYS_EDGE: &str = "bevy_gearbox_core::transitions::AlwaysEdge";
pub const AFTER: &str = "bevy_gearbox_core::transitions::After";

// Substring used to detect generic event edge component types like ...EventEdge<...>
pub const EVENT_EDGE_SUBSTR: &str = "EventEdge";



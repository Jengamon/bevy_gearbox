#![allow(dead_code)]
// Shared component type-name constants for server-side reflection queries

pub(crate) const STATE_MACHINE: &str = "bevy_gearbox_core::StateMachine";
pub(crate) const STATE_CHILDREN: &str = "bevy_gearbox_core::StateChildren";

pub(crate) const NAME: &str = "bevy_ecs::name::Name";

pub(crate) const TRANSITIONS: &str = "bevy_gearbox_core::transitions::Transitions";
pub(crate) const TARGET: &str = "bevy_gearbox_core::transitions::Target";
pub(crate) const ALWAYS_EDGE: &str = "bevy_gearbox_core::transitions::AlwaysEdge";
pub(crate) const AFTER: &str = "bevy_gearbox_core::transitions::After";

// Substring used to detect generic event edge component types like ...EventEdge<...>
pub(crate) const EVENT_EDGE_SUBSTR: &str = "EventEdge";



#![allow(dead_code)]
// Shared component type-name constants for server-side reflection queries

pub(crate) const STATE_MACHINE: &str = "bevy_gearbox_core::StateMachine";
pub(crate) const STATE_CHILDREN: &str = "bevy_gearbox_core::StateChildren";

pub(crate) const TRANSITIONS: &str = "bevy_gearbox_core::transitions::Transitions";
pub(crate) const TARGET: &str = "bevy_gearbox_core::transitions::Target";
pub(crate) const ALWAYS_EDGE: &str = "bevy_gearbox_core::transitions::AlwaysEdge";
pub(crate) const AFTER: &str = "bevy_gearbox_core::transitions::After";
pub(crate) const PARALLEL: &str = "bevy_gearbox_core::Parallel";
pub(crate) const INITIAL_STATE: &str = "bevy_gearbox_core::InitialState";

pub(crate) const NAME: &str = "bevy_ecs::name::Name";

// Substring used to detect generic event edge component types like ...EventEdge<...>
pub(crate) const EVENT_EDGE_SUBSTR: &str = "EventEdge";

// Server-side tracker components (exposed by bevy_gearbox_core::server)
pub(crate) const ACTIVE_TRACKER: &str = "bevy_gearbox_core::server::ActiveTracker";
pub(crate) const TRANSITION_FEED: &str = "bevy_gearbox_core::server::TransitionFeed";

pub(crate) const STATE_MACHINE_ID: &str = "bevy_gearbox_core::StateMachineId";



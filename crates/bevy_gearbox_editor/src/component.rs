#![allow(dead_code)]
// Shared component type-name constants for server-side reflection queries

pub(crate) const STATE_MACHINE: &str = "bevy_gearbox::StateMachine";
pub(crate) const STATE_CHILDREN: &str = "bevy_gearbox::Substates";

pub(crate) const TRANSITIONS: &str = "bevy_gearbox::transitions::Transitions";
pub(crate) const TARGET: &str = "bevy_gearbox::transitions::Target";
pub(crate) const ALWAYS_EDGE: &str = "bevy_gearbox::transitions::AlwaysEdge";
pub(crate) const AFTER: &str = "bevy_gearbox::transitions::After";
pub(crate) const PARALLEL: &str = "bevy_gearbox::Parallel";
pub(crate) const INITIAL_STATE: &str = "bevy_gearbox::InitialState";

pub(crate) const NAME: &str = "bevy_ecs::name::Name";

// Substring used to detect generic event edge component types like ...EventEdge<...>
pub(crate) const EVENT_EDGE_SUBSTR: &str = "EventEdge";

// Server-side tracker components (exposed by bevy_gearbox::server)
pub(crate) const ACTIVE_TRACKER: &str = "bevy_gearbox::server::ActiveTracker";
pub(crate) const TRANSITION_FEED: &str = "bevy_gearbox::server::TransitionFeed";

pub(crate) const STATE_MACHINE_ID: &str = "bevy_gearbox::StateMachineId";



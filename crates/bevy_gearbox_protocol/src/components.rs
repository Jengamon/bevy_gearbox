
// Stable component keys used on the wire (Phase 1 minimal)
pub const NAME: &str = "Name"; // maps to bevy::prelude::Name
// Reflect type-path (BRP) for Name
pub const NAME_REFLECT: &str = "bevy_ecs::name::Name";

// Phase 1: reflect type-path strings used by BRP world.* queries
// State machine domain types in bevy_gearbox
pub const STATE_MACHINE: &str = "bevy_gearbox::StateMachine";
pub const STATE_CHILDREN: &str = "bevy_gearbox::Substates";
pub const TRANSITIONS: &str = "bevy_gearbox::transitions::Transitions";
pub const TARGET: &str = "bevy_gearbox::transitions::Target";
pub const TARGETED_BY: &str = "bevy_gearbox::transitions::TargetedBy";
pub const ALWAYS_EDGE: &str = "bevy_gearbox::transitions::AlwaysEdge";
pub const AFTER: &str = "bevy_gearbox::transitions::After";
pub const PARALLEL: &str = "bevy_gearbox::Parallel";
pub const INITIAL_STATE: &str = "bevy_gearbox::InitialState";
pub const STATE_MACHINE_ID: &str = "bevy_gearbox::StateMachineId";

// Editor-exposed trackers (from bevy_gearbox::server)
pub const ACTIVE_TRACKER: &str = "bevy_gearbox::server::ActiveTracker";
pub const TRANSITION_FEED: &str = "bevy_gearbox::server::TransitionFeed";

// Substring used to detect generic event edge component types like ...EventEdge<...>
pub const EVENT_EDGE_SUBSTR: &str = "EventEdge";
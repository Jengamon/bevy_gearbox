pub use bevy_gearbox_core::*;

// Attribute macros
pub use bevy_gearbox_macros::gearbox_message;
pub use bevy_gearbox_macros::transition_message;
pub use bevy_gearbox_macros::state_component;
pub use bevy_gearbox_macros::state_bridge;


pub mod core {
    pub use bevy_gearbox_core::*;
}

#[cfg(feature = "server")]
pub mod server {
    pub use bevy_gearbox_protocol::server::*;
}
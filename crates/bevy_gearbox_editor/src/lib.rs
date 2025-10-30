// Minimal library surface: only expose the protocol server plugin
pub use bevy_gearbox_protocol::server::ServerPlugin;

pub mod prelude {
    pub use bevy_gearbox_protocol::server::ServerPlugin;
}



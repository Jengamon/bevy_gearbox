use bevy::prelude::*;
use std::net::SocketAddr;

/// Configuration plugin for enabling the Bevy Remote (BRP) server from core.
///
/// Defaults:
/// - bind_address: 127.0.0.1:15703
/// - headers: empty
#[cfg(feature = "remote_server")]
pub struct RemoteServerPlugin {
    pub headers: Vec<(String, String)>,
    pub bind_address: SocketAddr,
}

#[cfg(feature = "remote_server")]
impl Default for RemoteServerPlugin {
    fn default() -> Self {
        Self {
            headers: Vec::new(),
            bind_address: "127.0.0.1:15703".parse().expect("valid default bind address"),
        }
    }
}

#[cfg(feature = "remote_server")]
impl RemoteServerPlugin {
    pub fn new() -> Self { Self::default() }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_bind_address(mut self, addr: SocketAddr) -> Self {
        self.bind_address = addr;
        self
    }
}

#[cfg(feature = "remote_server")]
impl Plugin for RemoteServerPlugin {
    fn build(&self, app: &mut App) {
        // Register commonly-inspected types
        app.register_type::<Name>();

        // Register Bevy Gearbox types commonly interacted with by the editor
        app
            .register_type::<crate::StateChildOf>()
            .register_type::<crate::StateChildren>()
            .register_type::<crate::StateMachine>()
            .register_type::<crate::InitialState>()
            .register_type::<crate::Parallel>()
            .register_type::<crate::transitions::Source>()
            .register_type::<crate::transitions::Target>()
            .register_type::<crate::transitions::EdgeKind>()
            .register_type::<crate::transitions::AlwaysEdge>();

        // Configure HTTP transport for BRP
        let mut http = {
            let addr = self.bind_address;
            bevy::remote::http::RemoteHttpPlugin::default()
                .with_address(addr.ip())
                .with_port(addr.port())
        };

        if !self.headers.is_empty() {
            let mut headers = bevy::remote::http::Headers::new();
            for (k, v) in &self.headers {
                headers = headers.insert(k.clone(), v.clone());
            }
            http = http.with_headers(headers);
        }

        app.add_plugins(bevy::remote::RemotePlugin::default());
        app.add_plugins(http);

        // Register custom RPC endpoints if present (optional hook).
        // If you keep RPC registration in core, expose a `pub fn register_rpcs(app: &mut App)`
        // and call it here. For now, this is a no-op.
        // crate::rpcs::plugin(app);
    }
}




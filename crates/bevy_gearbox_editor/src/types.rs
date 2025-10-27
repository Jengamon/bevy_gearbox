use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub(crate) struct ServerEntity(pub u64);

#[derive(Clone, Debug)]
pub(crate) struct MachineSummary {
    pub(crate) id: ServerEntity,
    pub(crate) name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum NetError {
    Http(String),
    Parse(String),
    Server(String),
    Other(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Http(s) => write!(f, "HTTP: {}", s),
            NetError::Parse(s) => write!(f, "Parse: {}", s),
            NetError::Server(s) => write!(f, "Server: {}", s),
            NetError::Other(s) => write!(f, "{}", s),
        }
    }
}

impl From<String> for NetError {
    fn from(value: String) -> Self { NetError::Other(value) }
}

impl fmt::Display for ServerEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Bevy-style display: "<index>v<generation>" extracted from raw bits
        let raw = self.0;
        let index = (raw & 0xFFFF_FFFF) as u32;
        let generation = ((raw >> 32) & 0xFFFF_FFFF) as u32;
        write!(f, "{}v{}", index, generation)
    }
}



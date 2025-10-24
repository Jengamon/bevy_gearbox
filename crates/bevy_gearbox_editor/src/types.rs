use std::fmt;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ServerEntity(pub u64);

#[derive(Clone, Debug)]
pub(crate) struct MachineSummary {
    pub(crate) id: ServerEntity,
    pub(crate) name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct GraphText {
    pub(crate) id: ServerEntity,
    pub(crate) text: String,
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



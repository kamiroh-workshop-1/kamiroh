//! Actors and how they are addressed.

use std::fmt;

use crate::endpoint::EndpointId;

/// A name unique *within* an endpoint.
///
/// Names are addressing, not authentication: the remote runtime claims them,
/// nothing proves them. See the trust model in `ARCHITECTURE.md`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "String", into = "String"))]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorName(String);

impl ActorName {
    pub fn new(s: impl Into<String>) -> Result<Self, ActorNameError> {
        let s = s.into();
        if s.is_empty() {
            return Err(ActorNameError::Empty);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ActorName {
    type Error = ActorNameError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<ActorName> for String {
    fn from(name: ActorName) -> Self {
        name.0
    }
}

impl fmt::Display for ActorName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorNameError {
    Empty,
}

impl fmt::Display for ActorNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorNameError::Empty => f.write_str("actor name is empty"),
        }
    }
}

impl std::error::Error for ActorNameError {}

/// How one actor designates another: the pair (endpoint, name).
///
/// The endpoint half is transport-proven; the name half is claimed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    pub endpoint: EndpointId,
    pub name: ActorName,
}

impl Address {
    pub fn new(endpoint: EndpointId, name: ActorName) -> Self {
        Self { endpoint, name }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.endpoint)
    }
}

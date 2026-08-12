//! kamiroh's application layer.
//!
//! Services that sit between the ports: routing inbound deliveries to the
//! right actor, enforcing each actor's allowlist, and (to come) conversation
//! lifecycle and protocol state.

pub mod admission;
pub mod inbound;

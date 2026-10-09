//! Rusting Raft relay: frame codec, handshake vocabulary, limits, the relay server and the
//! game-side client links. std only.
#![forbid(unsafe_code)]

pub mod client;
pub mod code;
pub mod frame;
pub mod limit;
pub mod proto;
pub mod server;

pub use client::{ClientIncoming, ClientLink, HostIncoming, HostLink, RelayClient, RelayError};
pub use code::{generate_code, is_valid_code};
pub use frame::{read_frame, write_frame, MAX_DATA, MAX_FRAME};
pub use proto::{Refusal, BROADCAST, DEFAULT_PORT, MAX_CLIENT_ID};
pub use server::{Config, Server};

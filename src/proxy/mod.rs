#![allow(dead_code)]

pub mod request;
pub mod response;
pub mod body;
mod core;

pub use tokio::sync::broadcast::{Sender, Receiver};
pub use self::core::*;
use crate::Waitpoint;

use hyper::body::Bytes;
use request::RequestHead;
use response::ResponseHead;

#[derive(Debug, Clone)]
pub enum ProxyState {
    RequestHead(RequestHead, Waitpoint),
    RequestChunk{chunk: Bytes},
    RequestDone,
    ResponseHead(ResponseHead, Waitpoint),
    ResponseChunk{chunk: Bytes},
    ResponseDone,
    UpgradeOpen,
    UpgradeTx{id: u32, chunk: Bytes},
    UpgradeRx{id: u32, chunk: Bytes},
    UpgradeClose,
    Error(String), // Something has gone wrong affecting a state machine
    Msg(String),   // Non-state changing alerts
    Shutdown
}

#[derive(Debug, Clone)]
pub struct ProxyEvent {
    pub id: u32,
    pub event: ProxyState,
}

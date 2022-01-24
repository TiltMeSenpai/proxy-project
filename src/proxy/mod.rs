#![allow(dead_code)]

pub mod request;
pub mod response;
pub mod body;
mod core;

pub use tokio::sync::mpsc::{Sender, Receiver};
pub use tokio::sync::oneshot::{Sender as OneshotSender, Receiver as OneshotReciever, channel as oneshot_channel};
pub use self::core::*;

use hyper::body::Bytes;
use request::RequestHead;
use response::ResponseHead;

#[derive(Debug, Clone)]
pub enum ProxyState {
    RequestHead(RequestHead),
    RequestChunk(Bytes),
    RequestDone,
    ResponseHead(ResponseHead),
    ResponseChunk(Bytes),
    ResponseDone,
    UpgradeOpen,
    UpgradeTx{id: u32, chunk: Bytes},
    UpgradeRx{id: u32, chunk: Bytes},
    UpgradeClose,
    Error(String), // Something has gone wrong affecting a state machine
    Msg(String),   // Non-state changing alerts
}

#[derive(Debug)]
pub struct ProxyEvent {
    pub id: u32,
    pub event: ProxyState,
    pub callback: Option<OneshotSender<ProxyState>>
}

impl ProxyEvent {
    pub fn req_head(id: u32, head: &RequestHead) -> (Self, OneshotReciever<ProxyState>) {
        let (tx, rx) = oneshot_channel();
        (
            Self {
                id,
                event: ProxyState::RequestHead(head.clone()),
                callback: Some(tx)
            },
            rx
        )
    }

    pub fn req_chunk(id: u32, chunk: &Bytes) -> (Self, OneshotReciever<ProxyState>) {
        let (tx, rx) = oneshot_channel();
        (
            Self {
                id,
                event: ProxyState::RequestChunk(chunk.clone()),
                callback: Some(tx)
            },
            rx
        )
    }

    pub fn req_done(id: u32) -> Self {
        Self {
            id,
            event: ProxyState::RequestDone,
            callback: None
        }
    }

    pub fn resp_head(id: u32, head: &ResponseHead) -> (Self, OneshotReciever<ProxyState>) {
        let (tx, rx) = oneshot_channel();
        (
            Self {
                id,
                event: ProxyState::ResponseHead(head.clone()),
                callback: Some(tx)
            },
            rx
        )
    }

    pub fn resp_chunk(id: u32, chunk: &Bytes) -> (Self, OneshotReciever<ProxyState>) {
        let (tx, rx) = oneshot_channel();
        (
            Self {
                id,
                event: ProxyState::ResponseChunk(chunk.clone()),
                callback: Some(tx)
            },
            rx
        )
    }

    pub fn resp_done(id: u32) -> Self {
        Self {
            id,
            event: ProxyState::ResponseDone,
            callback: None
        }
    }

    pub fn upgrade_open(id: u32) -> Self {
        Self {
            id,
            event: ProxyState::UpgradeOpen,
            callback: None
        }
    }

    pub fn upgrade_tx(req_id: u32, id: u32, chunk: &Bytes) -> (Self, OneshotReciever<ProxyState>) {
        let (tx, rx) = oneshot_channel();
        (
            Self {
                id: req_id,
                event: ProxyState::UpgradeTx{id, chunk: chunk.clone()},
                callback: Some(tx)
            },
            rx
        )
    }

    pub fn upgrade_rx(req_id: u32, id: u32, chunk: &Bytes) -> (Self, OneshotReciever<ProxyState>) {
        let (tx, rx) = oneshot_channel();
        (
            Self {
                id: req_id,
                event: ProxyState::UpgradeRx{id, chunk: chunk.clone()},
                callback: Some(tx)
            },
            rx
        )
    }

    pub fn upgrade_close(id: u32) -> Self {
        Self {
            id,
            event: ProxyState::UpgradeClose,
            callback: None
        }
    }

    pub fn err(id: u32, msg: String) -> Self {
        Self {
            id,
            event: ProxyState::Error(msg),
            callback: None
        }
    }

    pub fn msg(msg: String) -> Self {
        Self {
            id: 0,
            event: ProxyState::Msg(msg),
            callback: None
        }
    }
}
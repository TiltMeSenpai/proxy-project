use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::proxy::request::RequestHead;
use super::proxy::response::ResponseHead;
use super::proxy::ProxyEvent;

#[derive(PartialEq)]
struct StoredRequest {
    head: RequestHead,
    body: Vec<u8>,
    last_chunk_id: u32
}

#[derive(PartialEq)]
struct StoredResponse {
    head: ResponseHead,
    body: Vec<u8>,
    last_chunk_id: u32
}

#[derive(PartialEq)]
struct StoredPair {
    request: Option<StoredRequest>,
    response: Option<StoredResponse>
}

impl Default for StoredPair {
    fn default() -> Self {
        Self {
            request: None,
            response: None
        }
    }
}

struct InnerStore {
    lag_count: AtomicU32,
    cache: RefCell<Vec<StoredPair>>
}

unsafe impl Sync for InnerStore {}

pub struct Store{
    store: Arc<InnerStore>,
    frame: Arc<Mutex<Option<eframe::epi::Frame>>>, // Store a frame so we can request a repaint with an update
    pub job: Option<JoinHandle<()>>
}

impl Store {
    pub fn new() -> Self {
        Self{
            store: Arc::new(InnerStore{
                lag_count: AtomicU32::new(0),
                cache: RefCell::new(Vec::new())
            }),
            job: None,
            frame: Arc::new(Mutex::new(None))
        }
    }

    pub fn size(&self) -> Option<usize> {
        self.store.cache.try_borrow().map(|store| store.len()).ok()
    }

    pub fn lag(&self) -> u32 {
        self.store.lag_count.load(Ordering::Relaxed)
    }

    pub fn set_frame(&self, frame: eframe::epi::Frame) {
        self.frame.lock().unwrap().replace(frame);
    }

    pub fn subscribe(&mut self, mut channel: Receiver<ProxyEvent>) {
        let store = self.store.clone();
        let frame = self.frame.clone();
        self.job = Some(tokio::spawn(
            async move {
                loop {
                    let mut repaint = false;
                    match channel.recv().await {
                        Ok(ProxyEvent{id, event}) => {
                            if let Ok(mut store_mut) = store.cache.try_borrow_mut() {
                                match event {
                                    crate::proxy::ProxyState::RequestHead(head) => {
                                        repaint = true;
                                        let len = (store_mut.len() + 1) as u32;
                                        match std::cmp::Ord::cmp(&len, &id) {
                                            std::cmp::Ordering::Equal => {
                                                    store_mut.push(StoredPair{
                                                        request: Some(StoredRequest {
                                                            head: head,
                                                            body: Vec::new(),
                                                            last_chunk_id: 0
                                                        }),
                                                        response: None
                                                    })
                                            }
                                            std::cmp::Ordering::Less => {
                                                println!("Missing requests, have {} but id is {}", len, id);
                                                for _ in len..id {
                                                    println!("Adding default req pair");
                                                    store_mut.push(Default::default());
                                                }
                                                store_mut.push(StoredPair{
                                                    request: Some(StoredRequest {
                                                        head: head,
                                                        body: Vec::new(),
                                                        last_chunk_id: 0
                                                    }),
                                                    response: None
                                                })
                                            }
                                            std::cmp::Ordering::Greater => {
                                                println!("Too many requests, have {} but id is {}", len, id);
                                                if let Some(slot) = store_mut.get_mut((id - 1) as usize) {
                                                    if None == slot.request {
                                                        println!("Slot is empty, filling");
                                                        slot.request = Some(StoredRequest {
                                                            head: head,
                                                            body: Vec::new(),
                                                            last_chunk_id: 0
                                                        })
                                                    }
                                                }
                                            },
                                        }
                                    },
                                    crate::proxy::ProxyState::RequestChunk { id, chunk } => {
                                    },
                                    crate::proxy::ProxyState::RequestDone { id } => {
                                    },
                                    crate::proxy::ProxyState::ResponseHead(_) => {

                                    },
                                    crate::proxy::ProxyState::ResponseChunk { id, chunk } => {

                                    },
                                    crate::proxy::ProxyState::ResponseDone { id } => {

                                    },
                                    crate::proxy::ProxyState::UpgradeTx { id, chunk } => {

                                    },
                                    crate::proxy::ProxyState::UpgradeRx { id, chunk } => {

                                    },
                                    crate::proxy::ProxyState::Error(_) => {

                                    },
                                    crate::proxy::ProxyState::Shutdown => {

                                    },
                                }
                                
                            }
                        },
                        Err(RecvError::Lagged(n)) => {
                            let count = store.lag_count.fetch_add(n as u32, Ordering::Relaxed);
                            println!("Lagged! {} requests missed", count as u64 + n);
                        },
                        Err(RecvError::Closed) => break
                    }
                    if repaint {
                        if let Some(frame) = frame.lock().unwrap().as_ref() {
                            frame.request_repaint()
                        }
                    }
                }
            }
        ))
    }
}
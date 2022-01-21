use std::cell::RefCell;
use std::io::Error;
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::{Ui, Label, RichText, Sense};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::proxy::request::RequestHead;
use super::proxy::response::ResponseHead;
use super::proxy::ProxyEvent;

#[derive(PartialEq, Clone)]
struct StoredRequest {
    head: RequestHead,
    body: Vec<u8>,
    last_chunk_id: u32,
    status: StoredResult
}

#[derive(PartialEq, Clone)]
struct StoredResponse {
    head: ResponseHead,
    body: Vec<u8>,
    last_chunk_id: u32,
    status: StoredResult
}

#[derive(PartialEq, Clone)]
struct StoredPair {
    request: Option<StoredRequest>,
    response: Option<StoredResponse>,
}

#[derive(PartialEq, Clone)]
enum StoredResult {
    Pending,
    Ok,
    Error(String)
}

impl Default for StoredPair {
    fn default() -> Self {
        Self {
            request: None,
            response: None,
        }
    }
}

impl StoredPair{
    fn req_mut(&mut self) -> Option<&mut StoredRequest> {
        self.request.as_mut()
    }

    fn resp_mut(&mut self) -> Option<&mut StoredResponse> {
        self.response.as_mut()
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
    active: Option<usize>,
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
            active: None,
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

    pub fn draw_active(&self, ui: &mut Ui) {
        if let Some(idx) = self.active {
            if let Some(cache) = self.store.cache.try_borrow().ok() {
                if let Some(pair) = cache.get(idx) {
                    if let Some(req) = &pair.request {
                        if let Some(resp ) = &pair.response {
                            ui.heading(format!("{}: {} {}", resp.head.status, req.head.method, req.head.uri));
                        } else {
                            ui.heading(format!("PENDING: {} {}", req.head.method, req.head.uri));
                        }
                    }
                }
            }
        }
    }

    pub fn draw_sidebar(&mut self, ui: &mut Ui, mut range: Range<usize>, line_width: usize) {
        if let Ok(cache ) =  self.store.cache.try_borrow() {
            // Clamp range to actual size of vec. Don't worry about neg idx bc it's unsigned anyways.
            if range.start > cache.len() {
                range.start = cache.len() - 1;
            }
            if range.end > cache.len() {
                range.end = cache.len();
            }
            if range.start > range.end {
                range.end = range.start
            }
            let start = range.start;
            cache[range].iter().enumerate().for_each( | (idx, pair) | {
                if let Some(req) = &pair.request {
                    let method: &str = &req.head.method.to_string();
                    let method_len = method.len();
                    let path = req.head.uri.path();
                    let path_len = path.len();
                    let total = method_len + path_len + 1;
                    let text = if total > line_width {
                        format!("{} {:.*}...", method, line_width - method_len - 6, path)
                    } else {
                        format!("{} {}", method, path)
                    };
                    let label = ui.add(Label::new(RichText::from(text).monospace()).wrap(false).sense(Sense::click()));
                    if label.clicked() {
                        self.active = Some(idx + start)
                    }
                }
            });
            ui.allocate_space(ui.available_size());
        }
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
                                if id > 0 {
                                    let id = (id - 1) as usize;
                                    let len = store_mut.len();
                                    match event {
                                        crate::proxy::ProxyState::RequestHead(head) => {
                                            repaint = true;
                                            match std::cmp::Ord::cmp(&len, &id) {
                                                std::cmp::Ordering::Equal => {
                                                        store_mut.push(StoredPair{
                                                            request: Some(StoredRequest {
                                                                head: head,
                                                                body: Vec::new(),
                                                                last_chunk_id: 0,
                                                                status: StoredResult::Pending
                                                            }),
                                                            response: None,
                                                        })
                                                }
                                                std::cmp::Ordering::Less => {
                                                    println!("Missing requests, have {} but id is {}", len, id);
                                                    for _ in len..id {
                                                        store_mut.push(Default::default());
                                                    }
                                                    store_mut.push(StoredPair{
                                                        request: Some(StoredRequest {
                                                            head: head,
                                                            body: Vec::new(),
                                                            last_chunk_id: 0,
                                                            status: StoredResult::Pending
                                                        }),
                                                        response: None,
                                                    })
                                                }
                                                std::cmp::Ordering::Greater => {
                                                    println!("Too many requests, have {} but id is {}", len, id);
                                                    if let Some(slot) = store_mut.get_mut(id) {
                                                        if None == slot.request {
                                                            println!("Slot is empty, filling");
                                                            slot.request = Some(StoredRequest {
                                                                head: head,
                                                                body: Vec::new(),
                                                                last_chunk_id: 0,
                                                                status: StoredResult::Pending
                                                            })
                                                        }
                                                    }
                                                },
                                            }
                                        },
                                        crate::proxy::ProxyState::RequestChunk { id, chunk } => {
                                            store_mut.get_mut(id as usize)
                                                .map(|pair| {
                                                    if let Some(req) = pair.req_mut() {
                                                        req.last_chunk_id = id;
                                                        req.body.extend_from_slice(&chunk);
                                                    }
                                            });
                                        },
                                        crate::proxy::ProxyState::RequestDone { id } => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                if let Some(req) = pair.req_mut() {
                                                    req.status = StoredResult::Ok;
                                                } else {
                                                    println!("Request done but nothing stored????")
                                                }
                                            }
                                        },
                                        crate::proxy::ProxyState::ResponseHead( head ) => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                if pair.response == None {
                                                    pair.response = Some(StoredResponse {
                                                        head: head,
                                                        body: Vec::new(),
                                                        last_chunk_id: 0,
                                                        status: StoredResult::Pending
                                                    })
                                                }
                                            } else {
                                                println!("Missing request {}, wtf???", id);
                                            }
                                        },
                                        crate::proxy::ProxyState::ResponseChunk { id, chunk } => {
                                            store_mut.get_mut(id as usize)
                                                .map(|pair| {
                                                    if let Some(resp) = pair.resp_mut() {
                                                        resp.body.extend_from_slice(&chunk)
                                                    }
                                            });

                                        },
                                        crate::proxy::ProxyState::ResponseDone { id } => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                if let Some(resp) = pair.resp_mut() {
                                                    resp.status = StoredResult::Ok;
                                                } else {
                                                    println!("Request done but nothing stored????")
                                                }
                                            }

                                        },
                                        crate::proxy::ProxyState::UpgradeTx { id, chunk } => {

                                        },
                                        crate::proxy::ProxyState::UpgradeRx { id, chunk } => {

                                        }
                                        crate::proxy::ProxyState::Error(e) => {
                                            if let Some(pair) = store_mut.get_mut(id) {
                                                if let Some( resp ) = pair.resp_mut() {
                                                    resp.status = StoredResult::Error(e)
                                                } else if let Some( req ) = pair.req_mut() {
                                                    req.status = StoredResult::Error(e)
                                                } else {
                                                    println!("Got error for {} but neigher req or resp", id);
                                                }
                                            }
                                        }
                                        _ => {} // None of the other enums do things with requests
                                    }
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
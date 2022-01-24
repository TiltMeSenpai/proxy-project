use std::cell::RefCell;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use eframe::egui::{Ui, Label, RichText, Sense};
use tokio::sync::mpsc::Receiver;
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

#[derive(PartialEq, Clone, Debug)]
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

    pub fn set_frame(&self, frame: eframe::epi::Frame) {
        self.frame.lock().unwrap().replace(frame);
    }

    fn get_status(&self, idx: usize) -> Option<(StoredResult, StoredResult)> {
        if let Ok(store) = self.store.cache.try_borrow() {
            store.get(idx).map(
                | pair | (
                    pair.request.as_ref().map( |request | request.status.clone() ).unwrap_or( StoredResult::Pending ),
                    pair.response.as_ref().map( |response | response.status.clone() ).unwrap_or( StoredResult::Pending )
                )
            )
        } else {
            None
        }
    }

    pub fn draw_active(&self, ui: &mut Ui) {
        if let Some(idx) = self.active {
            ui.heading(format!("{:?}", self.get_status(idx)));
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
                        format!("{} {:.*}...", method, line_width - method_len - 5, path)
                    } else {
                        format!("{} {}", method, path)
                    };
                    let label = ui.add(Label::new(RichText::from(text).monospace()).wrap(false).sense(Sense::click()));
                    if label.clicked() {
                        self.active = Some(idx + start)
                    }
                } else {
                    ui.label(format!("???"));
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
                        Some(ProxyEvent{id, event, callback}) => {
                            if let Ok(mut store_mut) = store.cache.try_borrow_mut() {
                                if id > 0 {
                                    let id = (id - 1) as usize;
                                    let len = store_mut.len();
                                    match &event {
                                        crate::proxy::ProxyState::RequestHead(head) => {
                                            repaint = true;
                                            match std::cmp::Ord::cmp(&len, &id) {
                                                std::cmp::Ordering::Equal => {
                                                        store_mut.push(StoredPair{
                                                            request: Some(StoredRequest {
                                                                head: head.clone(),
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
                                                            head: head.clone(),
                                                            body: Vec::new(),
                                                            last_chunk_id: 0,
                                                            status: StoredResult::Pending
                                                        }),
                                                        response: None,
                                                    });
                                                }
                                                std::cmp::Ordering::Greater => {
                                                    println!("Too many requests, have {} but id is {}", len, id);
                                                    if let Some(slot) = store_mut.get_mut(id) {
                                                        if None == slot.request {
                                                            println!("Slot is empty, filling");
                                                            slot.request = Some(StoredRequest {
                                                                head: head.clone(),
                                                                body: Vec::new(),
                                                                last_chunk_id: 0,
                                                                status: StoredResult::Pending
                                                            })
                                                        }
                                                    }
                                                },
                                            }
                                        },
                                        crate::proxy::ProxyState::RequestChunk ( chunk ) => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                    if let Some(req) = pair.req_mut() {
                                                        req.body.extend_from_slice(&chunk);
                                                    } else {
                                                        println!("Got chunk for {} but request empty", id)
                                                    }
                                            } else {
                                                println!("Got chunk for {} but index empty", id)
                                            }
                                        },
                                        crate::proxy::ProxyState::RequestDone => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                if let Some(req) = pair.req_mut() {
                                                    req.status = StoredResult::Ok;
                                                } else {
                                                    println!("Request {} done but nothing stored????", id)
                                                }
                                            }
                                        },
                                        crate::proxy::ProxyState::ResponseHead( head ) => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                if pair.response == None {
                                                    pair.response = Some(StoredResponse {
                                                        head: head.clone(),
                                                        body: Vec::new(),
                                                        last_chunk_id: 0,
                                                        status: StoredResult::Pending
                                                    })
                                                }
                                            } else {
                                                println!("Missing response {}, wtf???", id);
                                            }
                                        },
                                        crate::proxy::ProxyState::ResponseChunk ( chunk ) => {
                                            store_mut.get_mut(id as usize)
                                                .map(|pair| {
                                                    if let Some(resp) = pair.resp_mut() {
                                                        resp.body.extend_from_slice(&chunk)
                                                    }
                                            });

                                        },
                                        crate::proxy::ProxyState::ResponseDone => {
                                            if let Some(pair) = store_mut.get_mut(id as usize) {
                                                if let Some(resp) = pair.resp_mut() {
                                                    resp.status = StoredResult::Ok;
                                                } else {
                                                    println!("Response {} done but nothing stored????", id)
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
                                                    println!("Got error with stored rx: {}", id);
                                                    resp.status = StoredResult::Error(e.clone())
                                                } else if let Some( req ) = pair.req_mut() {
                                                    println!("Got error with stored tx: {}", id);
                                                    req.status = StoredResult::Error(e.clone())
                                                } else {
                                                    println!("Got error for {} but neither req or resp", id);
                                                }
                                            }
                                        }
                                        _ => {} // None of the other enums do things with requests
                                    }
                                }
                            }
                            // Intercept/edit logic will go here
                            if let Some(callback) = callback {
                                callback.send(event).unwrap();
                            };
                        },
                        None => break
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
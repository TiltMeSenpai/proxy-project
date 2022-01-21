use std::{task::Waker, sync::Arc};

use futures::Future;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct WaitpointInner {
    completed: bool,
    waker: Option<Waker>
}

#[derive(Clone, Debug)]
pub struct Waitpoint(Arc<Mutex<WaitpointInner>>);

impl Waitpoint {
    pub fn new() -> Self {
        Self(
            Arc::new(
                Mutex::new(
                    WaitpointInner { completed: false, waker: None }
                )
            )
        )
    }

    pub async fn complete(&mut self) {
        let mut inner = self.0.lock().await;
        inner.completed = true;
        if let Some(waker) = &inner.waker {
            waker.wake_by_ref()
        }
    }
}

impl Future for Waitpoint {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        let mut lock = Box::pin(self.0.lock());
        match Future::poll(lock.as_mut(), cx) {
            std::task::Poll::Ready(mut lock) => {
                if lock.completed {
                    std::task::Poll::Ready(())
                } else {
                    lock.waker = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            },
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

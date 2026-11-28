//! One GPU, two kinds of model: the chat model (llama-server) and the image
//! model (sd-server). Only one of them is loaded at a time.
//!
//! - Before llama-server loads, [`Gpu::claim_for_chat`] waits for any picture
//!   being made to finish, then stops sd-server.
//! - Before a picture is made, [`Gpu::claim_for_images`] waits for chats that
//!   are working to finish, then unloads the chat model.
//!
//! Switches happen one at a time (`switch`). Neither side waits for the other
//! while holding the switch lock in a way the other needs to make progress:
//! the chat side holds it while pictures that already started finish (they
//! don't need the lock), and the image side never holds it while a chat is
//! working.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Notify;

pub type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type StopFn = Arc<dyn Fn() -> BoxFuture + Send + Sync>;
type BusyFn = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    #[default]
    Free,
    Chat,
    Images,
}

#[derive(Default)]
pub struct Gpu {
    switch: tokio::sync::Mutex<()>,
    owner: Mutex<Owner>,
    /// Pictures being made now.
    image_jobs: AtomicUsize,
    jobs_done: Notify,
    /// True while llama-server is running (launched by Relay).
    chat_loaded: AtomicBool,
    /// Unloads the chat model and returns once llama-server has stopped.
    stop_chat: OnceLock<StopFn>,
    /// Whether a chat is working (and so needs the chat model).
    chat_busy: OnceLock<BusyFn>,
    /// Stops sd-server and returns once it has exited.
    stop_images: OnceLock<StopFn>,
}

/// Keeps the image model on the GPU while a picture is being made.
pub struct ImageJob(Arc<Gpu>);

impl Drop for ImageJob {
    fn drop(&mut self) {
        if self.0.image_jobs.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.jobs_done.notify_waiters();
        }
    }
}

impl Gpu {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn owner(&self) -> Owner {
        *self.owner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_owner(&self, o: Owner) {
        *self.owner.lock().unwrap_or_else(|e| e.into_inner()) = o;
    }

    pub fn set_chat_hooks(&self, stop: StopFn, busy: BusyFn) {
        let _ = self.stop_chat.set(stop);
        let _ = self.chat_busy.set(busy);
    }

    pub fn set_image_stop(&self, stop: StopFn) {
        let _ = self.stop_images.set(stop);
    }

    /// llama-server started or stopped.
    pub fn chat_running(&self, running: bool) {
        self.chat_loaded.store(running, Ordering::SeqCst);
        if !running && self.owner() == Owner::Chat {
            self.set_owner(Owner::Free);
        }
    }

    pub fn is_chat_loaded(&self) -> bool {
        self.chat_loaded.load(Ordering::SeqCst)
    }

    /// sd-server stopped (unloaded or exited).
    pub fn images_stopped(&self) {
        if self.owner() == Owner::Images {
            self.set_owner(Owner::Free);
        }
    }

    pub fn image_jobs(&self) -> usize {
        self.image_jobs.load(Ordering::SeqCst)
    }

    /// Called before llama-server loads: waits for pictures being made, then
    /// stops the image model. `waiting` is called once if it has to wait.
    pub async fn claim_for_chat(&self, waiting: impl FnOnce()) {
        let _switch = self.switch.lock().await;
        if self.owner() == Owner::Images {
            let mut told = Some(waiting);
            loop {
                let done = self.jobs_done.notified();
                if self.image_jobs() == 0 {
                    break;
                }
                if let Some(f) = told.take() {
                    f();
                }
                // The timeout guards against a missed wakeup.
                let _ = tokio::time::timeout(Duration::from_secs(2), done).await;
            }
            if let Some(stop) = self.stop_images.get() {
                tracing::info!("gpu: stopping the image model for a chat");
                stop().await;
            }
        }
        self.set_owner(Owner::Chat);
    }

    /// Called before a picture is made: waits for working chats to finish,
    /// unloads the chat model, and keeps the GPU for images until the
    /// returned guard is dropped. `waiting(true)` is called while it waits
    /// for a chat, `waiting(false)` once it stops waiting.
    pub async fn claim_for_images(self: &Arc<Self>, waiting: impl Fn(bool)) -> ImageJob {
        let mut waited = false;
        loop {
            let switch = self.switch.lock().await;
            let chat_busy = self.chat_busy.get().is_some_and(|f| f());
            if self.owner() == Owner::Images && self.image_jobs() > 0 {
                // Another picture already holds the GPU for images.
            } else if self.is_chat_loaded() || self.owner() == Owner::Chat {
                if chat_busy {
                    drop(switch);
                    if !waited {
                        waited = true;
                        waiting(true);
                    }
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    continue;
                }
                if let Some(stop) = self.stop_chat.get() {
                    tracing::info!("gpu: unloading the chat model for images");
                    stop().await;
                }
            }
            self.set_owner(Owner::Images);
            self.image_jobs.fetch_add(1, Ordering::SeqCst);
            drop(switch);
            if waited {
                waiting(false);
            }
            return ImageJob(self.clone());
        }
    }

    /// Lets the image model go when nothing uses it (idle unload). Returns
    /// false if a picture started meanwhile.
    pub async fn release_images(&self) -> bool {
        let _switch = self.switch.lock().await;
        if self.image_jobs() > 0 {
            return false;
        }
        if let Some(stop) = self.stop_images.get() {
            stop().await;
        }
        self.images_stopped();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn hooks(gpu: &Arc<Gpu>, busy: Arc<AtomicBool>, chat_stops: Arc<AtomicU32>) {
        let g = gpu.clone();
        gpu.set_chat_hooks(
            Arc::new(move || {
                let g = g.clone();
                let n = chat_stops.clone();
                Box::pin(async move {
                    n.fetch_add(1, Ordering::SeqCst);
                    g.chat_running(false);
                })
            }),
            Arc::new(move || busy.load(Ordering::SeqCst)),
        );
    }

    #[tokio::test]
    async fn images_wait_for_a_working_chat_then_unload_it() {
        let gpu = Gpu::new();
        let busy = Arc::new(AtomicBool::new(true));
        let stops = Arc::new(AtomicU32::new(0));
        hooks(&gpu, busy.clone(), stops.clone());
        gpu.claim_for_chat(|| {}).await;
        gpu.chat_running(true);

        let g = gpu.clone();
        let waited = Arc::new(AtomicBool::new(false));
        let w = waited.clone();
        let job = tokio::spawn(async move {
            let _job = g
                .claim_for_images(move |on| {
                    if on {
                        w.store(true, Ordering::SeqCst)
                    }
                })
                .await;
            g.owner()
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!job.is_finished(), "must wait while the chat works");
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        busy.store(false, Ordering::SeqCst);
        let owner = tokio::time::timeout(Duration::from_secs(5), job)
            .await
            .expect("claimed")
            .expect("join");
        assert_eq!(owner, Owner::Images);
        assert!(waited.load(Ordering::SeqCst));
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(!gpu.is_chat_loaded());
    }

    #[tokio::test]
    async fn chat_waits_for_a_picture_then_stops_images() {
        let gpu = Gpu::new();
        let stops = Arc::new(AtomicU32::new(0));
        hooks(&gpu, Arc::new(AtomicBool::new(false)), Arc::default());
        let n = stops.clone();
        let g2 = gpu.clone();
        gpu.set_image_stop(Arc::new(move || {
            let n = n.clone();
            let g = g2.clone();
            Box::pin(async move {
                n.fetch_add(1, Ordering::SeqCst);
                g.images_stopped();
            })
        }));
        let job = gpu.claim_for_images(|_| {}).await;
        let g = gpu.clone();
        let chat = tokio::spawn(async move {
            g.claim_for_chat(|| {}).await;
            g.owner()
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!chat.is_finished(), "must wait for the picture");
        drop(job);
        let owner = tokio::time::timeout(Duration::from_secs(5), chat)
            .await
            .expect("claimed")
            .expect("join");
        assert_eq!(owner, Owner::Chat);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}

//! OCR 并发预算、排队限制和按图像身份合并请求。

use super::StructuredResult;
#[cfg(test)]
use super::{OcrResult, StructuredOcr};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{oneshot, Semaphore};

const OCR_MAX_CONCURRENCY: usize = 1;
const OCR_MAX_QUEUED: usize = 2;

pub(super) struct OcrRuntime {
    pub(super) permits: Arc<Semaphore>,
    admission: Arc<Semaphore>,
    #[cfg(test)]
    pub(super) in_flight: Mutex<HashMap<String, Vec<oneshot::Sender<OcrResult>>>>,
    snapshots: Mutex<HashMap<String, Arc<SnapshotWaiters>>>,
}

struct SnapshotWaiters(Mutex<Vec<Waiter>>);

impl SnapshotWaiters {
    fn has_consumers(&self) -> bool {
        self.0.lock().unwrap().iter().any(Waiter::active)
    }
}

struct Waiter {
    result: oneshot::Sender<StructuredResult>,
    cancelled: Option<Arc<AtomicBool>>,
}

impl Waiter {
    fn cancelled(&self) -> bool {
        self.cancelled
            .as_ref()
            .is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
    }

    fn active(&self) -> bool {
        !self.result.is_closed() && !self.cancelled()
    }
}

impl OcrRuntime {
    pub(super) fn new(max_concurrency: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrency)),
            admission: Arc::new(Semaphore::new(max_concurrency + OCR_MAX_QUEUED)),
            #[cfg(test)]
            in_flight: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(super) async fn run_image<F, Fut>(&'static self, work: F) -> OcrResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = OcrResult> + Send + 'static,
    {
        let admission = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| "OCR 队列已满，请稍后重试".to_string())?;
        let (mut sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let _admission = admission;
            // 仍在排队且唯一消费者已离开，立即释放其冻结 PNG。
            let permit = tokio::select! {
                permit = self.permits.acquire() => permit,
                _ = sender.closed() => return,
            };
            let result = match permit {
                Ok(_permit) => work().await,
                Err(_) => Err("OCR 并发控制器已关闭".to_string()),
            };
            let _ = sender.send(result);
        });
        receiver
            .await
            .unwrap_or_else(|_| Err("OCR 任务意外结束".to_string()))
    }

    #[cfg(test)]
    pub(super) async fn run_clip<F, Fut>(&'static self, id: impl ToString, work: F) -> OcrResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = OcrResult> + Send + 'static,
    {
        let id = id.to_string();
        let (sender, receiver) = oneshot::channel();
        let should_start = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match in_flight.get_mut(&id) {
                Some(waiters) => {
                    if waiters.len() >= 64 {
                        return Err("OCR 等待者超过上限".into());
                    }
                    waiters.push(sender);
                    None
                }
                None => {
                    // 在接纳新任务前限制队列；同 ID 合并不重复占位。
                    let admission = self
                        .admission
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| "OCR 队列已满，请稍后重试".to_string())?;
                    in_flight.insert(id.clone(), vec![sender]);
                    Some(admission)
                }
            }
        };
        if let Some(admission) = should_start {
            tokio::spawn(async move {
                let _admission = admission;
                let permit = self.permits.acquire().await;
                let result = match &permit {
                    Err(_) => Err("OCR 并发控制器已关闭".to_string()),
                    Ok(_) => {
                        let has_waiters = {
                            let mut flights = self
                                .in_flight
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            let waiters = flights.get_mut(&id).expect("任务持有已登记的 OCR 身份");
                            waiters.retain(|waiter| !waiter.is_closed());
                            !waiters.is_empty()
                        };
                        if has_waiters {
                            work().await
                        } else {
                            Err("OCR 排队请求已取消".to_string())
                        }
                    }
                };
                let waiters = self
                    .in_flight
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&id)
                    .unwrap_or_default();
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
                // 登记清理与结果发布属于同一个并发槽，避免新任务并入已结束的旧 key。
                drop(permit);
            });
        } else {
            drop(work);
        }
        receiver
            .await
            .unwrap_or_else(|_| Err("OCR 任务意外结束".to_string()))
    }
}

pub(super) fn ocr_runtime() -> &'static OcrRuntime {
    static RUNTIME: OnceLock<OcrRuntime> = OnceLock::new();
    RUNTIME.get_or_init(|| OcrRuntime::new(OCR_MAX_CONCURRENCY))
}

impl OcrRuntime {
    pub(super) fn has_consumers(&self, key: &str) -> bool {
        self.snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .is_some_and(|waiters| waiters.has_consumers())
    }

    pub(super) fn retire_abandoned_snapshot(&self, key: &str) -> bool {
        let mut flights = self.snapshots.lock().unwrap();
        if flights
            .get(key)
            .is_some_and(|waiters| waiters.has_consumers())
        {
            return false;
        }
        // 回收可能尚未结束。此时同一张图的新请求要排进下一批，不能加入已决定取消的任务。
        flights.remove(key);
        true
    }

    #[cfg(test)]
    pub(super) fn snapshots_are_empty(&self) -> bool {
        self.snapshots.lock().unwrap().is_empty()
    }

    pub(super) async fn run_structured<F, Fut>(
        &'static self,
        key: String,
        work: F,
    ) -> StructuredResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = StructuredResult> + Send + 'static,
    {
        self.run_structured_cancellable(key, None, work).await
    }

    pub(super) async fn run_structured_cancellable<F, Fut>(
        &'static self,
        key: String,
        cancelled: Option<Arc<AtomicBool>>,
        work: F,
    ) -> StructuredResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = StructuredResult> + Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let waiter = Waiter {
            result: sender,
            cancelled,
        };
        let (admission, snapshot) = {
            let mut flights = self
                .snapshots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(snapshot) = flights.get(&key) {
                let mut waiters = snapshot.0.lock().unwrap();
                waiters.retain(|waiter| !waiter.result.is_closed());
                if waiters.len() >= 64 {
                    return Err("OCR 等待者超过上限".into());
                }
                waiters.push(waiter);
                (None, Arc::clone(snapshot))
            } else {
                let admission = self
                    .admission
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| "OCR 队列已满，请稍后重试".to_string())?;
                let snapshot = Arc::new(SnapshotWaiters(Mutex::new(vec![waiter])));
                flights.insert(key.clone(), snapshot.clone());
                (Some(admission), snapshot)
            }
        };
        if let Some(admission) = admission {
            tokio::spawn(async move {
                let _admission = admission;
                let permit = self.permits.acquire().await;
                let result = match &permit {
                    Err(_) => Err("OCR 并发控制器已关闭".to_string()),
                    Ok(_) => {
                        if snapshot.has_consumers() {
                            work().await
                        } else {
                            Err("OCR 排队请求已取消".into())
                        }
                    }
                };
                {
                    let mut flights = self.snapshots.lock().unwrap();
                    if flights
                        .get(&key)
                        .is_some_and(|active| Arc::ptr_eq(active, &snapshot))
                    {
                        flights.remove(&key);
                    }
                }
                let waiters = std::mem::take(&mut *snapshot.0.lock().unwrap());
                for waiter in waiters {
                    let result = if waiter.cancelled() {
                        Err("OCR 请求已取消".into())
                    } else {
                        result.clone()
                    };
                    let _ = waiter.result.send(result);
                }
                // 登记清理与结果发布属于同一个并发槽，避免新任务并入已结束的旧 key。
                drop(permit);
            });
        } else {
            // 合并消费者不持有它自己那一份大 PNG 到识别结束。
            drop(work);
        }
        receiver
            .await
            .unwrap_or_else(|_| Err("OCR 任务意外结束".into()))
    }
}

pub(super) fn prepare_permit() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static PREPARE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PREPARE
        .get_or_init(|| Arc::new(Semaphore::new(OCR_MAX_CONCURRENCY + OCR_MAX_QUEUED)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "OCR 队列已满，请稍后重试".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    async fn wait_for_waiters(runtime: &OcrRuntime, key: &str, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count = runtime
                    .snapshots
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .get(key)
                    .map_or(0, |waiters| waiters.0.lock().unwrap().len());
                if count == expected {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("OCR 等待者必须在时限内完成登记");
    }

    async fn wait_for_keys(runtime: &OcrRuntime, keys: &[&str]) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let all_registered = {
                    let registered = runtime
                        .snapshots
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    keys.iter().all(|key| registered.contains_key(*key))
                };
                if all_registered {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("OCR key 必须在时限内完成登记");
    }

    async fn wait_for_empty(runtime: &OcrRuntime) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if runtime
                    .snapshots
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_empty()
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("OCR 任务必须在时限内完成清理");
    }

    #[tokio::test]
    async fn cancelling_one_snapshot_consumer_keeps_the_other_alive() {
        let runtime: &'static OcrRuntime = Box::leak(Box::new(OcrRuntime::new(1)));
        let cancelled = Arc::new(AtomicBool::new(false));
        let first_cancelled = cancelled.clone();
        let (release, ready) = oneshot::channel();
        let first = tokio::spawn(async move {
            runtime
                .run_structured_cancellable("frame".into(), Some(first_cancelled), || async {
                    ready.await.unwrap();
                    Ok(StructuredOcr::tesseract(1, 1, "same frame".into(), None))
                })
                .await
        });
        wait_for_waiters(runtime, "frame", 1).await;
        let second = tokio::spawn(async move {
            runtime
                .run_structured("frame".into(), || async {
                    panic!("同一帧只能进入执行槽一次");
                })
                .await
        });
        wait_for_waiters(runtime, "frame", 2).await;
        cancelled.store(true, Ordering::Release);
        assert!(runtime.has_consumers("frame"));
        release.send(()).unwrap();
        assert_eq!(first.await.unwrap().unwrap_err(), "OCR 请求已取消");
        assert_eq!(second.await.unwrap().unwrap().text, "same frame");
    }

    #[tokio::test]
    async fn same_image_can_queue_again_while_the_cancelled_worker_is_reaping() {
        let runtime: &'static OcrRuntime = Box::leak(Box::new(OcrRuntime::new(1)));
        let cancelled = Arc::new(AtomicBool::new(false));
        let first_cancelled = cancelled.clone();
        let (entered, running) = oneshot::channel();
        let (reaped, wait_for_reap) = oneshot::channel();
        let first = tokio::spawn(async move {
            runtime
                .run_structured_cancellable("frame".into(), Some(first_cancelled), || async {
                    entered.send(()).unwrap();
                    wait_for_reap.await.unwrap();
                    Err("OCR 请求已取消".into())
                })
                .await
        });
        running.await.unwrap();
        cancelled.store(true, Ordering::Release);
        assert!(runtime.retire_abandoned_snapshot("frame"));
        let (started, mut second_entered) = oneshot::channel();
        let second = tokio::spawn(async move {
            runtime
                .run_structured("frame".into(), || async {
                    started.send(()).unwrap();
                    Ok(StructuredOcr::tesseract(1, 1, "new result".into(), None))
                })
                .await
        });
        wait_for_waiters(runtime, "frame", 1).await;
        assert!(matches!(
            second_entered.try_recv(),
            Err(oneshot::error::TryRecvError::Empty),
        ));
        reaped.send(()).unwrap();
        assert!(first.await.unwrap().is_err());
        assert_eq!(second.await.unwrap().unwrap().text, "new result");
        assert_eq!(second_entered.try_recv(), Ok(()));
    }

    #[tokio::test]
    async fn same_clip_uses_one_in_flight_task() {
        let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let first = runtime.run_clip(7, move || async move {
            first_calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok("shared".to_string())
        });
        let second_calls = Arc::clone(&calls);
        let second = runtime.run_clip(7, move || async move {
            second_calls.fetch_add(1, Ordering::SeqCst);
            Ok("duplicate".to_string())
        });
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.unwrap(), "shared");
        assert_eq!(second.unwrap(), "shared");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keyed_and_unkeyed_jobs_share_one_global_permit() {
        let runtime = Box::leak(Box::new(OcrRuntime::new(1)));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let work = |active: Arc<AtomicUsize>, peak: Arc<AtomicUsize>| async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            Ok("ok".to_string())
        };
        let keyed = runtime.run_clip(1, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move || work(active, peak)
        });
        let unkeyed = runtime.run_image({
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move || work(active, peak)
        });
        let different_key = runtime.run_clip(2, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            move || work(active, peak)
        });
        let (keyed, unkeyed, different_key) = tokio::join!(keyed, unkeyed, different_key);
        keyed.unwrap();
        unkeyed.unwrap();
        different_key.unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn structured_single_flight_keeps_arc_and_queued_loading_bounded() {
        let runtime: &'static OcrRuntime = Box::leak(Box::new(OcrRuntime::new(1)));
        let gate = Arc::new(tokio::sync::Notify::new());
        let entered = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let png = Arc::new(vec![0u8; 1024]);
        let mut handles = Vec::new();
        for _ in 0..2 {
            let gate = Arc::clone(&gate);
            let entered = Arc::clone(&entered);
            let calls = Arc::clone(&calls);
            let frozen = Arc::clone(&png);
            handles.push(tokio::spawn(runtime.run_structured(
                "same".into(),
                move || async move {
                    let _frozen = frozen;
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.notify_one();
                    gate.notified().await;
                    Ok(StructuredOcr::tesseract(1, 1, "done".into(), None))
                },
            )));
        }
        entered.notified().await;
        wait_for_waiters(runtime, "same", 2).await;
        assert_eq!(Arc::strong_count(&png), 2, "合并消费者已释放其Arc");
        let load_count = Arc::new(AtomicUsize::new(0));
        let mut queued = Vec::new();
        for key in ["second", "third"] {
            let load_count = Arc::clone(&load_count);
            queued.push(tokio::spawn(runtime.run_structured(
                key.into(),
                move || async move {
                    load_count.fetch_add(1, Ordering::SeqCst);
                    Ok(StructuredOcr::tesseract(1, 1, "queued".into(), None))
                },
            )));
        }
        wait_for_keys(runtime, &["second", "third"]).await;
        assert_eq!(load_count.load(Ordering::SeqCst), 0);
        let rejected = runtime
            .run_structured("fourth".into(), || async {
                panic!("满队列不得加载PNG")
            })
            .await;
        assert!(rejected.unwrap_err().contains("队列"));
        gate.notify_one();
        for handle in handles {
            assert_eq!(handle.await.unwrap().unwrap().text, "done");
        }
        for handle in queued {
            handle.await.unwrap().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(load_count.load(Ordering::SeqCst), 2);
        assert_eq!(Arc::strong_count(&png), 1);
    }
    #[tokio::test]
    async fn structured_waiter_cap_and_abandoned_queue_skip_loading() {
        let runtime: &'static OcrRuntime = Box::leak(Box::new(OcrRuntime::new(1)));
        let permit = runtime.permits.acquire().await.unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for _ in 0..64 {
            let loads = Arc::clone(&loads);
            jobs.push(tokio::spawn(runtime.run_structured(
                "shared".into(),
                move || async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(StructuredOcr::tesseract(1, 1, "shared".into(), None))
                },
            )));
        }
        let abandoned = tokio::spawn(runtime.run_structured("abandoned".into(), || async {
            panic!("无人等待的排队请求不得加载PNG")
        }));
        wait_for_waiters(runtime, "shared", 64).await;
        wait_for_keys(runtime, &["abandoned"]).await;
        assert!(runtime
            .run_structured("shared".into(), || async {
                panic!("不启动第65个等待者")
            })
            .await
            .unwrap_err()
            .contains("等待者"));
        abandoned.abort();
        let _ = abandoned.await;
        for job in jobs.drain(..63) {
            job.abort();
            let _ = job.await;
        }
        drop(permit);
        assert_eq!(jobs.pop().unwrap().await.unwrap().unwrap().text, "shared");
        wait_for_empty(runtime).await;
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }
}

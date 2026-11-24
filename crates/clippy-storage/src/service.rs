use crate::{StorageEngine, StorageError};
use clippy_core::models::ClipItem;
use std::fmt;
use std::path::Path;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

const REQUEST_QUEUE_CAPACITY: usize = 8;
const MAX_LIST_LIMIT: i64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipListRequest {
    pub generation: u64,
    pub query: Option<String>,
    pub favorites_only: bool,
    pub offset: i64,
    pub limit: i64,
}

impl ClipListRequest {
    pub fn new(
        generation: u64,
        query: Option<String>,
        favorites_only: bool,
        offset: i64,
        limit: i64,
    ) -> Result<Self, StorageDispatchError> {
        if offset < 0 || !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(StorageDispatchError::InvalidPage { offset, limit });
        }
        Ok(Self {
            generation,
            query,
            favorites_only,
            offset,
            limit,
        })
    }
}

type ListCallback = Box<dyn FnOnce(u64, Result<Vec<ClipItem>, StorageError>) + Send + 'static>;

enum Request {
    List {
        request: ClipListRequest,
        callback: ListCallback,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct StorageClient {
    sender: SyncSender<Request>,
}

impl StorageClient {
    pub fn list_clips(
        &self,
        request: ClipListRequest,
        callback: impl FnOnce(u64, Result<Vec<ClipItem>, StorageError>) + Send + 'static,
    ) -> Result<(), StorageDispatchError> {
        self.sender
            .try_send(Request::List {
                request,
                callback: Box::new(callback),
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => StorageDispatchError::QueueFull,
                TrySendError::Disconnected(_) => StorageDispatchError::WorkerUnavailable,
            })
    }
}

pub struct StorageRuntime {
    client: StorageClient,
    worker: Option<JoinHandle<()>>,
}

impl StorageRuntime {
    pub fn open(database_file: &Path) -> Result<Self, StorageError> {
        let database_file = database_file.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel(REQUEST_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("clippy-storage".to_string())
            .spawn(move || {
                let storage = match StorageEngine::new(&database_file) {
                    Ok(storage) => {
                        let _ = ready_sender.send(Ok(()));
                        storage
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                        return;
                    }
                };

                while let Ok(request) = receiver.recv() {
                    match request {
                        Request::List { request, callback } => {
                            let result = storage.get_clips(
                                request.query.as_deref(),
                                request.favorites_only,
                                request.offset,
                                request.limit,
                            );
                            callback(request.generation, result);
                        }
                        Request::Shutdown => break,
                    }
                }
            })?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                client: StorageClient { sender },
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(StorageError::Io(std::io::Error::other(
                    "存储线程在初始化完成前退出",
                )))
            }
        }
    }

    pub fn client(&self) -> StorageClient {
        self.client.clone()
    }
}

impl Drop for StorageRuntime {
    fn drop(&mut self) {
        let _ = self.client.sender.send(Request::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageDispatchError {
    InvalidPage { offset: i64, limit: i64 },
    QueueFull,
    WorkerUnavailable,
}

impl fmt::Display for StorageDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPage { offset, limit } => {
                write!(formatter, "无效分页参数: offset={offset}, limit={limit}")
            }
            Self::QueueFull => formatter.write_str("存储请求队列已满"),
            Self::WorkerUnavailable => formatter.write_str("存储线程不可用"),
        }
    }
}

impl std::error::Error for StorageDispatchError {}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_core::models::ContentType;

    #[test]
    fn page_bounds_are_checked_before_dispatch() {
        assert!(matches!(
            ClipListRequest::new(1, None, false, -1, 20),
            Err(StorageDispatchError::InvalidPage { .. })
        ));
        assert!(matches!(
            ClipListRequest::new(1, None, false, 0, 201),
            Err(StorageDispatchError::InvalidPage { .. })
        ));
    }

    #[test]
    fn worker_owns_the_connection_and_returns_generation_with_results() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("clips.db");
        {
            let storage = StorageEngine::new(&database).unwrap();
            storage
                .insert_clip(
                    &ContentType::Text,
                    Some("hello worker"),
                    None,
                    None,
                    "worker-hash",
                    12,
                    false,
                )
                .unwrap();
        }

        let runtime = StorageRuntime::open(&database).unwrap();
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        runtime
            .client()
            .list_clips(
                ClipListRequest::new(7, Some("hello".into()), false, 0, 20).unwrap(),
                move |generation, result| result_sender.send((generation, result)).unwrap(),
            )
            .unwrap();
        let (generation, clips) = result_receiver.recv().unwrap();
        assert_eq!(generation, 7);
        assert_eq!(
            clips.unwrap()[0].text_content.as_deref(),
            Some("hello worker")
        );
    }
}

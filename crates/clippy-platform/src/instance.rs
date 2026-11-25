use crate::private_files::ensure_private_file;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

pub enum Instance {
    Primary { _lock: File, activation: PathBuf },
    Forwarded,
}

impl Instance {
    pub fn acquire(directory: &Path, key: &str) -> io::Result<Self> {
        let path = directory.join(format!("{key}.lock"));
        let activation = directory.join("open-panel");
        ensure_private_file(&path)?;
        let lock = OpenOptions::new().read(true).write(true).open(path)?;
        match lock.try_lock() {
            Ok(()) => Ok(Self::Primary {
                _lock: lock,
                activation,
            }),
            Err(TryLockError::WouldBlock) => {
                // 这个文件只负责唤醒面板，重复启动合并成一次打开。
                File::create(activation)?;
                Ok(Self::Forwarded)
            }
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    pub fn take_activation(&self) -> io::Result<bool> {
        let Self::Primary { activation, .. } = self else {
            unreachable!("只有持锁实例消费打开请求");
        };
        match fs::remove_file(activation) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_forwards_once_and_lock_can_be_reacquired() {
        let directory = tempfile::tempdir().unwrap();
        let owner = Instance::acquire(directory.path(), "test-clippy").unwrap();
        assert!(matches!(
            Instance::acquire(directory.path(), "test-clippy").unwrap(),
            Instance::Forwarded
        ));
        assert!(owner.take_activation().unwrap());
        assert!(!owner.take_activation().unwrap());
        drop(owner);
        assert!(matches!(
            Instance::acquire(directory.path(), "test-clippy").unwrap(),
            Instance::Primary { .. }
        ));
    }
}

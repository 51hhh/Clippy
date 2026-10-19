//! 只服务合成录屏测试的目录所有权；生产构建不包含此模块。

use std::io::{self, Write};
use std::path::Path;

pub(crate) struct TestDirectory {
    directory: ::tempfile::TempDir,
}

impl From<::tempfile::TempDir> for TestDirectory {
    fn from(directory: ::tempfile::TempDir) -> Self {
        Self { directory }
    }
}

impl TestDirectory {
    pub(crate) fn path(&self) -> &Path {
        self.directory.path()
    }
}

pub(crate) fn tempdir() -> io::Result<TestDirectory> {
    ::tempfile::tempdir().map(TestDirectory::from)
}

fn write_retained_path(path: &Path, output: &mut impl Write) -> io::Result<()> {
    let path = serde_json::to_string(path).map_err(io::Error::other)?;
    writeln!(output, "CLIPPY_RECORDING_TEST_ARTIFACTS={path}")
}

fn retain(directory: &mut ::tempfile::TempDir, output: &mut impl Write) {
    // 先解除删除，再尽力输出路径；输出失败不得让原 panic 变成二次 panic。
    directory.disable_cleanup(true);
    let _ = write_retained_path(directory.path(), output);
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // 只保留自己的目录，不重写 manifest/媒体，也不改变其它 owner 的回收顺序。
            retain(&mut self.directory, &mut io::stderr().lock());
        }
    }
}

#[path = "test_artifacts/tests.rs"]
mod tests;

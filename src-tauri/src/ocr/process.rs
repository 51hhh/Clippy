//! OCR 子进程监督：管道预算、统一截止时间和 kill/wait 回收。

use super::executable::{invalidate_executable_cache, missing_tesseract_message};
use std::process::Stdio;
use std::time::Duration;

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("读取 OCR 输出失败: {error}"))?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            return Err("OCR 输出超过安全上限".to_string());
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

pub(super) async fn run_recognition_process(
    mut command: tokio::process::Command,
    png_bytes: &[u8],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncWriteExt;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                invalidate_executable_cache();
                missing_tesseract_message().to_string()
            } else {
                format!("启动 tesseract 失败: {error}")
            }
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "OCR stdin 不可用".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "OCR stdout 不可用".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "OCR stderr 不可用".to_string())?;
    // 三条管道并行：输出不能堵住输入；所有 I/O 与退出共同受同一个截止时间约束。
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            async {
                stdin
                    .write_all(png_bytes)
                    .await
                    .map_err(|error| format!("写入 OCR 图片失败: {error}"))?;
                drop(stdin);
                Ok(())
            },
            read_bounded(stdout, stdout_limit),
            read_bounded(stderr, stderr_limit),
            async {
                child
                    .wait()
                    .await
                    .map_err(|error| format!("等待 OCR 退出失败: {error}"))
            }
        )
    })
    .await;
    match result {
        Ok(Ok(((), stdout, stderr, status))) => {
            if !status.success() {
                return Err(format!(
                    "tesseract 执行失败: {}",
                    String::from_utf8_lossy(&stderr).trim()
                ));
            }
            Ok(stdout)
        }
        failure => {
            // 超时/输出超限/管道失败都先终止并回收，再把错误交给 runtime 释放许可。
            let _ = child.kill().await;
            let _ = child.wait().await;
            match failure {
                Err(_) => Err("OCR 识别超时，请重试或缩小识别区域".to_string()),
                Ok(Err(error)) => Err(error),
                _ => unreachable!(),
            }
        }
    }
}

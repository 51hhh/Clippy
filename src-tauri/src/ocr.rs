//! ocr.rs — Tesseract OCR 封装
//! 通过命令行调用系统 tesseract，避免编译时动态链接依赖。
//! tesseract 缺失时返回友好错误，不影响应用启动。

use std::io::Write;
use std::process::{Command, Stdio};

/// 检查系统是否安装了 tesseract
pub fn is_available() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// 对 PNG 图片字节进行 OCR 识别，返回文字内容。
/// 通过 stdin 管道传入图片数据，stdout 获取识别结果。
pub fn recognize(png_bytes: &[u8]) -> Result<String, String> {
    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", "eng+chi_sim"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "OCR 不可用：未安装 tesseract。请运行 sudo apt install tesseract-ocr tesseract-ocr-chi-sim".to_string()
            } else {
                format!("启动 tesseract 失败: {}", e)
            }
        })?;

    // 通过 stdin 传入图片数据
    if let Some(ref mut stdin) = child.stdin {
        stdin
            .write_all(png_bytes)
            .map_err(|e| format!("写入图片数据失败: {}", e))?;
    }
    // 关闭 stdin 触发 tesseract 处理
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 tesseract 结束失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tesseract 执行失败: {}", stderr.trim()));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(text)
}

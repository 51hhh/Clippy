//! ocr.rs — Tesseract OCR 封装
//! 通过命令行调用系统 tesseract，避免编译时动态链接依赖。
//! tesseract 缺失时返回友好错误，不影响应用启动。

use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const TESSERACT_PATH_ENV: &str = "CLIPPY_TESSERACT_PATH";

fn push_unique(candidates: &mut Vec<PathBuf>, candidate: impl Into<PathBuf>) {
    let candidate = candidate.into();
    if !candidate.as_os_str().is_empty() && !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

fn bundled_candidate() -> Option<PathBuf> {
    let executable_dir = env::current_exe().ok()?.parent()?.to_path_buf();

    #[cfg(target_os = "windows")]
    return Some(executable_dir.join("tesseract.exe"));

    #[cfg(target_os = "macos")]
    return executable_dir
        .parent()
        .map(|contents| contents.join("Resources").join("tesseract"));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Some(executable_dir.join("tesseract"))
}

fn platform_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = env::var_os(variable) {
                push_unique(
                    &mut candidates,
                    PathBuf::from(root)
                        .join("Tesseract-OCR")
                        .join("tesseract.exe"),
                );
            }
        }
        if let Some(root) = env::var_os("LOCALAPPDATA") {
            push_unique(
                &mut candidates,
                PathBuf::from(root)
                    .join("Programs")
                    .join("Tesseract-OCR")
                    .join("tesseract.exe"),
            );
        }
        candidates
    }

    #[cfg(target_os = "macos")]
    {
        [
            "/opt/homebrew/bin/tesseract",
            "/usr/local/bin/tesseract",
            "/opt/local/bin/tesseract",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(target_os = "linux")]
    {
        [
            "/usr/bin/tesseract",
            "/usr/local/bin/tesseract",
            "/snap/bin/tesseract",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Vec::new()
}

fn tesseract_candidates(override_path: Option<OsString>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = override_path {
        push_unique(&mut candidates, path);
    }
    if let Some(path) = bundled_candidate() {
        push_unique(&mut candidates, path);
    }
    // 保留 PATH 语义，终端启动和自定义包管理器路径仍可工作。
    push_unique(&mut candidates, "tesseract");
    for path in platform_candidates() {
        push_unique(&mut candidates, path);
    }
    candidates
}

fn first_available<F>(
    candidates: impl IntoIterator<Item = PathBuf>,
    mut probe: F,
) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    candidates.into_iter().find(|path| probe(path))
}

fn probe_tesseract(path: &Path) -> bool {
    Command::new(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn tesseract_executable() -> Option<PathBuf> {
    first_available(
        tesseract_candidates(env::var_os(TESSERACT_PATH_ENV)),
        probe_tesseract,
    )
}

/// 检查系统是否安装了可实际执行的 Tesseract。
pub fn is_available() -> bool {
    tesseract_executable().is_some()
}

#[cfg(target_os = "linux")]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未安装 tesseract。请运行 sudo apt install tesseract-ocr tesseract-ocr-chi-sim"
}

#[cfg(target_os = "windows")]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract.exe。请安装 Tesseract 后重启 Clippy，或设置 CLIPPY_TESSERACT_PATH"
}

#[cfg(target_os = "macos")]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract。请通过 Homebrew/MacPorts 安装，或设置 CLIPPY_TESSERACT_PATH"
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn missing_tesseract_message() -> &'static str {
    "OCR 不可用：未找到 tesseract"
}

/// 对 PNG 图片字节进行 OCR 识别，返回文字内容。
/// 通过 stdin 管道传入图片数据，stdout 获取识别结果。
pub fn recognize(png_bytes: &[u8]) -> Result<String, String> {
    let executable =
        tesseract_executable().ok_or_else(|| missing_tesseract_message().to_string())?;
    let mut child = Command::new(executable)
        .args(["stdin", "stdout", "-l", "eng+chi_sim"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                missing_tesseract_message().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_skips_broken_candidates_and_returns_the_executable_it_probed() {
        let candidates = ["missing", "broken", "working"]
            .into_iter()
            .map(PathBuf::from);
        let mut probed = Vec::new();

        let resolved = first_available(candidates, |path| {
            probed.push(path.to_path_buf());
            path == Path::new("working")
        });

        assert_eq!(resolved.as_deref(), Some(Path::new("working")));
        assert_eq!(
            probed,
            ["missing", "broken", "working"]
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn explicit_override_is_first_and_duplicate_candidates_are_removed() {
        let override_path = PathBuf::from("tesseract");
        let candidates = tesseract_candidates(Some(override_path.clone().into_os_string()));

        assert_eq!(candidates.first(), Some(&override_path));
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| *candidate == &override_path)
                .count(),
            1
        );
    }

    #[test]
    fn resolver_returns_none_when_every_probe_fails() {
        let resolved = first_available([PathBuf::from("missing")], |_| false);
        assert_eq!(resolved, None);
    }
}

//! 明确配置的 Python OCR 子进程；复用父模块的并发许可与进程回收。
use super::{
    process::run_recognition_process,
    protocol::StructuredOcr,
    tesseract::{self, OCR_STDERR_LIMIT, OCR_STDOUT_LIMIT, RECOGNITION_TIMEOUT},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

pub(super) const MANIFEST_ENV: &str = "CLIPPY_OCR_MANIFEST";
const MAX_INPUT: usize = 64 * 1024 * 1024;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MODULES: [&str; 4] = [
    "pipeline.py",
    "edge_features.py",
    "layout_groups.py",
    "visual_paragraphs.py",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Asset {
    path: PathBuf,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Assets {
    det: Asset,
    rec: Asset,
    dictionary: Asset,
    edge: Asset,
    #[serde(default)]
    english_rec: Option<Asset>,
    #[serde(default)]
    english_dictionary: Option<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    #[serde(default = "bitmap_default")]
    bitmap_threshold: f64,
    #[serde(default = "box_default")]
    box_threshold: f64,
    #[serde(default = "unclip_default")]
    unclip_ratio: f64,
    #[serde(default = "line_default")]
    line_threshold: f64,
    #[serde(default = "layout_default")]
    layout_threshold: f64,
}

fn bitmap_default() -> f64 {
    0.3
}
fn box_default() -> f64 {
    0.5
}
fn unclip_default() -> f64 {
    1.2
}
fn line_default() -> f64 {
    0.6
}
fn layout_default() -> f64 {
    0.52
}
impl Default for Options {
    fn default() -> Self {
        Self {
            bitmap_threshold: bitmap_default(),
            box_threshold: box_default(),
            unclip_ratio: unclip_default(),
            line_threshold: line_default(),
            layout_threshold: layout_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    version: u32,
    python: PathBuf,
    script: PathBuf,
    pipeline_id: String,
    feature_schema: String,
    models: Assets,
    #[serde(default)]
    options: Options,
}

#[derive(Clone)]
pub(super) struct Configuration {
    pub identity: String,
    pub fallback_reason: Option<String>,
    enhanced: Option<(PathBuf, Manifest, String)>,
}

impl Configuration {
    pub(super) fn is_enhanced(&self) -> bool {
        self.enhanced.is_some()
    }
}

fn manifest_setting() -> &'static RwLock<Option<PathBuf>> {
    static SETTING: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    SETTING.get_or_init(|| RwLock::new(None))
}

pub(super) fn set_manifest_setting(value: &str) {
    let selected = (!value.trim().is_empty()).then(|| PathBuf::from(value));
    *manifest_setting()
        .write()
        .unwrap_or_else(|error| error.into_inner()) = selected;
}

fn selected_manifest(candidate: Option<&str>) -> (Option<PathBuf>, &'static str) {
    let setting = match candidate {
        Some(value) => (!value.trim().is_empty()).then(|| PathBuf::from(value)),
        None => manifest_setting()
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone(),
    };
    if setting.is_some() {
        return (setting, "settings");
    }
    match std::env::var_os(MANIFEST_ENV) {
        Some(path) => (Some(PathBuf::from(path)), "environment"),
        None => (None, "none"),
    }
}

pub(super) fn configured() -> bool {
    selected_manifest(None).0.is_some()
}

fn read_bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|_| "OCR 配置或运行文件不可读取".to_string())?;
    let mut result = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut result)
        .map_err(|_| "OCR 配置或运行文件读取失败".to_string())?;
    if result.len() as u64 > limit {
        return Err("OCR 配置或运行文件超过大小上限".into());
    }
    Ok(result)
}

fn load(path: &Path) -> Result<Configuration, String> {
    if !path.is_absolute() {
        return Err("OCR manifest 必须为绝对路径".into());
    }
    if !path.is_file() {
        return Err("OCR manifest 不可读取".into());
    }
    let raw = read_bounded_file(path, 65536)?;
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|_| "OCR manifest 格式错误".to_string())?;
    if manifest.version != 1
        || manifest.feature_schema != "clippy-edge-features-v1"
        || manifest.pipeline_id.is_empty()
        || manifest.pipeline_id.len() > 128
    {
        return Err("OCR manifest 运行合同无效".into());
    }
    if !manifest.python.is_absolute() {
        return Err("OCR Python 路径无效".into());
    }
    if !manifest.python.is_file() {
        return Err("OCR Python 不可读取".into());
    }
    if !manifest.script.is_absolute() {
        return Err("OCR 脚本路径无效".into());
    }
    if !manifest.script.is_file() {
        return Err("OCR 脚本不可读取".into());
    }
    for value in [
        manifest.options.bitmap_threshold,
        manifest.options.box_threshold,
        manifest.options.line_threshold,
        manifest.options.layout_threshold,
    ] {
        if !value.is_finite() || value <= 0.0 || value > 1.0 {
            return Err("OCR 阈值无效".into());
        }
    }
    if !manifest.options.unclip_ratio.is_finite()
        || manifest.options.unclip_ratio <= 0.0
        || manifest.options.unclip_ratio > 3.0
    {
        return Err("OCR 扩框参数无效".into());
    }
    let mut assets = vec![
        ("det", &manifest.models.det),
        ("rec", &manifest.models.rec),
        ("dictionary", &manifest.models.dictionary),
        ("edge", &manifest.models.edge),
    ];
    match (
        manifest.models.english_rec.as_ref(),
        manifest.models.english_dictionary.as_ref(),
    ) {
        (Some(rec), Some(dictionary)) => {
            assets.push(("englishRec", rec));
            assets.push(("englishDictionary", dictionary));
        }
        (None, None) => {}
        _ => return Err("OCR 英语模型必须成对配置".into()),
    }
    for (role, asset) in assets {
        if !asset.path.is_absolute() {
            return Err(format!("OCR 模型路径无效:{role}"));
        }
        if !asset.path.is_file() {
            return Err(format!("OCR 模型缺失:{role}"));
        }
        if asset.sha256.len() != 64
            || !asset
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(format!("OCR 模型身份无效:{role}"));
        }
        // 命中结果缓存之前也核对真实资产，坏文件不能被旧的成功文本遮住。
        let mut file = File::open(&asset.path).map_err(|_| format!("OCR 模型不可读取:{role}"))?;
        let size = file
            .metadata()
            .map_err(|_| format!("OCR 模型大小不可读取:{role}"))?
            .len();
        if size == 0 || size > 64 * 1024 * 1024 {
            return Err(format!("OCR 模型超过大小上限:{role}"));
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| format!("OCR 模型读取失败:{role}"))?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > 64 * 1024 * 1024 {
                return Err(format!("OCR 模型超过大小上限:{role}"));
            }
            hash.update(&buffer[..count]);
        }
        if format!("{:x}", hash.finalize()) != asset.sha256 {
            return Err(format!("OCR 模型哈希不一致:{role}"));
        }
    }
    let directory = manifest
        .script
        .parent()
        .ok_or_else(|| "OCR 脚本目录无效".to_string())?;
    let mut digest = Sha256::new();
    digest.update(b"structured-ocr-protocol-v1\0");
    digest.update(&raw);
    digest.update(read_bounded_file(&manifest.script, 1024 * 1024)?);
    for module in MODULES {
        if !directory.join(module).is_file() {
            return Err(format!("OCR 运行模块缺失:{module}"));
        }
        digest.update(module.as_bytes());
        digest.update(read_bounded_file(&directory.join(module), 1024 * 1024)?);
    }
    Ok(Configuration {
        identity: format!("{}:{:x}", manifest.pipeline_id, digest.finalize()),
        fallback_reason: None,
        enhanced: Some((
            path.to_path_buf(),
            manifest,
            format!("{:x}", Sha256::digest(&raw)),
        )),
    })
}

pub(super) fn configuration() -> Configuration {
    match selected_manifest(None).0 {
        None => Configuration {
            identity: "tesseract-v1".into(),
            enhanced: None,
            fallback_reason: None,
        },
        Some(path) => match load(&path) {
            Ok(configuration) => configuration,
            Err(error) => {
                log::warn!("OCR 增强配置不可用: {error}");
                Configuration {
                    identity: "enhanced-invalid-v1".into(),
                    enhanced: None,
                    fallback_reason: Some("enhanced_configuration_invalid".into()),
                }
            }
        },
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OcrModelIdentity {
    role: &'static str,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OcrHealthStatus {
    pub available: bool,
    pub active_engine: &'static str,
    pub enhanced_state: &'static str,
    pub configuration_source: &'static str,
    pub pipeline_id: Option<String>,
    pub runtime_identity: Option<String>,
    pub model_identities: Vec<OcrModelIdentity>,
    pub tesseract_available: bool,
    pub fallback_reason: Option<&'static str>,
    pub issue_code: Option<&'static str>,
    pub missing_items: Vec<String>,
}

fn validation_issue(error: &str) -> (&'static str, Vec<String>) {
    let suffix = || error.split_once(':').map(|(_, value)| value.to_string());
    if error.contains("manifest 必须为绝对路径") {
        ("manifest_path_invalid", vec!["manifest".into()])
    } else if error.contains("manifest 不可读取") {
        ("manifest_missing", vec!["manifest".into()])
    } else if error.contains("manifest 格式错误") {
        ("manifest_format_invalid", vec!["manifest".into()])
    } else if error.contains("manifest 运行合同无效") || error.contains("成对配置") {
        ("manifest_contract_invalid", vec!["manifest".into()])
    } else if error.contains("Python") {
        ("python_missing", vec!["python".into()])
    } else if error.contains("脚本") {
        ("script_missing", vec!["script".into()])
    } else if error.contains("阈值") || error.contains("扩框参数") {
        ("parameters_invalid", vec!["manifest".into()])
    } else if error.contains("模型缺失") {
        ("model_missing", suffix().into_iter().collect())
    } else if error.contains("模型路径无效") || error.contains("模型身份无效") {
        ("model_identity_invalid", suffix().into_iter().collect())
    } else if error.contains("模型哈希不一致") {
        ("model_hash_mismatch", suffix().into_iter().collect())
    } else if error.contains("模型超过大小上限") {
        ("model_too_large", suffix().into_iter().collect())
    } else if error.contains("模型") {
        ("model_unreadable", suffix().into_iter().collect())
    } else if error.contains("运行模块缺失") {
        ("runtime_module_missing", suffix().into_iter().collect())
    } else {
        ("runtime_invalid", Vec::new())
    }
}

fn status_from(
    path: Option<PathBuf>,
    source: &'static str,
    tesseract_available: bool,
) -> OcrHealthStatus {
    let Some(path) = path else {
        return OcrHealthStatus {
            available: tesseract_available,
            active_engine: if tesseract_available {
                "tesseract"
            } else {
                "unavailable"
            },
            enhanced_state: "not_configured",
            configuration_source: source,
            pipeline_id: None,
            runtime_identity: None,
            model_identities: Vec::new(),
            tesseract_available,
            fallback_reason: None,
            issue_code: None,
            missing_items: vec!["manifest".into()],
        };
    };
    match load(&path) {
        Ok(configuration) => {
            let (_, manifest, _) = configuration
                .enhanced
                .as_ref()
                .expect("load 成功必须有增强配置");
            OcrHealthStatus {
                available: true,
                active_engine: "ppocrv6+edgegnn",
                enhanced_state: "ready",
                configuration_source: source,
                pipeline_id: Some(manifest.pipeline_id.clone()),
                runtime_identity: Some(configuration.identity.clone()),
                model_identities: {
                    let mut identities = vec![
                        ("det", &manifest.models.det),
                        ("rec", &manifest.models.rec),
                        ("dictionary", &manifest.models.dictionary),
                        ("edge", &manifest.models.edge),
                    ];
                    if let (Some(rec), Some(dictionary)) = (
                        manifest.models.english_rec.as_ref(),
                        manifest.models.english_dictionary.as_ref(),
                    ) {
                        identities.push(("englishRec", rec));
                        identities.push(("englishDictionary", dictionary));
                    }
                    identities
                        .into_iter()
                        .map(|(role, asset)| OcrModelIdentity {
                            role,
                            sha256: asset.sha256.clone(),
                        })
                        .collect()
                },
                tesseract_available,
                fallback_reason: None,
                issue_code: None,
                missing_items: Vec::new(),
            }
        }
        Err(error) => {
            let (issue_code, missing_items) = validation_issue(&error);
            OcrHealthStatus {
                available: tesseract_available,
                active_engine: if tesseract_available {
                    "tesseract"
                } else {
                    "unavailable"
                },
                enhanced_state: "invalid",
                configuration_source: source,
                pipeline_id: None,
                runtime_identity: None,
                model_identities: Vec::new(),
                tesseract_available,
                fallback_reason: Some("enhanced_configuration_invalid"),
                issue_code: Some(issue_code),
                missing_items,
            }
        }
    }
}

pub(crate) fn health_status(candidate: &str) -> OcrHealthStatus {
    let (path, source) = selected_manifest(Some(candidate));
    status_from(
        path,
        source,
        super::executable::tesseract_executable().is_some(),
    )
}

pub(super) fn image_dimensions(png: &[u8]) -> Result<(u32, u32), String> {
    if png.len() > MAX_INPUT {
        return Err("OCR 图片超过字节上限".into());
    }
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "OCR 图片不是有效 PNG".to_string())?;
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("OCR 图片超过像素上限".into());
    }
    Ok((width, height))
}

pub(super) fn request_key(png: &[u8], configuration: &Configuration) -> String {
    format!("{}:{:x}", configuration.identity, Sha256::digest(png))
}

type Cache = VecDeque<(String, StructuredOcr)>;
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn cached(key: &str) -> Option<StructuredOcr> {
    cache()
        .lock()
        .ok()?
        .iter()
        .find(|(identity, _)| identity == key)
        .map(|(_, result)| result.clone())
}

fn remember(key: String, result: &StructuredOcr) {
    // 有 fallbackReason 的结果不能遮住修好的配置/模型；单次输出有界，最多缓存8项。
    if result.fallback_reason.is_some() {
        return;
    }
    if let Ok(mut cache) = cache().lock() {
        cache.retain(|(identity, _)| identity != &key);
        cache.push_back((key, result.clone()));
        while cache.len() > 8 {
            cache.pop_front();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reply {
    version: u32,
    request_id: String,
    result: StructuredOcr,
}

async fn enhanced(
    png: &[u8],
    configuration: &Configuration,
    width: u32,
    height: u32,
    deadline: Instant,
) -> Result<StructuredOcr, String> {
    let (path, manifest, manifest_hash) = configuration
        .enhanced
        .as_ref()
        .ok_or_else(|| "OCR 增强未配置".to_string())?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("OCR 识别超时".into());
    }
    let request_id = format!("{:x}", Sha256::digest(png));
    let mut input = serde_json::to_vec(&serde_json::json!({"version":1,"requestId":request_id,"pngBytes":png.len(),"deadlineMs":remaining.as_millis().min(60000) as u64})).map_err(|_| "OCR 请求编码失败".to_string())?;
    input.push(b'\n');
    input.extend_from_slice(png);
    let mut command = tokio::process::Command::new(&manifest.python);
    command
        .arg("-I")
        .arg(&manifest.script)
        .arg("--manifest")
        .arg(path)
        .arg("--manifest-sha256")
        .arg(manifest_hash);
    command.env_remove("PYTHONPATH").env_remove("PYTHONHOME");
    let output = run_recognition_process(
        command,
        &input,
        remaining,
        OCR_STDOUT_LIMIT,
        OCR_STDERR_LIMIT,
    )
    .await?;
    let output = String::from_utf8(output)
        .map(|text| text.trim().to_string())
        .map_err(|_| "OCR 输出不是有效 UTF-8".to_string())?;
    let mut reply: Reply =
        serde_json::from_str(&output).map_err(|_| "OCR 增强结果格式错误".to_string())?;
    if reply.version != 1 || reply.request_id != request_id {
        return Err("OCR 增强请求身份不一致".into());
    }
    reply
        .result
        .validate_enhanced(width, height, &manifest.pipeline_id)?;
    // 在进程运行期间本地热更新脚本/模型时，拒绝把新运行结果写入旧身份缓存。
    let check_path = path.clone();
    let checked = tauri::async_runtime::spawn_blocking(move || load(&check_path))
        .await
        .map_err(|_| "OCR 配置复核失败".to_string())??;
    if Instant::now() >= deadline {
        return Err("OCR 识别超时".into());
    }
    if checked.identity != configuration.identity {
        return Err("OCR 运行配置在识别期间已变更".into());
    }
    reply.result.pipeline.id.clone_from(&configuration.identity);
    Ok(reply.result)
}

fn budget_failure(error: &str) -> bool {
    // Sidecar 稳定错误码与父进程I/O预算都禁止启动第二引擎。
    error.contains("超时")
        || error.contains("上限")
        || error.contains("_budget")
        || error.contains("ocr_deadline")
}

/// 调用者已经持有全局许可，不能在这里递归进入 OCR 调度器。
pub(super) async fn recognize_owned(
    png: Arc<Vec<u8>>,
    configuration: Configuration,
    cancelled: impl Fn() -> bool + Send,
) -> Result<StructuredOcr, String> {
    let (width, height) = image_dimensions(&png)?;
    let key = request_key(&png, &configuration);
    if let Some(result) = cached(&key) {
        return Ok(result);
    }
    if cancelled() {
        return Err("OCR 请求已取消".into());
    }
    let deadline = Instant::now() + RECOGNITION_TIMEOUT;
    let mut reason = configuration.fallback_reason.clone();
    if configuration.enhanced.is_some() {
        match enhanced(&png, &configuration, width, height, deadline).await {
            Ok(result) => {
                remember(key, &result);
                return Ok(result);
            }
            Err(error) => {
                log::warn!("OCR 增强识别失败: {error}");
                if Instant::now() >= deadline || budget_failure(&error) {
                    return Err("OCR 增强识别超过预算，请缩小区域或重试".into());
                }
                reason = Some("enhanced_failed".into());
            }
        }
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining < Duration::from_millis(1) {
        return Err("OCR 识别超时".into());
    }
    if cancelled() {
        return Err("OCR 请求已取消".into());
    }
    // 冷探测仍在 blocking pool；其等待也计入同一预算，迟到探测自身有kill/wait上限。
    let executable = tauri::async_runtime::spawn_blocking(move || {
        super::executable::tesseract_executable_until(deadline)
    })
    .await
    .map_err(|_| "OCR 探测线程异常".to_string())?
    .ok_or_else(|| super::executable::missing_tesseract_message().to_string())?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("OCR 识别超时".into());
    }
    // 增强进程/冷探测结束后再查消费者；取消不能新启动第二引擎。
    if cancelled() {
        return Err("OCR 请求已取消".into());
    }
    let text = tesseract::recognize_with_timeout(
        &executable,
        &png,
        remaining,
        OCR_STDOUT_LIMIT,
        OCR_STDERR_LIMIT,
    )
    .await?;
    let result = StructuredOcr::tesseract(width, height, text, reason);
    remember(key, &result);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("main.py");
        std::fs::write(&script, b"# main\n").unwrap();
        for module in MODULES {
            std::fs::write(dir.path().join(module), module).unwrap();
        }
        let model = dir.path().join("model");
        std::fs::write(&model, b"fixture model").unwrap();
        let asset = serde_json::json!({"path":model,"sha256":format!("{:x}",Sha256::digest(b"fixture model"))});
        let manifest = serde_json::json!({"version":1,"python":script,"script":script,"pipelineId":"test-pipeline","featureSchema":"clippy-edge-features-v1","models":{"det":asset,"rec":asset,"dictionary":asset,"edge":asset}});
        let path = dir.path().join("manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        (dir, path)
    }

    #[test]
    fn identity_covers_all_runtime_modules_and_model_changes_cannot_hit_cache() {
        let (directory, path) = fixture();
        let original = load(&path).unwrap().identity;
        for module in MODULES {
            std::fs::write(directory.path().join(module), format!("{module} changed")).unwrap();
            let changed = load(&path).unwrap().identity;
            assert_ne!(original, changed, "{module}");
            std::fs::write(directory.path().join(module), module).unwrap();
        }
        assert_eq!(original, load(&path).unwrap().identity);
        std::fs::write(directory.path().join("model"), b"fixture Model").unwrap();
        assert!(load(&path).is_err(), "同路径同长度坏模型不可复用成功缓存");
    }

    #[test]
    fn rejects_relative_unknown_and_out_of_range_configuration() {
        assert!(load(Path::new("relative.json")).is_err());
        let (_directory, path) = fixture();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["options"] = serde_json::json!({"layoutThreshold":2});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&path).is_err());
        value["options"] = serde_json::json!({"unknown":0.5});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn health_status_reports_verified_pipeline_and_model_identities() {
        let (_directory, path) = fixture();
        let status = status_from(Some(path), "settings", false);
        assert!(status.available);
        assert_eq!(status.active_engine, "ppocrv6+edgegnn");
        assert_eq!(status.enhanced_state, "ready");
        assert_eq!(status.pipeline_id.as_deref(), Some("test-pipeline"));
        assert_eq!(status.model_identities.len(), 4);
        assert!(status.issue_code.is_none());
    }

    #[test]
    fn optional_english_assets_are_paired_validated_and_reported() {
        let (_directory, path) = fixture();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let asset = manifest["models"]["rec"].clone();
        manifest["models"]["englishRec"] = asset.clone();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(load(&path).err().unwrap().contains("成对配置"));

        manifest["models"]["englishDictionary"] = asset;
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let status = status_from(Some(path), "settings", false);
        assert_eq!(status.model_identities.len(), 6);
        assert_eq!(status.model_identities[4].role, "englishRec");
        assert_eq!(status.model_identities[5].role, "englishDictionary");
    }

    #[test]
    fn health_status_keeps_tesseract_fallback_and_names_bad_model_role() {
        let (directory, path) = fixture();
        std::fs::write(directory.path().join("model"), b"tampered model").unwrap();
        let status = status_from(Some(path), "settings", true);
        assert!(status.available);
        assert_eq!(status.active_engine, "tesseract");
        assert_eq!(status.enhanced_state, "invalid");
        assert_eq!(status.issue_code, Some("model_hash_mismatch"));
        assert_eq!(status.missing_items, ["det"]);
        assert_eq!(
            status.fallback_reason,
            Some("enhanced_configuration_invalid")
        );
    }

    #[test]
    fn health_status_reports_missing_python_with_a_stable_code() {
        let (directory, path) = fixture();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["python"] = serde_json::Value::String(
            directory
                .path()
                .join("missing-python")
                .to_string_lossy()
                .into_owned(),
        );
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let status = status_from(Some(path), "settings", false);
        assert!(!status.available);
        assert_eq!(status.issue_code, Some("python_missing"));
        assert_eq!(status.missing_items, ["python"]);
    }

    #[test]
    fn health_status_without_any_manifest_is_explicit_about_tesseract_only() {
        let status = status_from(None, "none", true);
        assert_eq!(status.active_engine, "tesseract");
        assert_eq!(status.enhanced_state, "not_configured");
        assert_eq!(status.missing_items, ["manifest"]);
    }

    #[test]
    fn sidecar_stage_budget_and_deadline_do_not_allow_fallback() {
        for code in [
            "det_tile_budget",
            "rec_width_budget",
            "layout_node_budget",
            "ocr_deadline",
            "OCR 输出超过上限",
            "OCR 识别超时",
        ] {
            assert!(budget_failure(code), "{code}");
        }
        assert!(!budget_failure("model_hash_mismatch"));
        assert!(!budget_failure("runtime_failure"));
    }

    #[test]
    fn fallback_results_are_not_reused_as_an_enhanced_success() {
        let result =
            StructuredOcr::tesseract(1, 1, "fallback".into(), Some("enhanced_failed".into()));
        let key = "test-fallback-never-remembered".to_string();
        remember(key.clone(), &result);
        assert!(cached(&key).is_none());
    }

    #[tokio::test]
    #[ignore = "需要显式配置本地增强模型和合成PNG，仅在隔离验收运行"]
    async fn configured_real_pipeline_through_rust_supervisor() {
        let png_path = std::env::var_os("CLIPPY_OCR_TEST_PNG").expect("CLIPPY_OCR_TEST_PNG");
        let png = std::fs::read(png_path).unwrap();
        let result = super::super::recognize_snapshot(png).await.unwrap();
        assert_eq!(
            result.pipeline.engine, "ppocrv6+edgegnn",
            "{:?}",
            result.fallback_reason
        );
        assert!(result.fallback_reason.is_none());
        assert!(!result.lines.is_empty());
        assert!(!result.text.is_empty());
        eprintln!("{}", serde_json::to_string(&result).unwrap());
    }
    /// manifest 合同要求解释器是绝对路径的真实文件，所以从 PATH 解析而不是写死
    /// `/usr/bin/python3`：那个路径在 macOS 上只是 Command Line Tools 的 shim，不是解释器本体，
    /// 启动行为与真解释器不同。同目录的 `process_tests` 一直用 PATH 里的 `python3`，在同一个
    /// macOS runner 上全部通过，因此这里对齐同一约定。
    #[cfg(unix)]
    fn path_python3() -> PathBuf {
        let search = std::env::var_os("PATH").expect("PATH 未设置");
        std::env::split_paths(&search)
            .map(|directory| directory.join("python3"))
            .find(|candidate| candidate.is_absolute() && candidate.is_file())
            .expect("本测试需要 PATH 中的 python3（与 process_tests 相同的前置条件）")
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn cancelled_after_enhanced_non_timeout_failure_does_not_start_fallback() {
        let (directory, path) = fixture();
        let marker = directory.path().join("started");
        let script = directory.path().join("main.py");
        std::fs::write(&script,format!("import sys,time\nopen({:?},'w').write('started')\nsys.stdin.buffer.read()\ntime.sleep(.15)\nsys.exit(7)\n",marker)).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let interpreter = path_python3();
        manifest["python"] = serde_json::to_value(&interpreter).unwrap();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let configuration = load(&path).unwrap();
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let job = tokio::spawn(recognize_owned(
            Arc::new(output.into_inner()),
            configuration,
            move || signal.load(std::sync::atomic::Ordering::SeqCst),
        ));
        // 这里断言的是"是否启动"而不是"多快启动"：冷解释器在负载中的 runner 上可能超过 2s，
        // 放宽启动窗口不削弱断言，进程始终没起来照样红。RECOGNITION_TIMEOUT 是 60s，且取消判定
        // 发生在 enhanced 失败之后，15s 仍在同一预算内。
        let start_budget = Duration::from_secs(15);
        let deadline = Instant::now() + start_budget;
        while !marker.exists() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // 失败信息带上解释器：headless CI 只留这一行，否则无法区分"解释器起不来"和"启动太慢"。
        assert!(
            marker.exists(),
            "增强假进程必须实际启动（解释器 {interpreter:?}，等待 {start_budget:?} 后标记文件仍不存在）"
        );
        cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            job.await.unwrap().unwrap_err().contains("已取消"),
            "不能探测或运行Tesseract fallback"
        );
    }
}

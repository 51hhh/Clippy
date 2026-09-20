#[cfg(target_os = "linux")]
pub(super) mod x11;

/// 平台帧源完成原生显示器核验后生成的唯一物理来源描述。
///
/// 录制清单、控制窗规划和后续编码配置必须消费这份描述，不能再用前端逻辑坐标重复推导。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingSourceDescriptor {
    pub source_id: String,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
}

//! 错误类型定义。
//!
//! 采用 [`thiserror`] 定义结构化的领域错误，便于在命令层区分“参数错误”“校验失败”
//! “外部命令失败”等不同情况；二进制入口（`main`）再用 `anyhow` 添加上下文。

use std::path::PathBuf;

/// 统一的 `Result` 别名。
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// `rsctl` 可能产生的错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 底层 I/O 错误。
    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),

    /// 配置文件解析或取值错误。
    #[error("配置错误：{0}")]
    Config(String),

    /// 外部命令以非零退出码结束。
    #[error("命令 `{program}` 执行失败（退出状态 {status}）：{stderr}")]
    CommandFailed {
        /// 命令名。
        program: String,
        /// 退出码或信号描述。
        status: String,
        /// 标准错误输出。
        stderr: String,
    },

    /// 需要的系统命令不存在。
    #[error("未找到命令 `{program}`，请确认系统已安装相应组件")]
    CommandNotFound {
        /// 命令名。
        program: String,
    },

    /// 外部命令输出无法解析。
    #[error("解析 `{program}` 的输出失败：{message}")]
    ParseOutput {
        /// 命令名。
        program: String,
        /// 说明信息。
        message: String,
    },

    /// TOML 文件解析失败。
    #[error("TOML 解析失败（{path}）：{source}")]
    Toml {
        /// 文件路径。
        path: PathBuf,
        /// 原始解析错误。
        #[source]
        source: toml::de::Error,
    },

    /// JSON 序列化 / 反序列化失败。
    #[error("JSON 处理失败：{0}")]
    Json(#[from] serde_json::Error),

    /// SQLite 访问失败。
    #[error("数据库错误：{0}")]
    Database(#[from] rusqlite::Error),

    /// 调用方传入的参数非法。
    #[error("无效参数：{0}")]
    InvalidArgument(String),

    /// 安装前校验未通过。
    #[error("校验失败：{0}")]
    Validation(String),

    /// 目标软件包未安装。
    #[error("软件包 `{0}` 未安装")]
    PackageNotInstalled(String),

    /// 目标服务不存在。
    #[error("服务 `{0}` 不存在")]
    ServiceNotFound(String),

    /// 当前系统环境缺少所需能力。
    #[error("系统不支持：{0}")]
    Unsupported(String),

    /// 应用级变更锁被占用。
    #[error("变更锁被占用：{0}")]
    LockBusy(String),

    /// 需要 root 权限才能执行的变更操作。
    #[error("该操作需要 root 权限，请使用 sudo 执行：{0}")]
    PermissionDenied(String),

    /// 存在未完成的部署任务。
    #[error("存在未完成的部署任务 {task_id}（状态 {state}），请先处理后再执行新的变更")]
    IncompleteTask {
        /// 任务 ID。
        task_id: String,
        /// 任务当前状态。
        state: String,
    },
}

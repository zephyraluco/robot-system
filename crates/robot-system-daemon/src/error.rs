//! 常驻服务 `robot-system-daemon` 的错误类型。
//!
//! 该程序与 `rsctl` 是**两个独立实现**，各自定义自己的错误类型（见架构文档 §1）。

/// 统一的 `Result` 别名。
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// 常驻服务可能产生的错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 底层 I/O 错误。
    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),

    /// 配置错误。
    #[error("配置错误：{0}")]
    Config(String),

    /// SQLite 访问失败。
    #[error("数据库错误：{0}")]
    Database(#[from] rusqlite::Error),

    /// JSON 处理失败。
    #[error("JSON 处理失败：{0}")]
    Json(#[from] serde_json::Error),

    /// TOML 解析失败。
    #[error("TOML 解析失败（{path}）：{source}")]
    Toml {
        /// 文件路径。
        path: std::path::PathBuf,
        /// 原始错误。
        #[source]
        source: toml::de::Error,
    },

    /// 外部命令非零退出。
    #[error("命令 `{program}` 执行失败（退出状态 {status}）：{stderr}")]
    CommandFailed {
        /// 命令名。
        program: String,
        /// 退出码或信号。
        status: String,
        /// 标准错误。
        stderr: String,
    },

    /// 外部命令不存在。
    #[error("未找到命令 `{program}`")]
    CommandNotFound {
        /// 命令名。
        program: String,
    },

    /// 数据库迁移失败。
    #[error("数据库迁移失败：{0}")]
    Migration(String),

    /// 参数非法。
    #[error("无效参数：{0}")]
    InvalidArgument(String),
}

//! 系统适配层：封装 dpkg / APT、systemd、journald、`/proc` 等 Linux 原生能力，
//! 避免业务模块直接依赖底层命令细节（见架构文档 §3）。

pub mod command;
pub mod dpkg;
pub mod journald;
pub mod procfs;
pub mod systemd;

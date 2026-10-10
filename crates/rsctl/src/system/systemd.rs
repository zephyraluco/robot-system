//! systemd 适配层（见架构文档 §3.1、§4.1）。
//!
//! 更理想的实现是 systemd D-Bus API；当前通过 `systemctl` 实现，并统一使用
//! 参数数组调用、统一解析返回结果，接口保持不变，后续可替换为 D-Bus 实现。

use std::collections::BTreeSet;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::system::command;

/// 服务（或 target）状态快照。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ServiceStatus {
    /// unit 名称。
    pub unit: String,
    /// 加载状态：`loaded`、`not-found` 等。
    pub load_state: String,
    /// 活动状态：`active`、`inactive`、`failed` 等。
    pub active_state: String,
    /// 子状态：`running`、`dead`、`exited` 等。
    pub sub_state: String,
    /// 单元文件启用状态：`enabled`、`disabled`、`static` 等。
    pub unit_file_state: String,
    /// 主进程 PID（0 表示无）。
    pub main_pid: u32,
    /// 主进程退出方式代码（systemd 数值枚举）。
    pub exec_main_code: i32,
    /// 主进程退出状态码 / 信号。
    pub exec_main_status: i32,
    /// 服务结果：`success`、`exit-code`、`signal` 等。
    pub result: String,
    /// systemd 记录的重启次数。
    pub n_restarts: u32,
}

impl ServiceStatus {
    /// 单元是否存在于系统中。
    #[must_use]
    pub fn exists(&self) -> bool {
        !self.load_state.is_empty() && self.load_state != "not-found"
    }

    /// 服务是否处于活动状态。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active_state == "active"
    }

    /// 服务是否已启用（开机自启）。
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        matches!(
            self.unit_file_state.as_str(),
            "enabled" | "enabled-runtime" | "static"
        )
    }
}

/// `systemctl list-units` 中的一个单元条目。
#[derive(Debug, Clone, Serialize)]
pub struct UnitEntry {
    /// 单元名。
    pub unit: String,
    /// 加载状态。
    pub load_state: String,
    /// 活动状态。
    pub active_state: String,
    /// 子状态。
    pub sub_state: String,
    /// 描述。
    pub description: String,
}

const SHOW_PROPERTIES: &str = "LoadState,ActiveState,SubState,UnitFileState,MainPID,\
ExecMainCode,ExecMainStatus,Result,NRestarts";

/// 查询单个 unit 的状态。
///
/// 对于不存在的 unit，`systemctl show` 会输出 `LoadState=not-found`（不同 systemd 版本
/// 的退出码不完全一致），因此这里**解析输出优先**，仅在输出完全不可用时才报错，
/// 使调用方能够区分“不存在”与“查询失败”。
pub fn show(unit: &str) -> Result<ServiceStatus> {
    let out = command::run(
        "systemctl",
        &[
            "show",
            unit,
            "--no-pager",
            &format!("--property={SHOW_PROPERTIES}"),
        ],
    )?;

    let mut status = parse_show_output(&out.stdout);
    if !out.success() && out.stdout.trim().is_empty() {
        return Err(Error::CommandFailed {
            program: "systemctl".to_string(),
            status: out.status_text(),
            stderr: out.stderr.trim().to_string(),
        });
    }
    if status.load_state.is_empty() {
        status.load_state = "not-found".to_string();
    }
    status.unit = unit.to_string();
    Ok(status)
}

/// 列出所有 service 单元。
pub fn list_services() -> Result<Vec<UnitEntry>> {
    let out = command::run_checked(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--all",
            "--no-legend",
            "--plain",
            "--no-pager",
        ],
    )?;
    Ok(parse_list_units(&out.stdout))
}

/// 启动 unit。
pub fn start(unit: &str) -> Result<()> {
    command::run_checked("systemctl", &["start", unit])?;
    Ok(())
}

/// 停止 unit。
pub fn stop(unit: &str) -> Result<()> {
    command::run_checked("systemctl", &["stop", unit])?;
    Ok(())
}

/// 重启 unit。
pub fn restart(unit: &str) -> Result<()> {
    command::run_checked("systemctl", &["restart", unit])?;
    Ok(())
}

/// 启用 unit（建立开机启动关系）。
pub fn enable(unit: &str) -> Result<()> {
    command::run_checked("systemctl", &["enable", unit])?;
    Ok(())
}

/// 禁用 unit。
pub fn disable(unit: &str) -> Result<()> {
    command::run_checked("systemctl", &["disable", unit])?;
    Ok(())
}

/// 执行 `systemctl daemon-reload`。
pub fn daemon_reload() -> Result<()> {
    command::run_checked("systemctl", &["daemon-reload"])?;
    Ok(())
}

/// systemd 树形输出中每一层占用的字符宽度（`├─`、`└─`、`│ `、`  ` 都是 2 个字符）。
const TREE_CELL_WIDTH: usize = 2;

/// 查询某个 target **直接**依赖的 systemd 服务单元名（去重后按名称排序）。
///
/// 业务服务的 unit 文件由各自的 DEB 提供，本系统不保存业务服务的副本；因此“需要监听
/// 哪些服务”以 systemd 的依赖关系为准：业务服务通过 `WantedBy=<target>` 声明归属，
/// `systemctl enable` 会在 `<target>.wants/` 下建立链接，从而成为 target 的直接依赖。
///
/// 只取**直接**依赖（第 1 层）。必须解析树形输出：`--plain` 会把不同层级拍平到同一
/// 缩进，无法区分直接依赖与传递依赖（例如业务服务声明的 `Wants=network-online.target`
/// 会把一批系统服务带进来）。
pub fn service_dependencies(target: &str) -> Result<Vec<String>> {
    validate_unit_name(target)?;
    let out = command::run("systemctl", &["list-dependencies", target, "--no-pager"])?;
    // unit 不存在时多数版本仍以 0 退出并只打印自身名称；仅在完全无输出时报错。
    if !out.success() && out.stdout.trim().is_empty() {
        return Err(Error::CommandFailed {
            program: "systemctl".to_string(),
            status: out.status_text(),
            stderr: out.stderr.trim().to_string(),
        });
    }
    Ok(parse_service_dependencies(&out.stdout))
}

/// 从 `systemctl list-dependencies` 的树形输出中取出第 1 层的服务单元。
///
/// 行首可能带一个状态标记（`●` / `○` / `×`），它占用一个单元格但不代表层级。
/// 当所有条目都处于同一层时 systemd 会省略树形符号，此时按“每层 2 个空格”的缩进判断。
#[must_use]
pub fn parse_service_dependencies(text: &str) -> Vec<String> {
    let mut units = BTreeSet::new();
    for line in text.lines() {
        let Some((depth, name)) = parse_dependency_line(line.trim_end()) else {
            continue;
        };
        if depth == 1 && name.ends_with(".service") {
            units.insert(name.to_string());
        }
    }
    units.into_iter().collect()
}

/// 树形前缀中的分支符号。
///
/// UTF-8 环境下 systemd 使用 `├─` / `└─`；在 `LANG=C` 等无法输出 Unicode 的环境下会
/// 退回到 ASCII 的 `|-` / `` `- ``。两者都是 2 个字符的单元格，因此解析逻辑可以共用。
const BRANCH_CHARS: [char; 4] = ['\u{251c}', '\u{2514}', '|', '`'];

/// 解析一行树形输出，返回（层级, 单元名）；非条目行返回 `None`。
///
/// 层级由单元名之前的单元格数量决定：每个层级占 2 个字符（`├─`、`└─`、`│ `、`  ` 或
/// 对应的 ASCII 形式），行首的状态标记（`●` / `○` / `*` / `x`）也占一个单元格，但
/// 不代表层级。所有条目处于同一层时 systemd 会省略这些符号，此时改按缩进判断。
fn parse_dependency_line(line: &str) -> Option<(usize, &str)> {
    if line.trim().is_empty() {
        return None;
    }

    let chars: Vec<(usize, char)> = line.char_indices().collect();

    // 取最后一个分支符号：它是紧邻单元名的那个单元格。
    if let Some(branch) = chars.iter().rposition(|(_, c)| BRANCH_CHARS.contains(c)) {
        // 分支单元格占 2 个字符（`├─`、`|-`、`` `- ``），其后即单元名。
        let (name_offset, _) = *chars.get(branch + TREE_CELL_WIDTH)?;
        let cells = (branch + TREE_CELL_WIDTH) / TREE_CELL_WIDTH;
        // 行首的状态标记占用一个单元格，但不代表层级。
        let has_marker = !line.starts_with(' ');
        let depth = cells.saturating_sub(usize::from(has_marker));
        return Some((depth, line.get(name_offset..)?));
    }

    // 所有条目处于同一层时 systemd 会省略树形符号，此时层级由缩进决定。
    let indent = line.len() - line.trim_start().len();
    Some((indent / TREE_CELL_WIDTH, line.trim_start()))
}

/// 解析 `systemctl show` 的 `Key=Value` 输出。
#[must_use]
pub fn parse_show_output(text: &str) -> ServiceStatus {
    let mut status = ServiceStatus::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "LoadState" => status.load_state = value.to_string(),
            "ActiveState" => status.active_state = value.to_string(),
            "SubState" => status.sub_state = value.to_string(),
            "UnitFileState" => status.unit_file_state = value.to_string(),
            "MainPID" => status.main_pid = value.parse().unwrap_or(0),
            "ExecMainCode" => status.exec_main_code = value.parse().unwrap_or(0),
            "ExecMainStatus" => status.exec_main_status = value.parse().unwrap_or(0),
            "Result" => status.result = value.to_string(),
            "NRestarts" => status.n_restarts = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    status
}

/// 解析 `systemctl list-units --no-legend --plain` 的输出。
///
/// 会自动跳过尾部统计行（如 `2 loaded units listed.`）：这类行的首字段不是合法的单元名。
#[must_use]
pub fn parse_list_units(text: &str) -> Vec<UnitEntry> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let unit = parts.next()?;
            // 单元名总是带类型后缀（如 `.service`）；借此过滤统计行与空行。
            if !unit.contains('.') {
                return None;
            }
            let load_state = parts.next()?.to_string();
            let active_state = parts.next()?.to_string();
            let sub_state = parts.next()?.to_string();
            let description = parts.collect::<Vec<_>>().join(" ");
            Some(UnitEntry {
                unit: unit.to_string(),
                load_state,
                active_state,
                sub_state,
                description,
            })
        })
        .collect()
}

/// 校验服务名是否合法，避免把可疑输入传给 systemd。
pub fn validate_unit_name(unit: &str) -> Result<()> {
    if unit.is_empty() || unit.len() > 256 {
        return Err(Error::InvalidArgument(format!("服务名长度非法：{unit}")));
    }
    if unit
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | '\\' | ':'))
    {
        Ok(())
    } else {
        Err(Error::InvalidArgument(format!(
            "服务名包含非法字符：{unit}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_show_output() {
        let text = "LoadState=loaded\n\
                    ActiveState=active\n\
                    SubState=running\n\
                    UnitFileState=enabled\n\
                    MainPID=1234\n\
                    ExecMainCode=0\n\
                    ExecMainStatus=0\n\
                    Result=success\n\
                    NRestarts=3\n";
        let status = parse_show_output(text);
        assert!(status.exists());
        assert!(status.is_active());
        assert!(status.is_enabled());
        assert_eq!(status.main_pid, 1234);
        assert_eq!(status.n_restarts, 3);
    }

    #[test]
    fn detects_not_found_unit() {
        let status = parse_show_output("LoadState=not-found\nActiveState=inactive\n");
        assert!(!status.exists());
    }

    #[test]
    fn parses_list_units_output() {
        let text = "robot-lidar.service loaded active running Robot Lidar Service\n\
                    robot-camera.service loaded inactive dead Robot Camera Service\n\
                    \n\
                    2 loaded units listed.\n";
        let units = parse_list_units(text);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].unit, "robot-lidar.service");
        assert_eq!(units[0].description, "Robot Lidar Service");
        assert_eq!(units[1].active_state, "inactive");
    }

    #[test]
    fn rejects_illegal_unit_names() {
        assert!(validate_unit_name("robot-lidar.service").is_ok());
        assert!(validate_unit_name("robot-lidar; rm -rf /").is_err());
        assert!(validate_unit_name("").is_err());
    }

    #[test]
    fn parses_flat_dependency_list() {
        // systemd 在同层条目上省略树形符号（Ubuntu 24.04 systemd 255 的实际输出形状）。
        let text = "robot-system.target\n  robot-lidar.service\n  robot-camera.service\n";
        assert_eq!(
            parse_service_dependencies(text),
            vec!["robot-camera.service", "robot-lidar.service"]
        );
    }

    #[test]
    fn keeps_only_first_level_services() {
        // 层级 1 = target 的直接依赖；更深层级由业务服务的 `Wants=` 引入，不属于受管范围。
        let text = "robot-system.target\n\
                    ○ ├─robot-lidar.service\n\
                    ○ └─robot-camera.service\n\
                    ○   ├─robot-camera-helper.service\n\
                    ○   └─network-online.target\n\
                    ●     └─NetworkManager.service\n";
        assert_eq!(
            parse_service_dependencies(text),
            vec!["robot-camera.service", "robot-lidar.service"]
        );
    }

    #[test]
    fn handles_continuation_bars_and_targets() {
        let text = "graphical.target\n\
                    ● ├─display-manager.service\n\
                    ● └─multi-user.target\n\
                    ●   ├─dbus.service\n\
                    ●   └─basic.target\n\
                    ●     ├─systemd-journald.service\n";
        assert_eq!(
            parse_service_dependencies(text),
            vec!["display-manager.service"]
        );
    }

    #[test]
    fn empty_or_target_only_output_yields_nothing() {
        assert!(parse_service_dependencies("").is_empty());
        assert!(parse_service_dependencies("robot-system.target\n").is_empty());
        assert!(parse_service_dependencies("robot-system.target").is_empty());
    }

    #[test]
    fn ignores_non_service_units_and_duplicates() {
        let text = "robot-system.target\n\
                    ○ ├─robot-lidar.socket\n\
                    ○ ├─robot-lidar.timer\n\
                    ○ ├─robot-lidar.service\n\
                    ○ └─robot-lidar.service\n";
        assert_eq!(
            parse_service_dependencies(text),
            vec!["robot-lidar.service"]
        );
    }

    #[test]
    fn parses_ascii_tree_rendering() {
        // `LANG=C` 等非 UTF-8 环境下 systemd 使用 ASCII 树形符号（实机输出形状）。
        let text = "graphical.target\n\
                    * |-display-manager.service\n\
                    * `-multi-user.target\n\
                    *   |-apport.service\n\
                    *     `-deep.service\n";
        assert_eq!(
            parse_service_dependencies(text),
            vec!["display-manager.service"]
        );
    }

    #[test]
    fn parses_ascii_tree_with_continuation_bar() {
        let text = "robot-system.target\n\
                    * |-robot-lidar.service\n\
                    * `-robot-camera.service\n\
                    * | `-robot-camera-helper.service\n";
        assert_eq!(
            parse_service_dependencies(text),
            vec!["robot-camera.service", "robot-lidar.service"]
        );
    }
}

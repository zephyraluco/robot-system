//! systemd 适配层（见架构文档 §3.1、§4.1、§4.2、§4.3）。
//!
//! 常驻服务只**读取** systemd 状态，不执行服务变更操作（变更由 `rsctl` 负责，见 §2.3）。

use crate::error::Result;
use crate::system;

/// 服务状态快照。
#[derive(Debug, Clone, Default)]
pub struct ServiceStatus {
    /// 加载状态。
    pub load_state: String,
    /// 活动状态：`active` / `inactive` / `failed` / `activating` / `deactivating`。
    pub active_state: String,
    /// 子状态：`running` / `dead` / `exited` 等。
    pub sub_state: String,
    /// 启用状态。
    pub unit_file_state: String,
    /// 主进程 PID（0 表示无）。
    pub main_pid: u32,
    /// 主进程退出方式（systemd 数值枚举：1=exited，2=killed 等）。
    pub exec_main_code: i32,
    /// 主进程退出状态码或信号。
    pub exec_main_status: i32,
    /// 服务结果：`success` / `exit-code` / `signal` 等。
    pub result: String,
    /// 重启次数。
    pub n_restarts: u32,
    /// 服务类型：`simple` / `oneshot` / `forking` / `notify` 等。
    pub service_type: String,
}

impl ServiceStatus {
    /// 单元是否存在。
    #[must_use]
    pub fn exists(&self) -> bool {
        !self.load_state.is_empty() && self.load_state != "not-found"
    }

    /// 是否正在运行。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active_state == "active"
    }

    /// 是否处于失败状态。
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.active_state == "failed"
    }

    /// 是否处于过渡状态。
    #[must_use]
    pub fn is_transitioning(&self) -> bool {
        matches!(
            self.active_state.as_str(),
            "activating" | "deactivating" | "reloading"
        )
    }

    /// 是否为一次性服务（正常退出即视为完成，不应报告为意外停止）。
    #[must_use]
    pub fn is_oneshot(&self) -> bool {
        self.service_type == "oneshot"
    }

    /// 由 systemd 退出信息推导出的可确认结果。
    ///
    /// 无法确认退出原因时返回 `unknown`，**不伪造退出码**（见 §6.1、§8.2）。
    #[must_use]
    pub fn exit_details(&self) -> (Option<i64>, Option<i64>, &'static str) {
        // systemd 的 ExecMainCode：1 = exited，2 = killed（信号）。
        match self.exec_main_code {
            1 => {
                let code = i64::from(self.exec_main_status);
                let result = if self.result == "success" && code == 0 {
                    "exited"
                } else {
                    "failed"
                };
                (Some(code), None, result)
            }
            2 => (None, Some(i64::from(self.exec_main_status)), "failed"),
            3..=5 => (None, None, "failed"),
            _ => {
                if self.result == "success" {
                    (Some(0), None, "exited")
                } else {
                    (None, None, "unknown")
                }
            }
        }
    }
}

const SHOW_PROPERTIES: &str = "LoadState,ActiveState,SubState,UnitFileState,MainPID,\
ExecMainCode,ExecMainStatus,Result,NRestarts,Type";

/// 查询单个 unit 的状态。
///
/// 对不存在的 unit，`systemctl show` 会输出 `LoadState=not-found`；不同 systemd 版本的
/// 退出码并不统一，因此这里以**解析输出**为准。
pub fn show(unit: &str) -> Result<ServiceStatus> {
    let out = system::run(
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
        return Err(crate::error::Error::CommandFailed {
            program: "systemctl".to_string(),
            status: out.status_text(),
            stderr: out.stderr.trim().to_string(),
        });
    }
    if status.load_state.is_empty() {
        status.load_state = "not-found".to_string();
    }
    Ok(status)
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
            "Type" => status.service_type = value.to_string(),
            _ => {}
        }
    }
    status
}

/// systemd 树形输出中每一层占用的字符宽度（`├─`、`└─`、`│ `、`  ` 都是 2 个字符）。
const TREE_CELL_WIDTH: usize = 2;

/// 查询某个 target **直接**依赖的 systemd 服务单元名（去重后按名称排序）。
///
/// 业务服务的 unit 文件由各自的 DEB 提供，本系统不保存副本；常驻服务因此通过 systemd
/// 的依赖关系确定需要监听哪些服务：业务服务以 `WantedBy=<target>` 声明归属，
/// `systemctl enable` 会在 `<target>.wants/` 下建立链接，从而成为 target 的直接依赖。
///
/// 只取第 1 层。这里必须解析树形输出：`--plain` 会把不同层级拍平到同一缩进，无法区分
/// 直接依赖与传递依赖（例如业务服务声明的 `Wants=network-online.target` 会带进一批
/// 系统服务）。
pub fn service_dependencies(target: &str) -> Result<Vec<String>> {
    validate_unit_name(target)?;
    let out = system::run("systemctl", &["list-dependencies", target, "--no-pager"])?;
    // unit 不存在时多数版本仍以 0 退出并只打印自身名称；仅在完全无输出时报错。
    if !out.success() && out.stdout.trim().is_empty() {
        return Err(crate::error::Error::CommandFailed {
            program: "systemctl".to_string(),
            status: out.status_text(),
            stderr: out.stderr.trim().to_string(),
        });
    }
    Ok(parse_service_dependencies(&out.stdout))
}

/// 从 `systemctl list-dependencies` 的树形输出中取出第 1 层的服务单元。
///
/// 行首可能带一个状态标记（`●` / `○` / `×`），它占用一个单元格但不代表层级；当所有
/// 条目处于同一层时 systemd 会省略树形符号，此时按“每层 2 个空格”的缩进判断。
#[must_use]
pub fn parse_service_dependencies(text: &str) -> Vec<String> {
    let mut units = std::collections::BTreeSet::new();
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

/// 校验服务名是否合法。
pub fn validate_unit_name(unit: &str) -> Result<()> {
    let valid = !unit.is_empty()
        && unit.len() <= 256
        && unit
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | '\\' | ':'));
    if valid {
        Ok(())
    } else {
        Err(crate::error::Error::InvalidArgument(format!(
            "非法服务名：{unit}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_show_output() {
        let text = "LoadState=loaded\nActiveState=failed\nSubState=failed\n\
                    MainPID=0\nExecMainCode=1\nExecMainStatus=7\nResult=exit-code\nNRestarts=2\n";
        let status = parse_show_output(text);
        assert!(status.is_failed());
        assert_eq!(status.n_restarts, 2);

        let (code, signal, result) = status.exit_details();
        assert_eq!(code, Some(7));
        assert_eq!(signal, None);
        assert_eq!(result, "failed");
    }

    #[test]
    fn derives_signal_from_killed_code() {
        let text = "ActiveState=failed\nExecMainCode=2\nExecMainStatus=9\nResult=signal\n";
        let (code, signal, result) = parse_show_output(text).exit_details();
        assert_eq!(code, None);
        assert_eq!(signal, Some(9));
        assert_eq!(result, "failed");
    }

    #[test]
    fn clean_exit_is_not_a_failure() {
        let text = "ActiveState=inactive\nExecMainCode=1\nExecMainStatus=0\nResult=success\n";
        let (code, signal, result) = parse_show_output(text).exit_details();
        assert_eq!(code, Some(0));
        assert_eq!(signal, None);
        assert_eq!(result, "exited");
    }

    #[test]
    fn missing_exit_info_stays_unknown() {
        let text = "ActiveState=inactive\nResult=success\n";
        let (code, signal, result) = parse_show_output(text).exit_details();
        assert_eq!(code, Some(0));
        assert_eq!(signal, None);
        assert_eq!(result, "exited");

        let text = "ActiveState=inactive\nResult=\n";
        let (_, _, result) = parse_show_output(text).exit_details();
        assert_eq!(result, "unknown");
    }

    #[test]
    fn recognizes_oneshot_services() {
        let status = parse_show_output("Type=oneshot\n");
        assert!(status.is_oneshot());
        let status = parse_show_output("Type=simple\n");
        assert!(!status.is_oneshot());
    }

    #[test]
    fn detects_not_found_load_state() {
        let status = parse_show_output("LoadState=not-found\nActiveState=inactive\n");
        assert_eq!(status.load_state, "not-found");
        assert!(!status.is_active());
        assert!(!status.is_failed());
    }

    #[test]
    fn rejects_illegal_unit_names() {
        assert!(validate_unit_name("robot-lidar.service").is_ok());
        assert!(validate_unit_name("bad; rm -rf /").is_err());
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
    fn empty_or_target_only_output_yields_nothing() {
        assert!(parse_service_dependencies("").is_empty());
        assert!(parse_service_dependencies("robot-system.target\n").is_empty());
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

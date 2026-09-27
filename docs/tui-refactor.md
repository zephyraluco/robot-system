# TUI 重构记录（`refactor/tui`）

> 本分支是**长期重构/精简分支**：只做结构调整与代码删减，**不加新功能**。
> 目标是让 TUI 的模态、按键、渲染只有一条清晰路径，并持续减小体量。

## 1. 当前基线

| 指标 | main | 现在 |
|---|---|---|
| `app/keys.rs` | 1650 | 1154（不含任何模态分支） |
| `overlays.rs` | 592 | 264（只剩共享模态 chrome） |
| 删除的专用控件 | — | `effort_picker.rs`(1004)、`render_context_menu`(70)、`HelpOverlay`/`render_help_overlay`/`kb_line` |
| 新增 | — | `dialogs/modal_keys.rs`、`dialogs/help_dialog.rs` |
| tui 源码合计 | — | 40625 行 |

## 2. 已完成

### 2.1 键盘输入两层结构

`App::handle_key_event` 是唯一入口，只有两条下落路径：

```
normalize_layout_shortcut_key
  → dialogs::modal_keys::handle_modal_key   （模态层：有模态就全捕获）
      ├─ Consumed → 结束
      ├─ Submit   → 作为 prompt 提交（命令面板）
      └─ NoModal  ↓
  → App::handle_main_key                    （主界面层：输入框/滚动/快捷键）
```

- 所有 `DialogCore` 模态共用同一协议 `DialogBehavior::handle_key`（`drive()` 是唯一漏斗）。
- 唯一非 `DialogCore` 例外：权限弹窗，经 `dialogs::permission::handle_permission_key(app, key)` 接入（CLI 主循环还要用 `selected_key()` 回送 `decision_tx`）。

### 2.2 模态统一到 `DialogCore` / `DialogSelectState`

| 界面 | 之前 | 现在 |
|---|---|---|
| `/effort` | 停靠底部横向轨道 + ▲ 标记 + rainbow + 红色声波动画（1004 行专用控件） | `DialogSelectState` 列表模态（`App::open_effort_picker()`） |
| 帮助 `? / F1 / /help` | `overlays::HelpOverlay`（自绘双栏） | `dialogs::help_dialog::HelpDialogState`（双栏/过滤/滚动不变） |
| 右键菜单 | 自绘圆角菜单盒 + 坐标夹取 + 悬停高亮 | `DialogSelectState` 列表模态（`App::show_context_menu(kind)`），并获得搜索/翻页/点击选中 |

### 2.3 死代码清理

- 删除 4 处 `#[allow(dead_code)]` 及其内容：`prompt_is_accepting_text`（已被 `paste_burst_allowed` 取代）、`find_line_boundaries`、`get_env_var_for_provider`（与 `claurst_core::api_key_env_vars_for_provider` 重复）、`get_url_for_provider`（全仓库无调用者）。**tui 内已无任何 `allow(dead_code)` / `allow(unused*)`。**
- `dialog_select.rs::content_height()` 修正：高度必须含 3 行 header 与 2 行边框，否则列表末尾会被裁掉（categorized 列表此前一直少 2 行）。

## 3. 待办候选（按风险排序）

| # | 项目 | 风险 | 说明 |
|---|---|---|---|
| 1 | 权限弹窗单一路径 | 中 | CLI 主循环（`claurst.rs`）与 `handle_permission_key` 重复处理按键；后者不发送 `decision_tx`，当前不可达但是隐患。收敛进 App 后 CLI 只负责把它拿到/放回 pending 队列。 |
| 2 | CLI ↔ TUI 边界收敛 | 中高 | `claurst.rs`(4875) 里的 TUI 策略可下沉进 App：paste-burst 门控、`any_modal_open()` 的 Enter/submit 判定、`permission_request_from_core`、`handle_exit_key`、对话框粘贴路由、agent-mode 切换。 |
| 3 | 文件拆分 | 中 | `render.rs`(3284)、`dialogs/permission.rs`(2064)、`app/mouse.rs`、`messages/mod.rs`(2734)、`lib.rs`(1130，其中大半是 `mod tests`)。 |
| 4 | 解耦 `commands → tui` | 中 | `crates/commands` 仅为 `HelpEntry` + `input::{is_slash_command, parse_slash_command}` 依赖 `claurst-tui`（分层倒置）。把这两个纯类型/函数下沉到 `claurst-core` 即可断开依赖。 |
| 5 | `KeyContext` 裁剪 | 中高 | 18 个变体，两层分发后 `Settings`/`Tabs`/`Footer`/`ModelPicker` 等可能已无 TUI 侧用处；但会影响 `keybindings.json` 的用户可见结构，须先核对 `core/src/keybindings.rs` 绑定表。 |
| 6 | `crates/commands::build_help_entries()` | 低（待确认设计） | 全仓库无调用者；App 用的是 `app/commands.rs::help_dialog_entries()`（基于静态 `PROMPT_SLASH_COMMANDS`）。**要么删掉它，要么反过来用它**（注册表驱动，帮助列表才与实际命令一致）——这是设计选择，需先确认。 |
| 7 | `app/mod.rs` 字段分区 | 中 | `App` 仍是"上帝对象"，可按域拆成子结构（dialogs / session / ui）。 |
| 8 | `overlays.rs` 命名 | 低 | 现在只剩共享 chrome，可考虑改名 `modal_chrome.rs` 或并入 `dialogs/`。 |

## 4. 工作约定

- 每个增量必须过：`cargo clippy -p claurst-tui --all-targets -- -D warnings`、`cargo test -p claurst-tui --all-targets`、`cargo check --workspace --all-targets`。
- 新增/修改模态的固定四步见 `docs/crates/tui.md` §8.1 与 `every_registered_modal_renders_and_captures_input` 测试（`app/views.rs::modal_states()` 是可见性清单的单一事实源）。
- 提交粒度：一次提交只做一件事，前缀用 `refactor(tui):`。

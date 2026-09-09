# `claurst-tui` — 终端 UI

> 路径：`crates/tui` ｜ 约 **45,620 行**（workspace 最大 crate）｜ 依赖：`claurst-core`、`claurst-api`、`claurst-tools`、`claurst-query`、`claurst-mcp`

## 1. 定位

ratatui + crossterm 的交互式终端界面——消息渲染（语法高亮）、流式输出、工具进度、权限对话框、成本/token 显示、通知横幅、各类 overlay、bridge 状态、插件提示等。

## 2. 依赖

- **内部**：core、api、tools、query、mcp（依赖最全，位于依赖图顶端之一；仅被 `cli` 和 `commands` 引用）。
- **关键外部**：`ratatui` + `crossterm`、`syntect`（语法高亮）、`similar`（diff）、`icy_sixel` + `image`（Sixel/Kitty 图像协议）、`unicode-width`。
- **features**：约 40 个**透传 feature**（pass-through 到 `claurst-core`）。

## 3. 主入口与 run loop

- **`lib.rs`**（1495 行）：终端初始化/拆除——`setup_terminal(mouse_capture)`（raw mode、alternate screen、bracketed paste、kitty keyboard enhancement 协议检测、mouse capture 可关、panic hook 只在主线程恢复终端）、`restore_terminal()`、OSC 9;4 进度指示、终端标题管理。全部子模块声明与 re-export 也在这里。
- **`app/run.rs` → `App::run(&mut terminal)`**：**TUI 主事件循环**，返回 `Option<String>`。每帧：drain 后台 session 列表 → `terminal.draw(render::render_app)` → OSC 8 超链接扫描重发（URL 可 Ctrl/Cmd 点击）→ 50ms 轮询 crossterm 事件 → 粘贴爆发检测 → `handle_key_event` → 提交/退出。token 警告横幅与 `CostTracker` 同步也在此。
- **集成点**：`crates/cli/src/bin/claurst.rs` 中 `App::new(...)` + `app.run(...)` + 多处 `claurst_query::run_query_loop(...)`——CLI 主程序把 TUI 输入喂给查询循环，循环经 `QueryEvent` channel 回流渲染。

## 4. `App` 结构与 app/ 子模块

`App` 是整个 TUI 应用的顶层状态容器（来源：`crates/tui/src/app/mod.rs`）：约 **170 个字段**、
2 个私有字段、若干常量/工具函数以及少量核心方法。字段按功能域划分为十余个区块，
由同目录子模块（`commands`、`keys`、`messages`、`mouse`、`prompt`、`providers`、
`run`、`turns`、`views` 等）分别操作。逐字段解析见 4.2 起。

| 文件 | 行数 | 职责 |
|---|---|---|
| `app/mod.rs` | 906 | App 结构体、状态字段、子模块组织；`try_copy_to_clipboard`（跨平台） |
| `app/run.rs` | 561 | **主事件循环 `run()`** |
| `app/keys.rs` | 2703 | 键盘事件处理：快捷键、kitty 协议 shift 归一化（#183）、vim 命令行 `:q`/`:wq` |
| `app/mouse.rs` | 658 | 鼠标事件：滚动、右键菜单、拖选 |
| `app/commands.rs` | 356 | `PROMPT_SLASH_COMMANDS` 斜杠命令表与分发、help overlay 条目 |
| `app/prompt.rs` | — | 输入提示相关助手 |
| `app/providers.rs` | 441 | provider/model 选择器条目构建 |
| `app/messages.rs` | — | 消息列表操作助手 |
| `app/turns.rs` | 176 | 回合状态转换助手 |
| `app/views.rs` | 404 | 各视图状态切换助手 |
| `app/types.rs` | 245 | `DisplayMessage`、`SystemAnnotation`、`ToolUseBlock`、`TurnMetadata`、`FocusTarget` 等类型 |
| `app/tests.rs` | 960 | App 层测试 |

从 `types` 公开再导出的类型：
`ContextMenuKind`、`DisplayMessage`、`FocusTarget`、`HistorySearch`、
`RecentSession`、`SystemAnnotation`、`SystemMessageStyle`、`ToolStatus`、
`ToolUseBlock`、`TurnMetadata`、`recent_session_label`。

### 4.1 常量与 crate 级工具函数

```rust
pub const ACCENT_BUILD: Color = Color::Rgb(233, 30, 99); // Build 模式（粉）
pub const ACCENT_PLAN:  Color = Color::Rgb(66, 135, 245); // Plan 模式（蓝）
pub fn accent_for_mode(mode: Option<&str>) -> Color;     // 模式 → 强调色
pub fn try_copy_to_clipboard(text: &str) -> bool;        // crate 级 API（lib.rs 再导出）
```

`try_copy_to_clipboard` 按平台依次尝试：
- **Windows**: `clip`
- **macOS**: `pbcopy`
- **Linux**: `wl-copy` → `xclip -selection clipboard` → `xsel --clipboard --input`

### 4.2 核心状态

| 字段 | 类型 | 说明 |
|---|---|---|
| `config` | `Config` | 全局配置（模型、主题、provider 等） |
| `cost_tracker` | `Arc<CostTracker>` | 成本/Token 统计追踪器（跨线程共享） |
| `messages` | `Vec<Message>` | 真实对话消息列表 |
| `display_messages` | `Vec<DisplayMessage>` | 与 `messages` 同步的展示列表（含注入的系统标注），渲染器只需遍历一个序列 |
| `system_annotations` | `Vec<SystemAnnotation>` | 渲染时穿插在真实消息之间的合成系统标注 |
| `input` | `String` | 输入框当前文本 |
| `prompt_input` | `PromptInputState` | 提示输入组件状态 |
| `input_history` | `Vec<String>` | 输入历史（↑/↓ 回溯） |
| `history_index` | `Option<usize>` | 当前回溯到的历史条目下标 |
| `scroll_offset` | `usize` | 消息窗格滚动偏移 |
| `is_streaming` | `bool` | 是否正在流式接收回复 |
| `streaming_text` | `String` | 流式接收中的正文文本 |
| `streaming_thinking` | `String` | 流式接收中的思考 (thinking) 文本 |
| `status_message` | `Option<String>` | 状态栏临时消息 |
| `spinner_verb` | `Option<String>` | 流式时 spinner 旁随机显示的动词 |
| `should_exit` | `bool` | 退出标志 |
| `show_help` | `bool` | 是否显示帮助 |
| `kitty_keyboard_active` | `bool` | 终端是否支持 kitty 键盘协议；决定是否对可打印键重新应用 Shift 映射（issue #183） |

### 4.3 扩展状态（模型 / 凭证 / 模式）

| 字段 | 类型 | 说明 |
|---|---|---|
| `tool_use_blocks` | `Vec<ToolUseBlock>` | 工具调用块的聚合状态 |
| `permission_request` | `Option<PermissionRequest>` | 待处理的工具权限请求对话框 |
| `frame_count` | `u64` | 帧计数器（动画/定时用） |
| `token_count` | `u32` | 当前 token 用量 |
| `token_budget` | `Option<u32>` | 最大 token 预算（P2 特性开关，`load_token_budget()` 读取 `CLAURST_TOKEN_BUDGET`） |
| `cost_usd` | `f64` | 累计花费（美元） |
| `model_name` | `String` | 当前模型名 |
| `has_credentials` | `bool` | 是否有有效 API 凭证；false 时启动显示 provider 配置对话框 |
| `effort_level` | `EffortLevel` | 扩展思考预算等级 |
| `fast_mode` | `bool` | 快速模式（锁定 FAST_MODE_MODEL） |
| `agent_mode` | `Option<String>` | 当前代理模式："build" / "plan" |
| `accent_color` | `Color` | 由代理模式派生的强调色（Build=粉，Plan=蓝） |
| `agent_mode_changed` | `bool` | `cycle_agent_mode` 设置，主循环据此更新查询配置和工具列表 |
| `agent_status` | `Vec<(String, String)>` | 子代理状态列表 |
| `history_search` | `Option<HistorySearch>` | Ctrl+R 历史搜索状态 |
| `keybindings` | `KeybindingResolver` | 键位解析器（加载用户自定义键位） |
| `cursor_pos` | `usize` | 输入框内光标位置（字节偏移） |

### 4.4 滚动 / Token 警告

| 字段 | 类型 | 说明 |
|---|---|---|
| `auto_scroll` | `bool` | 消息窗格是否自动跟随最新消息 |
| `new_messages_while_scrolled` | `usize` | 用户上翻期间到达的新消息数 |
| `token_warning_threshold_shown` | `u8` | 已通知的 token 阈值（0/80/95/100），保证每档横幅只提示一次 |

### 4.5 会话计时 / Rustle 吉祥物动画

| 字段 | 类型 | 说明 |
|---|---|---|
| `session_start` | `Instant` | 会话开始时间（状态栏显示已用时长） |
| `rustle_current_pose` | `RustlePose` | 当前帧 Rustle（吉祥物）姿势 |
| `rustle_pose_until` | `Option<Instant>` | 临时姿势的截止时刻 |
| `rustle_temp_pose` | `Option<RustlePose>` | 临时姿势（如 Tab 触发的 look-down） |
| `rustle_next_blink` | `u64` | 下一次随机眨眼/眼神偏移的帧号 |
| `turn_start` | `Option<Instant>` | 本回合流式开始时间 |
| `last_turn_elapsed` | `Option<String>` | 上一回合耗时字符串，如 "2m 5s" |
| `last_turn_verb` | `Option<&'static str>` | 回合完成后的过去式动词，如 "Worked" / "Baked" |
| `turn_metadata` | `Vec<TurnMetadata>` | 转录渲染器使用的逐回合快照 |
| `transcript_version` | `Cell<u64>` | 转录可见状态变更计数，用于跨按键复用布局缓存 |

相关方法：`tick_rustle_pose()`（每帧更新姿势：卡顿 3s 后显示 Loading spinner、随机 look-right、临时姿势到期恢复默认）、`rustle_look_down()`。

### 4.6 覆盖层 / 通知 / 桥接

| 字段 | 类型 | 说明 |
|---|---|---|
| `help_overlay` | `HelpOverlay` | 全屏帮助覆盖层（? / F1），启动时从 `help_overlay_entries()` 填充 |
| `history_search_overlay` | `HistorySearchOverlay` | Ctrl+R 历史搜索覆盖层 |
| `global_search` | `GlobalSearchState` | 全局 ripgrep 搜索 / 快速打开覆盖层 |
| `message_selector` | `MessageSelectorOverlay` | `/rewind` 使用的消息选择器 |
| `rewind_flow` | `RewindFlowOverlay` | 多步回退流程覆盖层 |
| `bridge_state` | `BridgeConnectionState` | Bridge（远程会话）连接状态 |
| `notifications` | `NotificationQueue` | 通知队列 |
| `error_modal_scroll_offset` | `usize` | 错误弹窗文本滚动偏移 |
| `plugin_hints` | `Vec<PluginHintBanner>` | 插件提示横幅 |
| `session_title` | `Option<String>` | 状态栏显示的会话标题 |
| `remote_session_url` | `Option<String>` | Bridge 连接后的远程会话 URL（命令可读） |
| `mcp_manager` | `Option<Arc<McpManager>>` | 实时 MCP 管理器快照源 |
| `pending_mcp_reconnect` | `bool` | 请求交互循环执行真实 MCP 重连 |
| `pending_provider_reload` | `bool` | 会话内 provider 连接（如 OAuth 登录）后，主循环需重建 client + provider registry |
| `pending_mcp_panel_auth` | `Option<String>` | 待处理的 MCP 面板认证请求 |
| `file_history` | `Option<Arc<Mutex<FileHistory>>>` | 共享文件历史服务（回合 diff 重建） |
| `current_turn` | `Option<Arc<AtomicUsize>>` | 查询循环共享的回合计数器 |

### 4.7 视觉模式指示

| 字段 | 类型 | 说明 |
|---|---|---|
| `plan_mode` | `bool` | Plan 模式（输入框边框变蓝，状态栏显示 [PLAN]），由 `cycle_agent_mode` 同步 |
| `away_summary` | `Option<String>` | 欢迎屏 "While you were away" 摘要 |
| `stall_start` | `Option<Instant>` | 流式卡顿起点（3s 后 spinner 变红） |

### 4.8 对话框、屏幕与后台加载（约 66 个字段）

#### 全屏屏幕 / 主对话框

| 字段 | 类型 | 说明 |
|---|---|---|
| `settings_screen` | `SettingsScreen` | 全屏标签式设置屏（/config、/settings） |
| `theme_screen` | `ThemeScreen` | 主题选择器（/theme），配套 `apply_theme()` 方法 |
| `stats_dialog` | `StatsDialogState` | Token/费用分析对话框 |
| `mcp_view` | `McpViewState` | MCP 服务器浏览与工具详情 |
| `agents_menu` | `AgentsMenuState` | 代理定义与活跃代理状态覆盖层 |
| `diff_viewer` | `DiffViewerState` | Diff 查看器覆盖层 |
| `paste_viewer` | `PasteViewer` | `[Pasted text #N ...]` 占位符只读查看器 |
| `feedback_survey` | `FeedbackSurveyState` | 会话质量反馈问卷覆盖层 |
| `memory_file_selector` | `MemoryFileSelectorState` | 内存文件选择器（AGENTS.md 浏览） |
| `hooks_config_menu` | `HooksConfigMenuState` | Hooks 配置只读浏览器 |
| `overage_upsell` | `OverageCreditUpsellState` | 超额积分升级横幅 |
| `desktop_upsell` | `DesktopUpsellStartupState` | 桌面应用升级启动对话框 |
| `invalid_config_dialog` | `InvalidConfigDialogState` | settings.json / AGENTS.md 损坏时的启动错误对话框 |
| `memory_update_notification` | `MemoryUpdateNotificationState` | 内存更新通知横幅 |
| `elicitation` | `ElicitationDialogState` | MCP elicitation 表单对话框 |
| `model_picker` | `ModelPickerState` | 模型选择器（/model） |
| `session_browser` | `SessionBrowserState` | 会话浏览器（/session、/resume、/rename、/export） |
| `session_branching` | `SessionBranchingState` | 会话分支覆盖层（Ctrl+B） |
| `tasks_overlay` | `TasksOverlay` | 任务进度覆盖层（Ctrl+T，可切换状态） |
| `export_dialog` | `ExportDialogState` | 导出格式选择（/export） |
| `context_viz` | `ContextVizState` | 上下文窗口 / 速率限制可视化（/context） |
| `go_to_line_dialog` | `GoToLineDialog` | 跳转行号对话框（Ctrl+G） |

#### MCP 审批（项目级信任）

| 字段 | 类型 | 说明 |
|---|---|---|
| `mcp_approval` | `McpApprovalDialogState` | MCP 服务器审批对话框 |
| `mcp_pending_project` | `VecDeque<McpServerConfig>` | 待审批的项目级 MCP 服务器队列（一次弹一个） |
| `mcp_prompting` | `Option<McpServerConfig>` | 当前正在审批的项目 MCP 服务器 |
| `mcp_session_trusted` | `HashSet<String>` | 本次会话已批准的项目 MCP 指纹（"仅本次会话允许"，不落盘） |
| `mcp_project_root` | `Option<PathBuf>` | 用于持久化 MCP 信任审批的项目根路径 |

#### 权限 / 安全确认对话框

| 字段 | 类型 | 说明 |
|---|---|---|
| `bypass_permissions_dialog` | `BypassPermissionsDialogState` | `--dangerously-skip-permissions` 启动确认，必须显式接受否则退出 |
| `bypass_permissions_dialog_shown` | `bool` | 本次会话是否已展示过该确认框 |
| `file_injection_dialog` | `FileInjectionDialogState` | @refs 中超大/二进制文件注入警告 |
| `file_injection_force` | `bool` | 为 true 时下一次大小检查限额为 0（放行已确认的文件） |
| `bash_prefix_allowlist` | `HashSet<String>` | 本会话永久放行的 bash 命令前缀（权限对话框"允许以 X 开头的命令"） |

#### 引导与 Provider 配置对话框族

| 字段 | 类型 | 说明 |
|---|---|---|
| `onboarding_dialog` | `OnboardingDialogState` | 首次启动欢迎向导 |
| `effort_picker` | `EffortPickerState` | 思考力度选择器（/effort 无参数时） |
| `key_input_dialog` | `KeyInputDialogState` | API Key 输入框（从 /connect 打开） |
| `custom_provider_dialog` | `CustomProviderDialogState` | 自定义 provider（URL + Key）对话框 |
| `free_mode_dialog` | `FreeModeDialogState` | "Free" 复合 provider 设置（警告 + 2 个 API Key） |
| `device_auth_dialog` | `DeviceAuthDialogState` | 设备码/浏览器认证（GitHub Copilot 设备流、Anthropic OAuth） |
| `device_auth_pending` | `Option<String>` | 非空时主循环为该 provider 派生异步认证任务 |
| `connect_dialog` | `DialogSelectState` | 连接 provider 选择（/connect） |
| `import_config_picker` | `DialogSelectState` | 导入配置来源选择（/import-config） |
| `import_config_dialog` | `ImportConfigDialogState` | 导入配置预览与确认 |
| `command_palette` | `DialogSelectState` | Ctrl+K 命令面板（由 `PROMPT_SLASH_COMMANDS` 构建） |

#### 注册表 / 凭证 / 后台异步加载

| 字段 | 类型 | 说明 |
|---|---|---|
| `provider_registry` | `Option<Arc<ProviderRegistry>>` | 动态模型获取的共享 provider 注册表 |
| `model_registry` | `ModelRegistry` | models.dev 模型注册表（`/model` 选择器的唯一数据源；启动时加载磁盘缓存并应用用户覆盖，issue #309） |
| `model_picker_fetch_pending` | `bool` | 主循环应派生异步任务拉取当前 provider 的模型列表 |
| `model_picker_provider_id` | `Option<String>` | 模型选择器针对的 provider ID（/connect 场景） |
| `model_fetch_rx` | `Option<mpsc::Receiver<...>>` | /model 后台模型列表结果接收端，每帧 drain |
| `session_list_pending` | `bool` | 应派生异步任务从磁盘加载会话列表 |
| `session_list_rx` | `Option<mpsc::Receiver<Vec<SessionEntry>>>` | 后台会话列表结果接收端 |
| `recent_sessions` | `Vec<RecentSession>` | 欢迎屏 "Recent activity" 列表数据 |
| `recent_sessions_pending` | `bool` | 应派生一次性任务加载最近会话（启动置位，取件后清除，避免每帧重复列出） |
| `recent_sessions_rx` | `Option<mpsc::Receiver<Vec<RecentSession>>>` | 最近会话后台加载接收端 |
| `auth_store` | `AuthStore` | provider API Key 与 OAuth Token 凭证存储 |
| `user_question_rx` | `Option<UnboundedReceiver<UserQuestionEvent>>` | AskUserQuestion 工具产生的提问事件接收端 |
| `ask_user_dialog` | `AskUserDialogState` | 模型发起的 ask-user 提问对话框状态 |

#### 消息队列 / 杂项运行时状态

| 字段 | 类型 | 说明 |
|---|---|---|
| `queued_messages` | `VecDeque<String>` | 流式期间用户键入的消息，回合结束后按序自动提交（issue #149） |
| `pending_auto_submit` | `bool` | 主循环下次迭代注入合成 Enter 事件以出队下一条消息 |
| `pending_key` | `Option<KeyEvent>` | 粘贴突发检测时被排除的单个按键，下次循环顶部重放 |
| `home_dir_warning` | `bool` | 是否从家目录启动（启动提示） |
| `output_style` | `String` | 输出样式："auto" / "stream" / "verbose" |
| `pr_number` / `pr_url` / `pr_state` | `Option<...>` | 当前分支的 PR 号 / URL / 审查状态 |
| `current_dir` / `git_branch` | `Option<String>` | 当前工作目录 / git 分支（构造时从环境读取） |
| `background_task_count` / `background_task_status` | `usize` / `Option<String>` | 进行中后台任务数与文本（驱动底部胶囊） |
| `status_line_override` | `Option<String>` | 外部状态行命令输出（`CLAUDE_STATUS_COMMAND`） |
| `auto_compact_enabled` / `auto_compact_threshold` / `auto_compact_running` | `bool` / `u8` / `bool` | 自动压缩开关、触发阈值(0-100)、防重入标志 |

### 4.9 上下文窗口与速率限制

| 字段 | 类型 | 说明 |
|---|---|---|
| `context_window_size` | `u64` | 当前模型上下文窗口大小（tokens） |
| `context_used_tokens` | `u64` | 当前已用 tokens |
| `rate_limit_5h_pct` | `Option<f32>` | 5 小时窗口用量百分比（0–100） |
| `rate_limit_7day_pct` | `Option<f32>` | 7 天窗口用量百分比（0–100） |
| `worktree_name` / `worktree_branch` | `Option<String>` | 活动工作树（worktree）名称与分支 |
| `agent_type_badge` | `Option<String>` | 代理类型徽标："agent" / "coordinator" / "subagent" |
| `active_goal_badge` | `Option<String>` | 目标徽标（"active · 5m · 3 turns"），每回合后由 REPL 更新 |

配套方法：`refresh_context_window_size()` —— 优先从 `model_registry` 查询当前模型的上下文窗口，查不到则按 provider 回退到默认值（anthropic 200k、openai 128k、google 1M、其他 128k）。

### 4.10 思考块展开与渲染缓存（跨帧命中测试）

| 字段 | 类型 | 说明 |
|---|---|---|
| `thinking_expanded` | `HashSet<u64>` | 已展开的思考块内容哈希集合 |
| `last_msg_area` | `Cell<Rect>` | 上一帧消息窗格区域（鼠标命中测试） |
| `last_selectable_area` | `Cell<Rect>` | 支持文本选择的帧区域 |
| `last_input_area` | `Cell<Rect>` | 上一帧提示输入区域（焦点路由） |
| `footer_right_column_area` | `Cell<Rect>` | 底栏右列区域（提示文字） |
| `focus` | `FocusTarget` | 当前持有键盘焦点的 TUI 区域 |
| `thinking_row_map` | `RefCell<HashMap<u16, u64>>` | 虚拟行号 → 思考块哈希（点击检测） |
| `message_row_map` | `RefCell<HashMap<u16, usize>>` | 屏幕行 → 转录消息下标（右键命中测试） |
| `total_message_lines` | `Cell<usize>` | 上一帧总消息行数（虚拟行映射） |
| `last_render_scroll_offset` | `Cell<u16>` | 上一帧滚动偏移（选择校验） |
| `last_max_scroll` | `Cell<usize>` | 上一帧最大滚动偏移；渲染器写入（唯一知道内容总高的地方），滚动事件读取以钳制 `scroll_offset`（#223） |

### 4.11 文本选择

| 字段 | 类型 | 说明 |
|---|---|---|
| `selection_anchor` | `Option<(u16, u16)>` | 选择拖拽锚点 (列, 行)，鼠标按下时设置 |
| `selection_focus` | `Option<(u16, u16)>` | 拖拽焦点点，拖动/松开时更新 |
| `selection_text` | `RefCell<String>` | 当前选择提取出的文本（每帧更新） |
| `last_row_text` | `RefCell<HashMap<u16, String>>` | 可选区域内 行 → 渲染文本 缓存，供双击选词/三击选段使用 |

### 4.12 高级鼠标交互与滚动加速

| 字段 | 类型 | 可见性 | 说明 |
|---|---|---|---|
| `last_click_time` | `Option<Instant>` | pub | 上次左键点击时间（双/三击检测） |
| `last_click_position` | `Option<(u16, u16)>` | pub | 上次左键点击位置 |
| `click_count` | `u32` | pub | 连续点击计数：1=单击、2=双击、3+=三击 |
| `context_menu_state` | `Option<ContextMenuState>` | pub | 上下文菜单状态（位置 + 选中项） |
| `scroll_accel` | `f32` | **私有** | 滚动事件加速度倍率（触控板手感） |
| `scroll_last_time` | `Option<Instant>` | **私有** | 上次滚动事件时间（突发检测） |

### 4.13 退出确认 / 托管代理 / 更新

| 字段 | 类型 | 说明 |
|---|---|---|
| `update_available` | `Option<String>` | 后台更新检查发现的新版本号，显示在底栏状态条 |
| `managed_agent_cost_breakdown` | `Option<(f64, f64, f64)>` | 托管代理会话费用分解：(manager, executors, total) |
| `managed_agents_active` | `bool` | 托管代理模式是否激活 |
| `last_exit_key_warning` | `Option<Instant>` | 首次按退出键显示确认的时间（约 2 秒有效） |
| `exit_key_sequence_start` | `Option<char>` | 当前确认序列由哪个退出键 ('c' / 'd') 触发 |

### 4.14 `App` 的方法一览

| 方法 | 作用 |
|---|---|
| `new(config, cost_tracker)` | 构造函数：初始化全部字段；预构建 `ModelRegistry`（加载缓存 + 应用用户模型覆盖）；加载用户键位；从环境推断 `current_dir` 与 `git_branch` |
| `load_token_budget()` | 私有辅助。有 `token_budget` 特性时读取 `CLAURST_TOKEN_BUDGET` 环境变量，否则返回 `None` |
| `tick_rustle_pose()` | 每帧调用：流式卡顿 3s+ → Loading spinner；临时姿势生效/到期；按 200–500 帧间隔随机 look-right |
| `rustle_look_down()` | 触发 Rustle 低头 1 秒（Tab / 模式切换时调用） |
| `cycle_agent_mode()` | build ↔ plan 轮换；更新 `agent_mode`、`accent_color`、`plan_mode` 并置 `agent_mode_changed` |
| `refresh_context_window_size()` | 从 `model_registry` 刷新当前模型上下文窗口大小，带 provider 级默认值回退 |
| `apply_theme(theme_name)` | 按名称应用主题（dark/light/default/deuteranopia/自定义），写入 config 并持久化到 settings 文件 |

### 4.15 设计要点总结

1. **单一大状态对象**：`App` 采用"上帝对象"模式集中全部 UI 状态，子模块（keys/messages/mouse/views…）通过 `impl App` 分文件实现行为，避免单文件膨胀。
2. **渲染缓存回写**：渲染器把区域矩形、最大滚动量等写回 `Cell` 字段，下一帧的鼠标/键盘处理据此做命中测试与钳制 —— 形成渲染与输入的帧间契约。
3. **pending 标志 + channel** 的异步协作模式：UI 主循环不做阻塞 IO，通过 `*_pending: bool` 请求主循环派生 tokio 任务，用 `*_rx: mpsc::Receiver` 每帧 drain 结果（模型列表、会话列表、最近会话、用户提问等）。
4. **双列表同步**：`messages`（真实对话）与 `display_messages`（含系统标注的展示列表）保持同步，渲染器只需遍历后者。
5. **会话级信任不落盘**：`mcp_session_trusted`、`bash_prefix_allowlist` 等仅存活于本次会话，与持久化审批（`mcp_project_root`）严格区分。

## 5. 渲染层

| 文件 | 行数 | 职责 |
|---|---|---|
| `render.rs` | 3894 | **全部 ratatui 渲染逻辑**，`render_app` 总入口，按 App 状态分派面板/overlay 绘制 |
| `messages/mod.rs` | 2675 | 各消息类型渲染器，流式渲染 |
| `messages/markdown.rs` | 339 | Markdown 基础渲染 |
| `messages/markdown_enhanced.rs` | 389 | 增强 Markdown（表格/代码块等） |
| `virtual_list.rs` | 421 | 消息高效渲染的虚拟滚动列表 |
| `transcript_turn.rs` | 175 | 回合感知的 transcript 分组与元数据 |
| `prompt_input.rs` | **5084**（最大文件） | 完整提示输入组件：vim 模式、历史、typeahead、粘贴处理、渲染 |
| `rustle.rs` | 260 | Rustle 吉祥物（🦀）渲染 |
| `figures.rs` | 29 | 图标/符号常量 |
| `theme_colors.rs` | 212 | 主题调色板与无障碍支持 |
| `theme_screen.rs` | 332 | 主题选择 overlay |
| `osc8.rs` | 325 | 渲染后 OSC 8 超链接叠加 |
| `kitty_image.rs` | 424 | Kitty 图形协议内联图片渲染（含文本回退） |
| `image_paste.rs` | 508 | 剪贴板图片粘贴 + Ctrl+V |

## 6. 对话框 / 覆盖层（按功能分组）

> 所有对话框组件位于 `dialogs/` 目录，统一基于 `dialogs/dialog.rs` 的 `DialogCore` + `DialogBehavior` 基座；`dialogs/mod.rs` 声明全部子模块并 re-export 权限对话框 API。

- **权限/确认**：`dialogs/permission.rs`(1728，权限/确认对话框 + MCP 审批)、`dialogs/bypass_permissions_dialog.rs`（--dangerously-skip-permissions 启动确认）
- **提问/表单**：`dialogs/ask_user_dialog.rs`（AskUserQuestion 弹窗）、`dialogs/elicitation_dialog.rs`(797，MCP elicitation 表单)
- **模型/effort**：`dialogs/model_picker.rs`(1585)、`effort_picker.rs`(1004)
- **会话**：`dialogs/session_browser.rs`(605)、`dialogs/session_branching.rs`（Ctrl+B 分支）、`dialogs/export_dialog.rs`、`dialogs/memory_file_selector.rs`
- **代码视图**：`dialogs/diff_viewer.rs`(1305，两栏 diff)、`paste_viewer.rs`
- **系统状态**：`dialogs/stats_dialog.rs`(914)、`context_viz.rs`、`mcp_view.rs`(676)、`tasks_overlay.rs`、`agents_view.rs`(959)、`hooks_config_menu.rs`
- **设置/引导**：`settings_screen.rs`(892)、`dialogs/onboarding_dialog.rs`、`dialogs/invalid_config_dialog.rs`、`dialogs/import_config_dialog.rs`
- **认证/账户**：`dialogs/device_auth_dialog.rs`（设备码 OAuth）、`dialogs/key_input_dialog.rs`、`dialogs/custom_provider_dialog.rs`、`dialogs/free_mode_dialog.rs`
- **通用控件**：`dialogs/dialog_select.rs`(621，可复用模糊搜索选择列表)、`dialogs/dialog.rs`（基座）
- **通知/横幅**：`notifications.rs`、`dialogs/feedback_survey.rs`、`overage_upsell.rs`、`dialogs/desktop_upsell_startup.rs`、`memory_update_notification.rs`
- **输入辅助**：`file_injection.rs`（@file 引用解析）、`dialogs/file_injection_dialog.rs`、`message_copy.rs`(480，多格式复制)
- **连接**：`bridge_state.rs`、`plugin_views.rs`
- **overlays.rs**(2246)：帮助 overlay、历史搜索、消息选择器、rewind 流程、全局搜索
- `input.rs`：斜杠命令解析（`is_slash_command`/`parse_slash_command`）

## 7. tests/

`diff_viewer.rs`、`markdown_enhancements.rs`、`render_snapshots.rs`（ratatui TestBackend 渲染快照）、`app/tests.rs`。

## 8. 设计要点

1. **主循环归属**：`App::run` 只负责输入/渲染；真正的"用户回合"驱动（调用 `run_query_loop`、goal continuation）在 `crates/cli` 的 REPL 中完成。
2. **单一大状态对象**：`App` 集中全部 UI 状态，行为按子模块（keys/mouse/views…）分文件 `impl App`。
3. **渲染缓存回写**：渲染器把区域矩形、最大滚动量等写回 `Cell` 字段，下一帧输入处理据此做命中测试与钳制。
4. **pending 标志 + channel** 的异步协作：UI 主循环不做阻塞 IO，用 `*_pending` 请求派生 tokio 任务、用 `*_rx` 每帧 drain 结果。
5. **commands ↔ tui 单向依赖**：commands 调 tui 的纯函数（slash 解析、HelpEntry），tui 不调 commands。

# `claurst-commands` — 斜杠命令系统

> 路径：`crates/commands` ｜ 约 **13,148 行 / 37 个源文件** ｜ 依赖：core、api、tools、query、mcp、tui、plugins、bridge（除 cli 二进制外依赖最全）｜ 仅被 `claurst`（cli）依赖

## 1. 定位

Claude Code 风格的 **slash 命令系统**（`/help`、`/compact`、`/model`、`/config`、`/cost` 等 **102 个**注册命令）+ **顶层 named command 框架**（`claurst agents` 等），以 `SlashCommand` trait 为核心，被 CLI 交互循环和 named-command fast-path 调用。

## 2. 依赖

- **features**：`ultraplan = ["claurst-core/ultraplan"]`（透传）。
- **关键外部**：`tokio`、`serde_json`、`anyhow`、`async-trait`、`arboard`（剪贴板 /copy）、`qrcode`（/share 二维码）、`tokio-tungstenite`（remote WebSocket）、`reqwest`。

## 3. 命令注册与执行机制（核心公开 API）

### `SlashCommand` trait 与上下文

```rust
pub struct CommandContext {
    pub config: Config,
    pub cost_tracker: Arc<CostTracker>,
    pub messages: Vec<Message>,
    pub working_dir: PathBuf,
    pub session_id: String,
    pub session_title: Option<String>,
    pub remote_session_url: Option<String>,
    pub mcp_manager: Option<Arc<McpManager>>,
    pub mcp_auth_runner: Option<Arc<dyn Fn(McpAuthSession) + Send + Sync>>,
}

pub enum CommandResult {
    Message(String),              // 仅显示，不进模型上下文
    UserMessage(String),          // 作为用户消息注入对话（/compact、模板命令）
    ConfigChange(Config),
    McpAuthFlow { .. },
    ClearConversation, SetMessages(..),
    ResumeSession(..), RenameSession(..),
    StartOAuthFlow(..), StartLoginForProvider { .. },
    Exit, Silent, Error(..),
    OpenRewindOverlay, OpenHooksOverlay, OpenImportConfigOverlay,
    RefreshProviderState, NewSession,
    MoveSession { .. },
}

#[async_trait]
pub trait SlashCommand: Send + Sync {
    fn name(&self) -> &str;
    fn aliases(&self) -> Vec<&str> { vec![] }
    fn description(&self) -> &str;
    fn hidden(&self) -> bool { false }
    async fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult;
}
```

### 注册机制

每个命令是一个零字段 struct 实现 `SlashCommand`；**`all_commands() -> Vec<Box<dyn SlashCommand>>` 是唯一静态注册表**（已验证 **102 个** `Box::new` 注册）。配套：

- `find_command(name)`：按名字或别名查找（自动剥 `/` 前缀）。
- **`execute_command(input, ctx)`（总入口）** 按顺序解析：内置命令 → `settings.commands` 用户自定义模板（支持 `$ARGUMENTS`/`$1`/`$2` 占位）→ `.claurst/skills/` 及 git URL 发现的技能 → 插件命令（`PluginSlashCommandAdapter`）。返回 `None` 表示不是 slash 命令。
- `build_help_entries()`：为 TUI HelpOverlay 生成条目（归类：Conversation/Settings/Usage & Cost/System/Auth & Permissions/Project/Integrations/Sessions & Remote/AI & Thinking/Tools & Extras/General）。

### named command 框架（`named_commands.rs`）

`NamedCommand` trait + `all_named_commands()`/`find_named_command()` 注册表；`NamedCommandAdapter` 把 named command 适配回 slash 命令。`StatsCommand` 一个 struct 同时实现两个 trait（slash 版看当前会话，named 版走 `stats.rs` 聚合磁盘 JSONL）。

**调用方**：`crates/cli/src/bin/claurst.rs` 的 `run_interactive()` 调 `execute_command`；named-command fast-path 调 `find_named_command`。**TUI 不直接调用本 crate**——命令执行统一在 CLI 层。

## 4. 文件清单（按功能分组）

### lib.rs（1876 行）— 核心 + 内置基础命令
`CommandContext`/`CommandResult`/`SlashCommand`；provider 辅助（`provider_lookup_ids` 别名归一、`resolve_fast_model_id` 等）；`split_command_args`（支持引号）；keybindings 模板生成；内置命令：`/help`、`/clear`、`/compact`、`/cost`、`/exit`、`/model`、`/version`、`/resume`、`/status`、`/diff`、`/init`、`/hooks`、`/import-config`、`/thinking`、`NamedCommandAdapter`。

### 会话管理类
- `session.rs`：`/plan`、`/tasks`、`/session`、`/fork`
- `new_move.rs`：`/new`（惰性新会话——重置为空白 home，首条消息才落盘）+ `/move`（会话 re-home 到另一 worktree，含变更迁移）
- `session_tools.rs`：`/skills`、`/rewind`、`/stats`、`/files`、`/rename`、`/effort`、`/summary`、`/commit`
- `history.rs`：shadow-git 快照/回滚——`/undo`、`/revert`、`/checkpoints`、`/snapshot-diff`
- `search.rs`：`/search`（跨全部会话搜索，SQLite）

### 配置类
`config_cmd.rs`（`/config`）、`permissions.rs`（`/permissions`）、`ui_settings.rs`（私有模块，UI 设置读写辅助）。

### 外观/显示类
`appearance.rs`（`/color`、`/theme`、`/output-style`、`/keybindings`、`/privacy-settings`）、`display.rs`（`/context`、`/vim`）、`setup.rs`（`/statusline`、`/security-review`、`/terminal-setup`）。

### Provider / 账号类
`accounts.rs`（`/login`、`/logout`、`/refresh`、`/accounts`、`/switch`）、`providers.rs`（`/providers`、`/connect`、`/agent`）、`usage.rs`（`/usage`、`/extra-usage`）、`extras.rs`（`/advisor`、`/fast`、`/color-set`）。

### MCP / 插件 / 集成类
`mcp.rs`(601)（`/mcp` 状态管理 + OAuth 重连触发）、`plugin.rs`（`/plugin`、`/reload-plugins`）、`chrome.rs`(506)（`/chrome` CDP-over-WebSocket 控制真实 Chrome）、`remote.rs`（`/remote-control`、`/remote-env`）。

### UI / 趣味 / 语音类
`speech.rs`（`/caveman`、`/rocky`、`/normal` persona）、`thinkback.rs`（`/think-back`、`/thinkback-play` 思维轨迹回放）、`copy.rs`（`/copy`）、`share.rs`（`/share` + 二维码、`/links`）。

### 诊断 / 维护类
`doctor.rs`（`/doctor`）、`diagnostics.rs`（`/btw` 旁路提问、`/ctx-viz`、`/heapdump`、`/insights`）、`maintenance.rs`（`/update`、`/release-notes`、`/rate-limit-options`）。

### 高级 agent 能力类
`goal.rs`（`/goal` 持久自主目标：set/status/pause/resume/clear，软预算 + 200 轮失控保护）、`managed_agents.rs`（`/managed-agents` manager-executor 配置）、`review.rs`（`/review` 代码审查，可发 GitHub PR）、`ultrareview.rs`（`/ultrareview` 多维穷尽审查）、`teleport.rs`（`/teleport` 会话上下文导入/导出为可移植 bundle）。

### named_commands.rs（1272 行）
顶层 `claurst <name>` 命令：`agents`（list/create/edit/delete）、`add-dir`、`branch`、`tag`、`ide`、`pr-comments`、`desktop`、`mobile`、`remote-setup`、`ultraplan`（feature gate 下的 agentic 规划器）等 + `render_qr()`。

### stats.rs（1538 行）
持久化会话统计聚合引擎：读取磁盘 JSONL transcript，产出 `summary|sessions|tools|daily|session <id>` 视图。

## 5. 测试

lib.rs 尾部大量 `#[tokio::test]`：注册表完整性、`/new` `/move` 行为、persona 解析、keybindings 模板合法性、参数切分等。

## 6. 设计要点

1. **命令语义与 UI 呈现分层**：命令语义在 commands，执行结果（如 `OpenRewindOverlay`）由 CLI 驱动 TUI 呈现；commands 反向调用 tui 的纯函数（slash 解析、HelpEntry），tui 不调 commands。
2. **四级命令来源**：内置 → settings 模板 → 发现的技能 → 插件，同名按序优先。
3. **双 trait 体系**：slash 命令（会话内 `/xxx`）与 named command（顶层 `claurst xxx`）互相适配。

## 7. `CommandResult` 全部变体（已验证，18 个）

| 变体 | 语义 |
|---|---|
| `Message(String)` | 仅显示，不进模型上下文 |
| `UserMessage(String)` | 作为用户消息注入对话（/compact、模板命令） |
| `ConfigChange(Config)` | 修改配置 |
| `ConfigChangeMessage(Config, String)` | 修改配置 + 显示消息 |
| `McpAuthFlow { server_name, auth_url, redirect_uri }` | 触发后台 MCP OAuth |
| `ClearConversation` | 清空对话 |
| `SetMessages(Vec<Message>)` | 替换消息列表 |
| `ResumeSession(ConversationSession)` | 恢复会话 |
| `RenameSession(String)` | 重命名会话 |
| `StartOAuthFlow(bool)` | 启动 Anthropic OAuth（bool = login_with_claude_ai） |
| `StartLoginForProvider { provider, login_with_claude_ai, label }` | 为指定 provider 启动登录 |
| `Exit` / `Silent` / `Error(String)` | 退出 / 无输出 / 错误 |
| `OpenRewindOverlay` / `OpenHooksOverlay` / `OpenImportConfigOverlay` | 请求 CLI 打开对应 TUI overlay |
| `RefreshProviderState` | 重建 provider 运行时 |
| `NewSession` | 新会话 |
| `MoveSession { destination, moved_changes }` | 会话 re-home（/move） |

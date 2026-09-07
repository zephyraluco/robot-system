# `claurst-tools` — 代理工具系统

> 路径：`crates/tools` ｜ 约 **12,686 行**（src/ 下约 45 个文件）｜ 依赖：`claurst-core`、`claurst-api`、`claurst-mcp`

## 1. 定位

所有 LLM 可调用的工具（agentic tools）的完整实现集合——shell 执行、文件读写编辑、代码搜索、Web 访问、任务/团队/技能管理、`AskUserQuestion` 等。lib.rs 首注释：*"All tool implementations for Claurst. Each tool maps to a capability the LLM can invoke."*

## 2. 依赖

- **内部**：`claurst-core`（config/permission/cost/types/snapshot/file_history）、`claurst-api`、`claurst-mcp`（MCP 资源工具）。**不依赖 query**（`AgentTool` 留在 query 以避免循环依赖）。
- **关键外部**：`tokio`/`tokio-stream`/`tokio-util`/`futures`、`async-trait`、`similar`（diff）、`glob`/`walkdir`/`regex`、**`portable-pty`**（PTY bash）、`reqwest`、`enigo`+`xcap`+`image`（computer-use，可选）、`dashmap`/`parking_lot`、`which`、`open`；Unix 专属 `nix`、`libc`（PTY master fd 轮询，#220/#184）。
- **features**：`computer-use = ["dep:enigo", "dep:xcap", "dep:image"]`。

## 3. 核心抽象（lib.rs，950 行）

### `Tool` trait（async_trait，已验证签名）

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn permission_level(&self) -> PermissionLevel;
    fn self_gates(&self) -> bool { false }   // 默认由中央 backstop 拦截
    fn advanced(&self) -> bool { false }     // 延迟披露元数据（ToolSearch）
    fn input_schema(&self) -> Value;
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult;
    fn to_definition(&self) -> ToolDefinition { ... }
}
```

### 关键类型

| 类型 | 说明 |
|---|---|
| `PermissionLevel` | `None / ReadOnly / Write / Execute / Dangerous / Forbidden`（`Forbidden` 供 bash 分类器识别 `rm -rf /`、fork 炸弹等永不执行） |
| `ToolResult` | `content: String` + `is_error: bool` + `metadata: Option<Value>`（TUI 据此渲染 diff 等） |
| `ToolContext` | 每次工具调用共享上下文（16 个字段，见下） |
| `UserQuestionEvent` | AskUserQuestion ⇄ TUI 事件：`question`、`options`、`reply_tx: oneshot::Sender<String>`（用户回答通道） |
| `PendingPermissionRequest/Store` | 排队给 TUI 的权限请求（queue + waiting map） |

### `ToolContext` 字段（已验证）

```rust
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub permission_mode: PermissionMode,
    pub permission_handler: Arc<dyn PermissionHandler>,
    pub cost_tracker: Arc<CostTracker>,
    pub session_id: String,
    pub file_history: Arc<Mutex<FileHistory>>,      // /rewind 文件恢复
    pub current_turn: Arc<AtomicUsize>,             // 回合局部 diff 重建
    pub non_interactive: bool,                      // headless 模式直接拒绝
    pub mcp_manager: Option<Arc<McpManager>>,
    pub config: Config,                             // 含 hooks
    pub managed_agent_config: Option<ManagedAgentConfig>,
    pub completion_notifier: Option<CompletionNotifier>,
    pub pending_permissions: Option<Arc<Mutex<PendingPermissionStore>>>,
    pub permission_manager: Option<Arc<Mutex<PermissionManager>>>,
    pub user_question_tx: Option<UnboundedSender<UserQuestionEvent>>,
    pub cancel_token: CancellationToken,
}
```

内置 `check_permission*` 系列方法（含 path/details 变体）：交互模式下把 `PermissionRequest` 推入 `PendingPermissionStore` 队列并用 oneshot channel 阻塞等待 TUI 决策；非交互模式下直接 `PermissionDenied`。另有 `record_file_change()` 写入 FileHistory。

### 会话级全局设施

`session_shell_state()`（按 session_id 持久化 cwd/env 的 `ShellState`，dashmap）、`session_shadow()`（core 的 shadow-git 快照）、`write_atomic()`（临时文件+rename 原子写）、`CompletionNotifier`（后台任务完成通知注入下一轮）、**`all_tools()`**（lib.rs:567，工厂函数聚合 **45 个** `Box::new` 工具实例，非 registry 模式）、`find_tool(name)`。

## 4. 工具清单（src/ 文件 → 工具）

### Shell / 执行类
| 文件 | 工具 |
|---|---|
| `pty_bash.rs`(1055) | `PtyBashTool`：PTY 伪终端包 shell；持久化 cwd/env；与 bash_classifier 协作拦截 Critical 命令 |
| `powershell.rs` | `PowerShellTool`（Windows） |
| `repl_tool.rs` | `ReplTool`：持久解释器会话（Python/Node 等） |
| `sleep.rs` | `SleepTool` |

### 文件读写/编辑类
`file_read.rs`（`FileReadTool`）、`file_write.rs`（原子写）、`file_edit.rs`（old/new 精确替换）、`batch_edit.rs`（多文件批量编辑，全部预检后原子应用）、`apply_patch.rs`（统一 diff 应用）、`notebook_edit.rs`（Jupyter notebook）、`line_endings.rs`（CRLF/LF 处理助手）。

### 搜索类
`glob_tool.rs`、`grep_tool.rs`（ripgrep 风格正则搜索）、`tool_search.rs`（模型按名字/关键词检索工具定义，配合 `advanced()` 延迟披露）。

### Web 类
`web_fetch.rs`（抓取网页转 markdown）、`web_search.rs`。

### 计划/交互类
`ask_user.rs`（`AskUserQuestionTool`：暂停循环等待用户回答）、`enter_plan_mode.rs`/`exit_plan_mode.rs`、`brief.rs`、`goal_complete.rs`（/goal 自主循环完成标记）、`todo_write.rs`（任务清单 + 查询循环 todo nudge）。

### 任务/后台类
`tasks.rs`（`TaskCreate/Get/Update/List/Stop/Output` + 全局 `TASK_STORE`）、`monitor_tool.rs`、`cron.rs`（`CronCreate/Delete/List` 定时任务 + `CRON_STORE`）。

### 多代理/协作类
`team_tool.rs`（`TeamCreate/Delete` + `register_agent_runner` 回调注入，打破与 query 的循环依赖：创建 N 个并行子代理 swarm）、`send_message.rs`（`SendMessageTool` + 代理间信箱）、`remote_trigger.rs`（跨会话事件派发）。

### 技能类
`skill_tool.rs`（执行 Markdown 技能模板）、`bundled_skills.rs`（内置技能定义表）。

### 其他
`worktree.rs`（进入/退出 git worktree 隔离区）、`config_tool.rs`、`lsp_tool.rs`（经 LSP 做 hover/定义/引用/诊断）、`mcp_resources.rs`、`mcp_auth_tool.rs`、`synthetic_output.rs`（coordinator 模式结构化输出）、`formatter.rs`（写/编辑后跑格式化器）、`computer_use.rs`（仅 computer-use feature：enigo 鼠标键盘 + xcap 截图）。

## 5. 设计要点

1. **工具定义方式**：每文件实现一个（或一组）`Tool` trait 结构体，`all_tools()` 工厂聚合；无动态注册表。
2. **权限模型三层**：工具内部自检（self_gates）→ query 循环中央 backstop → TUI 弹窗异步决策（oneshot 阻塞等待）。
3. **交互通道三件套**：`QueryEvent`（query→TUI）、`UserQuestionEvent`（tools⇄TUI）、`PendingPermissionStore`（tools→TUI 弹窗）。
4. **`AgentTool` 不在此 crate**：放在 query 层以避免 `tools→query→tools` 循环依赖。

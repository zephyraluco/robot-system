# `claurst-query` — 查询/推理循环

> 路径：`crates/query` ｜ 约 **10,406 行**（16 个源文件）｜ 依赖：`claurst-core`、`claurst-api`、`claurst-plugins`、`claurst-tools`

## 1. 定位

核心 agentic 查询循环——向 API 发消息、处理流式响应、检测并派发工具调用、把工具结果回灌模型、auto-compact 上下文管理、停止条件与预算控制。lib.rs 首注释列出这 6 条职责。

## 2. 依赖

- **内部**：`claurst-core`、`claurst-api`、`claurst-plugins`、`claurst-tools`（query → tools，因此 `AgentTool` 必须放这里）。
- **关键外部**：tokio/tokio-stream/futures/tokio-util（CancellationToken）、serde_json、anyhow/thiserror、dashmap、parking_lot、once_cell。

## 3. 关键类型与公开 API

| 类型 | 说明 |
|---|---|
| `QueryConfig` | 单次循环配置：model、max_tokens/max_turns、system_prompt（基础/追加）、effort_level、thinking_budget/temperature、`tool_result_budget`（默认 50,000 字符）、`command_queue`（T1-4 优先级命令队列）、`skill_index`、`max_budget_usd`、`fallback_model`、`provider_registry`、agent 定义、`model_registry`、`managed_agents`、`enabled_tools`（渐进工具披露 #233）、`continuation`（回合结束延续策略 #230/MI-3）。有 `from_config()`/`from_config_with_registry()` |
| `QueryOutcome` | 循环结果：`EndTurn{message,usage}`、`MaxTokens{partial_message,usage}`、`Cancelled`、`Error`、`BudgetExceeded{cost_usd,limit_usd}` |
| `QueryEvent` | 经 `mpsc::UnboundedSender` 发给 TUI（已验证 7 个变体）：`Stream(AnthropicStreamEvent)`、`ToolStart{tool_name,tool_id,input_json}`、`ToolEnd{tool_name,tool_id,result,is_error}`、`TurnComplete{turn,stop_reason,usage}`、`Status(String)`、`Error(String)`、`TokenWarning{state,pct_used}`（≥80% Warning / ≥95% Critical） |
| `run_query_loop(...)` | 核心入口（约 2800 行巨型函数）：`(client, messages, tools, tool_ctx, config, cost_tracker, event_tx, cancel_token, pending_messages) -> QueryOutcome` |
| `run_single_query(...)` | 单发非 agentic 查询（runner/single.rs） |

其他公开 API：`AgentTool`/`init_team_swarm_runner`、`CommandQueue`/`drain_command_queue`、`ContinuationPolicy`/`StopPolicy`/`TurnEndContext`、`start_cron_scheduler`、goal 系列（`check_and_continue_goal` 等）、skill 系列（`SkillIndex`/`prefetch_skills`）、`sanitize_history`、compact 系列（约 20 个函数/类型）。

## 4. 查询循环工作方式（`run_query_loop` 主流程）

每个 turn 依次执行：

1. **取消检查** — cancel_token 取消 → `Cancelled`。
2. **排队消息注入** — drain `pending_messages`（工具执行期间用户输入）与 `command_queue`（T1-4 优先级命令）。
3. **工具结果预算** — 累计工具结果超预算时用截断占位符替换最旧结果。
4. **历史不变量修复** — `sanitize_history()`（#229/MI-2）：修复 `tool_use ↔ tool_result` 配对破坏，避免 provider 400；幂等。
5. **构建请求 + provider 派发** — 非 anthropic provider 且 registry 有该 provider 时走 `ProviderRegistry`，否则走 `AnthropicClient`；runner/stream.rs 把统一 `StreamEvent` 映射回 `AnthropicStreamEvent`（TUI 只见单一事件类型）。
6. **流式处理** — mpsc 收流，`StreamAccumulator` 累积；45 秒无数据视为停滞（最多自动重试 2 次）；overload/rate-limit 切换 `fallback_model`。
7. **工具调用** — 停止原因为 `tool_use` 时：
   - `parse_tool_args()`：拼接 partial-JSON delta；**非空解析失败必须报错**，绝不静默以 `{}` 执行（#215）；
   - **中央权限 backstop**：`!tool.self_gates()` 且级别属 Write/Execute/Dangerous/Forbidden 时先 `check_permission`；
   - **并发执行**：`run_tool_batch()` 用 `join_all` 并发跑一批工具，`tokio::select!` 监听取消——取消时在飞 future 被放弃、每个位置填合成取消结果，保证历史中每个 tool_use 都有配对 tool_result（#218）；
   - 结果回灌为 tool_result 消息，进入下一 turn。
8. **auto-compact** — `AutoCompactState` 检查 token 占用，接近窗口上限自动压缩；向 TUI 发 `TokenWarning`（80%/95%）。
9. **max_tokens 恢复** — 注入恢复提示（"直接续写"）继续，最多 3 次。
10. **max-steps 优雅降级**（#230/MI-3）— 超过 max_turns 不直接返回，而是跑一个**无工具**的最终总结 turn。
11. **hooks** — PostModelTurn 钩子（同步阻塞，exit>1 硬否决）、Stop 钩子（fire-and-forget）。
12. **end_turn 决策** — `ContinuationPolicy.decide(TurnEndContext)` 决定 `Continue`（goal 自主循环：注入延续消息）或 `Stop`（返回 EndTurn）。
13. **成本追踪** — 每轮检查累计花费，超 `max_budget_usd` → `BudgetExceeded`。
14. **shadow-git 快照** — `auto_commits` 开启时每 turn 捕获 worktree 状态。

每轮还注入 **todo nudge**（未完成 TodoWrite 提醒）与 **skill listing attachment**（技能索引就绪后）。

## 5. 模块清单

### runner/（lib.rs 拆出的助手，#232，`pub use runner::*`）
- `single.rs`：`run_single_query`
- `stream.rs`：统一 provider 流事件 → Anthropic 事件
- `tools.rs`：`parse_tool_args`、`execute_tool`（中央权限 backstop）、`run_tool_batch`（并发+取消）、`build_todo_nudge`
- `hooks.rs`：`fire_post_sampling_hooks`（PostModelTurn）、`stop_hooks_with_full_behavior`（Stop）
- `tool_budget.rs`：`apply_tool_result_budget`
- `provider_options.rs`：按 provider/model 生成请求选项（thinking、effort 等）
- `prompt.rs`：`build_system_prompt`

### 顶层模块
| 文件 | 行数 | 职责 |
|---|---|---|
| `lib.rs` | 3227 | 公开类型 + `run_query_loop` 主体 |
| `compact.rs` | 2266 | auto-compact 服务：`AutoCompactState`、`compact_conversation`、`reactive_compact`、`micro_compact_if_needed`、`snip_compact`、`context_collapse`、`MessageGroup` 分组、上下文窗口解析等 |
| `agent_tool.rs` | 675 | `AgentTool`：派生子代理跑复杂子任务（避免循环依赖故放此层） |
| `session_memory.rs` | 660 | 会话结束后台任务：提取值得记住的事实并持久化到 AGENTS.md |
| `sanitize.rs` | 501 | 消息历史不变量修复，幂等 |
| `auto_dream.rs` | 473 | AutoDream：自动记忆整合守护进程（时间门 + 会话积累阈值） |
| `coordinator.rs` | 383 | Coordinator 模式：多 worker 编排 |
| `context_analyzer.rs` | 310 | 上下文窗口分析（/ctx-viz 用） |
| `skill_prefetch.rs` | 200 | 后台读取技能定义建可搜索索引 |
| `away_summary.rs` | 184 | "While you were away" 离开回顾 |
| `command_queue.rs` | 174 | T1-4 优先级命令队列 |
| `continuation.rs` | 154 | 回合结束延续策略 |
| `goal_loop.rs` | 143 | /goal 延续引擎：runaway/预算守卫、延续决策、完成标记 |
| `cron_scheduler.rs` | 121 | 每分钟检查 `CRON_STORE` 的后台任务 |
| `managed_orchestrator.rs` | 98 | manager-executor 托管代理提示注入 |

## 6. 设计要点

1. **单循环多职责**：流式、工具派发、压缩、预算、hooks、goal 续航全部收敛在 `run_query_loop`。
2. **事件归一**：所有 provider 的流事件统一映射回 `AnthropicStreamEvent`，TUI 无感。
3. **健壮性不变量**：历史配对修复（幂等）、取消时 tool_result 补齐、tool args 解析失败必报错、max_tokens/max_turns 优雅恢复。
4. **与 tools 的分层**：tools 依赖 core/api/mcp；query 依赖 tools 并持有 `AgentTool` 与中央权限 backstop。

## 7. 关键公开 API 汇总

| 类别 | 条目 |
|---|---|
| **入口** | `run_query_loop(...)`（核心 agentic 循环）、`run_single_query(...)`（单发查询） |
| **配置/结果** | `QueryConfig`（`from_config` / `from_config_with_registry`）、`QueryOutcome`（EndTurn/MaxTokens/Cancelled/Error/BudgetExceeded） |
| **事件** | `QueryEvent`（7 变体，见 §3） |
| **代理** | `AgentTool`、`init_team_swarm_runner` |
| **队列** | `CommandQueue`、`QueuedCommand`、`drain_command_queue` |
| **续航** | `ContinuationPolicy`、`ContinuationMode`、`StopPolicy`、`TurnEndContext`、`check_and_continue_goal`、`decide_goal_continuation`、`mark_goal_complete`、`GoalContinuation` |
| **技能** | `SkillDefinition`、`SkillIndex`、`prefetch_skills`、`format_skill_listing` |
| **压缩** | `AutoCompactState`、`should_auto_compact`、`compact_conversation`、`reactive_compact`、`micro_compact_if_needed`、`snip_compact`、`TokenWarningState`、`context_window_for_model`、`resolve_context_window`、`get_compact_prompt` 等（约 20 个） |
| **其他** | `sanitize_history`、`start_cron_scheduler`、`context_analyzer`（/ctx-viz）、`away_summary` |

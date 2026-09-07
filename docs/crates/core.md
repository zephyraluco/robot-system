# `claurst-core` — 核心领域层

> 路径：`crates/core` ｜ 约 **26,074 行 / 56 个源文件**（workspace 第二大 crate）｜ 依赖图最底层（零内部依赖，被全部其他 crate 依赖）

## 1. 定位

全 workspace 的核心领域层：类型系统、错误处理、配置/Settings、会话持久化、认证凭据、权限、记忆（AGENTS.md/memdir）、上下文组装、成本统计、后台任务等一切基础业务能力的单点归属地。lib.rs 头部自述：*"cc-core: Core types, error handling, configuration, settings, and constants for Claurst."*

## 2. 依赖

- **内部依赖**：无（依赖图最底层）。
- **关键外部依赖**：`tokio`、`serde/serde_json/toml`、`anyhow` + `thiserror`、`tracing`、`uuid`、`chrono`、`dirs`、`parking_lot`、`dashmap`、`indexmap`、`regex`、`base64/sha2/hex/getrandom`、`url/urlencoding`、`schemars`、`reqwest`、`glob`、`similar`、`which`、`tokio-util`（CancellationToken）、`tokio-tungstenite`（WebSocket，remote session 同步）、`rusqlite`（SQLite 会话存储）。

## 3. Features（36 个编译期实验特性门控）

默认仅 `ultraplan`。`dev_full` 一次性开启全部。下游 `tui`、`commands` 通过 `feature = ["xxx"] = ["claurst-core/xxx"]` 转发。

| 类别 | Features |
|---|---|
| 交互/UI | `ultraplan`、`ultrathink`、`history_picker`、`token_budget`、`message_actions`、`quick_search`、`away_summary`、`hook_prompts`、`kairos_brief`、`kairos_channels`、`lodestone` |
| Agent/记忆 | `agent_triggers`、`agent_triggers_remote`、`extract_memories`、`verification_agent`、`builtin_explore_plan_agents`、`cached_microcompact`、`compaction_reminders`、`agent_memory_snapshot`、`teammem` |
| 工具/基建 | `bash_classifier`、`bridge_mode`、`mcp_rich_output`、`connector_text`、`unattended_retry`、`new_init`、`powershell_auto_mode`、`shot_stats`、`tree_sitter_bash`、`tree_sitter_bash_shadow`、`prompt_cache_break_detection`、`native_clipboard_image`、`ccr_auto_connect`、`ccr_mirror`、`ccr_remote_setup` |

## 4. 结构总览（56 个文件）

`lib.rs`（约 5200+ 行）含 11 个内联模块；其余 45 个独立文件 + 2 个子目录（`snapshot/`、`share_export/`）。

### 4.1 lib.rs 内联模块

| 模块 | 职责 |
|---|---|
| `error` | 统一错误类型 `ClaudeError`（Api/Auth/PermissionDenied/Tool/RateLimit/ContextWindowExceeded/Cancelled/…）+ `Result<T>`；`is_retryable()`（429/529）、`is_context_limit()` |
| `types` | 消息核心类型：`Role`、`ContentBlock`（13 种变体，含 Thinking/ToolUse/ToolResult/Gemini `thought_signature`）、`Message`（含 uuid/cost/snapshot_patch 与大量构造/查询方法）、`ToolDefinition`、`UsageInfo`、`MessageCost` 等 |
| `config` | 配置体系核心（见下） |
| `constants` | 全局常量：默认模型（`DEFAULT_MODEL`）、token 限制、API 端点/beta header、文件名（AGENTS.md/settings.json/.claurst）、内置工具名字符串、重试预算 |
| `context` | `ContextBuilder`：每轮系统上下文（平台/cwd/git status/IDE）与用户上下文（日期 + 全局/项目 AGENTS.md 收集） |
| `permissions` | 权限系统（见下） |
| `history` | `ConversationSession`、`SessionCheckpoint`、`save/load/list/delete/rename/tag/branch/search_sessions`（文件系统 JSON 存储） |
| `cost` | `ModelPricing`（Opus/Sonnet/Haiku/FREE 四档单价 + 免费上游 ID 列表）；`CostTracker`——无锁 AtomicU64 线程安全累计器 |
| `hooks` | 用户配置钩子执行：`HookContext`（stdin JSON）、`HookOutcome`（Allowed/Blocked/Modified）、`run_hooks()` |
| `oauth` | Anthropic OAuth 2.0 PKCE：Console 与 Claude.ai 双路径、`OAuthTokens`（持久化 `~/.claurst/oauth_tokens.json`）、PKCE 辅助、`build_auth_url()` |
| `tasks` | 全局后台任务注册表：`TaskRegistry`（DashMap 进程级单例）、`BackgroundTask`（含 CancellationToken 与 pid） |

### 4.2 config 模块重点

- **`Settings`**（`~/.claurst/settings.json`）：内嵌 `config` + `projects`、`permission_rules`、插件启停、`provider/providers`、`model_overrides`、`commands`（自定义斜杠命令模板）、`agents`、`managed_agents`、大量 UI 开关。
- **`Config`**（运行时生效配置）：`effective_model()`（各 provider 默认模型回退表）、`resolve_provider_api_key()`（config → env var → AuthStore 三级解析 + `{env:VAR}` 替换）、`resolve_auth_async()`（OAuth Bearer vs API key 双路径）。
- **`Settings::config_dir()`** —— 全 workspace 主目录唯一解析点（`$CLAURST_HOME` → `~/.claurst` → XDG）。
- **`load_hierarchical(cwd)`**：全局 + 项目级 `.claurst/settings.json[c]` 合并，项目优先，项目 MCP server 强制打 `McpServerOrigin::Project` 标记。
- **Agent**：`AgentDefinition`、`default_agents()`、`ManagedAgentConfig`（manager-executor + `BudgetSplitPolicy`）、6 个 `ManagedAgentPreset`。
- **MCP**：`McpServerConfig`（含 `origin: McpServerOrigin`，防仓库伪造绕过信任门）。
- 其他：`strip_jsonc_comments()`/`substitute_env_vars()`、`PermissionMode`、`Theme`、`CommandTemplate` 等。

### 4.3 permissions 模块重点

- `PermissionLevel`（Read/Write/Execute/Network）、`PermissionAction`、`PermissionScope`（Session/Persistent）。
- `PermissionRule`（tool + glob path 匹配）与持久化形态 `SerializedPermissionRule`。
- `PermissionDecision`（Allow/AllowPermanently/Deny/DenyPermanently/Ask）+ `format_permission_reason()`。
- **`PermissionManager`**：mode、会话/持久规则、in-flight `PendingPermission`（oneshot channel 供 TUI/bridge 异步裁决）、工作区路径边界判断。
- 四种 `PermissionHandler` 实现：`Auto`、`Interactive`、`ManagedAuto`、`ManagedInteractive`。

### 4.4 独立文件（按功能分组）

**配置/设置类**：`settings_sync.rs`（与 claude.ai 双向同步）、`remote_settings.rs`（企业托管设置）、`import_config.rs`（从 Claude Code 导入）、`migrations.rs`（版本迁移）、`oauth_config.rs`、`feature_flags.rs`（GrowthBook）、`feature_gates.rs`、`mcp_trust.rs`（项目级 MCP 信任门，防 RCE）、`mcp_templates.rs`（`{{var}}` 模板渲染）、`paths.rs`、`output_styles.rs`、`system_prompt.rs`（模块化 system prompt 组装 + 缓存）。

**认证/账号类**：`auth_store.rs`（`~/.claurst/auth.json` per-provider 凭据）、`accounts.rs`（多账号 profile 切换）、`device_code.rs`（GitHub Device Code Flow）、`codex_oauth.rs`（Codex OAuth 常量）、`crypto_utils.rs`。

**会话/存储类**：`session_storage.rs`（JSONL 转录持久化，TS 兼容 schema）、`sqlite_storage.rs`（`SqliteSessionStore`）、`cloud_session.rs`（云会话 API）、`remote_session.rs`（WebSocket 后台同步到 Claude.ai）、`prompt_history.rs`（`history.jsonl` + 大粘贴外存）、`file_history.rs`（/rewind 文件恢复）、`session_tracing.rs`（OTel no-op 桩）、`goal.rs`（/goal 持久目标，goals.sqlite）、`share_export/`（/share 自包含 HTML + gist）、`snapshot/`（影子 git 快照/撤销：`ShadowSnapshot` + per-session registry）。

**记忆/上下文类**：`claudemd.rs`（AGENTS.md 分层加载 managed > user > project > local）、`memdir.rs`（内存目录系统）、`team_memory_sync.rs`（团队记忆 delta 推送）、`context_collapse.rs`（上下文折叠）、`attachments.rs`（每轮附件组装管线）、`token_budget.rs`、`truncate.rs`/`format_utils.rs`/`message_utils.rs`。

**权限/分类器类**：`bash_classifier.rs`（Bash 命令风险分类与自动批准判定）、`ps_classifier.rs`（PowerShell 同构版）、`auto_mode.rs`（自动批准模式跟踪）。

**其他基础能力**：`keybindings.rs`（`KeyContext` + 可配置键位）、`analytics.rs`（`SessionMetrics`，全 AtomicU64）、`lsp.rs`（LSP JSON-RPC 客户端 + `LspManager`）、`provider_id.rs`（`ProviderId`/`ModelId` 品牌化 newtype）、`effort.rs`（全 workspace 唯一 `EffortLevel` 枚举：Low/Medium/High/Max/XHigh/Ultracode）、`keywords.rs`、`skill_discovery.rs`、`ide.rs`、`update_check.rs`、`tips.rs`、`spinner.rs`（spinner 动词）、`git_utils.rs`。

### 4.5 tests/

`snapshot_tests.rs`（ShadowSnapshot 集成测试）、`parity_smoke.rs`（与 TS 数据结构对齐冒烟）、`test_mcp_templates.rs`。

## 6. 关键公开 API（crate 根 re-export）

| 类别 | 条目 |
|---|---|
| **错误** | `ClaudeError`（15 变体）、`Result<T>`、`is_retryable()`、`is_context_limit()` |
| **消息类型** | `Role`、`ContentBlock`（13 变体）、`Message`、`MessageContent`、`MessageCost`、`ToolDefinition`、`UsageInfo`、`ImageSource`/`DocumentSource` |
| **配置** | `Config`、`Settings`、`AgentDefinition`、`default_agents()`、`ManagedAgentConfig`、`ManagedAgentPreset`、`builtin_managed_agent_presets()`、`McpServerConfig`、`McpServerOrigin`、`PermissionMode`、`ProviderConfig`、`ModelOverride`、`SkillsConfig`、`Theme`、`OutputFormat`、`CommandTemplate`、`HookEvent`/`HookEntry` |
| **认证** | `AuthStore`、`StoredCredential`、`OAuthTokens`、PKCE 辅助、`device_code` 流 |
| **权限** | `PermissionManager`、`PermissionLevel`、`PermissionRule`、`PermissionDecision`、`PermissionHandler`（Auto/Interactive/ManagedAuto/ManagedInteractive 四实现） |
| **会话** | `ConversationSession`、`SessionCheckpoint`、`save/load/list/delete/rename/tag/branch/search_sessions`、`SqliteSessionStore`/`SessionSummary` |
| **成本** | `ModelPricing`、`CostTracker` |
| **标识** | `ProviderId`、`ModelId`（品牌化 newtype）、`EffortLevel`（Low/Medium/High/Max/XHigh/Ultracode） |
| **其他** | `Goal*` 系列、`FeatureFlagManager`、`IdeKind`/`detect_ide`、`check_for_updates`/`UpdateInfo`、`discover_skills`/`DiscoveredSkill`、import_config 全套、spinner 动词常量、`KeybindingResolver`/`UserKeybindings`、`ContextBuilder`、`TaskRegistry`、`run_hooks`/`HookOutcome` |

## 7. 设计要点

1. **唯一依赖图底座**：不依赖任何内部 crate，其他 10 个 crate 全部直接依赖它。
2. **配置路径单点**：`Settings::config_dir()` 是全 workspace 主目录唯一解析点（`$CLAURST_HOME` → `~/.claurst` → XDG）。
3. **Feature 门控实验特性**：36 个 feature 让实验功能可在编译期开关，由 tui/commands 转发联动。
4. **权限机制统一归属**：tools/query/tui 的所有权限交互类型（`PermissionRequest`/`PermissionDecision`/Handler）都定义在此。
5. **TS 兼容性**：session_storage JSONL schema、prompt_history、claudemd 等与 TypeScript 版数据结构互通（`parity_smoke.rs` 冒烟测试保障），未知条目以 `Other(Value)` tombstone 保留。

# `claurst-mcp` — MCP 客户端实现层

> 路径：`crates/mcp` ｜ 约 **4,407 行 / 6 个源文件**（lib.rs 约 1800 行含 3 个内联模块）｜ 依赖：`claurst-core` ｜ 被 cli、commands、tools、tui 依赖

## 1. 定位

MCP（Model Context Protocol）客户端实现层：JSON-RPC 2.0 客户端原语、协议握手、工具/资源/prompt 发现与调用、stdio 与 HTTP/SSE 传输、OAuth 认证、多服务器连接管理与断线重连。

## 2. 依赖

- **内部**：`claurst-core`（`McpServerConfig`、`ToolDefinition`、`mcp_templates::TemplateRenderer`、`Settings::config_dir`）。
- **关键外部**：**`rmcp = 1.4.0`**（官方 Rust MCP SDK，features：`client`、`auth`、`transport-child-process`、`transport-streamable-http-client-reqwest` 等）、`tokio`/`tokio-stream`/`tokio-util`、`reqwest`、`dashmap`、`open`（浏览器 OAuth）。

## 3. 结构（6 个文件，lib.rs 约 1800 行含 3 个内联模块）

### lib.rs

- **环境变量展开**：`expand_env_vars()`（`${VAR}` 与 `${VAR:-default}`，未知变量保留原文，镜像 TS 行为）+ `expand_server_config()`。
- **内联 `types` 模块**：`JsonRpcRequest/Response/Error`；`ServerCapabilities`/`ServerInfo`；`McpTool`、`McpContent`（Text/Image/Audio/Resource）、`McpResource`、`McpPrompt` 等。
- **内联 `transport` 模块**：`McpTransport` trait（send/recv/close/通知订阅）；协议版本常量（legacy SSE `2024-11-05` 与 streamable HTTP 各版本逐版本协商降级）；SSE 解析辅助。
- **内联 `client` 模块**：**`McpClient`**——与单服务器完成初始化握手的客户端；`connect()` 按 `server_type` 分派 stdio/sse/http；提供 `list_tools/call_tool/list_resources/read_resource/list_prompts/get_prompt/subscribe_resource` 等；`process_notification()` 处理资源更新与工具变更通知；资源注解里的 prompt 模板经 `TemplateRenderer` 渲染。
- **`McpAuthState`**：NotRequired / Required{auth_url} / Authenticated{token_expiry} / Error。
- **`McpManager`**（多服务器连接池）：`HashMap<String, Arc<McpClient>>` + `failed_servers` + `resource_subscriptions: DashMap`。方法：`connect_all(configs)`、`all_statuses`、**`all_tool_definitions()`**（返回 `(server_name, ToolDefinition)` 供 API 注入）、`call_tool`（按服务器路由）、`server_instructions`（注入 system prompt）、资源/prompt 批量接口、OAuth 系列（`initiate_auth/authenticate/store_token/load_token`）、`spawn_notification_poll_loop()`。
- `mcp_result_to_string()`：把 `CallToolResult` 内容块拍平成字符串。

### 其他文件

| 文件 | 职责 |
|---|---|
| `backend.rs` | 传输无关的**后端抽象层**：`McpClientBackend` async trait + `McpClientSnapshot`（服务器信息/能力/工具的不可变快照）。设计目标：上层保持稳定，传输实现可独立演进 |
| `rmcp_backend.rs` | 官方 `rmcp` SDK 后端：stdio（`TokioChildProcess`）、streamable HTTP（带 Bearer）、legacy SSE；通知收集到 unbounded channel |
| `connection_manager.rs` | 镜像 TS `useManageMCPConnections` 的重连管理：**`McpConnectionManager`** per-server 状态跟踪、指数退避重连循环；**`McpServerStatus`** 枚举（Connected/Connecting/Disconnected/Failed）+ `display()`（`/mcp status` 文案） |
| `registry.rs` | 官方 MCP 服务器静态注册表（filesystem/github/gitlab/google-drive 等，含 install_command 与分类）；`is_official_mcp_url()` |
| `oauth.rs` | MCP OAuth / XAA IdP 登录流程：`McpToken`（含过期宽限）、token 存储 `<claurst home>/mcp-tokens/`（文件名安全过滤防路径穿越）、PKCE 授权 + 本地回环 redirect |

> **Elicitation 说明**：MCP 服务器发起的 elicitation 表单对话框 UI 位于 `crates/tui/src/elicitation_dialog.rs`；插件可通过 `HookEventKind::Elicitation/ElicitationResult` 订阅事件。本 crate 不含 elicitation UI。

## 4. 关键公开 API

| 类别 | 条目 |
|---|---|
| **环境展开** | `expand_env_vars()`（`${VAR}` / `${VAR:-default}`）、`expand_server_config()` |
| **JSON-RPC 类型** | `JsonRpcRequest`（`new`/`notification`）、`JsonRpcResponse`、`JsonRpcError` |
| **服务器信息** | `ServerCapabilities`（tools/resources/prompts 子能力）、`ServerInfo` |
| **工具** | `McpTool`、`ListToolsResult`、`CallToolParams`、`CallToolResult`、`McpContent`（Text/Image/Audio/Resource） |
| **资源/prompt** | `McpResource`、`ResourceContents`、`ListResourcesResult`、`McpPrompt`、`McpPromptArgument`、`PromptMessage`、`PromptMessageContent` |
| **客户端** | `McpClient`（connect/list_tools/call_tool/list_resources/read_resource/list_prompts/get_prompt/subscribe_resource/tool_definitions/process_notification） |
| **管理** | `McpManager`（connect_all/all_tool_definitions/call_tool/server_instructions/all_statuses/spawn_notification_poll_loop）、`McpConnectionManager`/`McpServerStatus` |
| **认证** | `McpAuthState`、`McpToken`、`store_mcp_token`/`get_mcp_token`/`remove_mcp_token`、PKCE 流程 |
| **辅助** | `mcp_result_to_string()`、`OfficialMcpServer`/`OFFICIAL_SERVERS`、`is_official_mcp_url()` |

## 5. 设计要点

1. **后端抽象**：`McpClientBackend` trait 让传输实现（rmcp / 未来其他）可替换，上层 `McpManager`/tools/TUI 稳定。
2. **逐版本协议协商**：streamable HTTP 按版本列表逐个尝试，兼容新旧服务器。
3. **信任边界**：项目级 MCP server 的信任门在 core（`mcp_trust.rs`/`McpServerOrigin`），本 crate 只负责连接。
4. **三重集成**：工具注入（`all_tool_definitions` → API）、system prompt（`server_instructions`）、TUI 面板（`McpViewState` 消费 `all_statuses`）。

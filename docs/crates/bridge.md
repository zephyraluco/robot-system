# `claurst-bridge` — 远程控制桥

> 路径：`crates/bridge` ｜ 单文件 `lib.rs`（**1,710 行**）｜ 依赖：`claurst-core`、`claurst-api`、`claurst-query` ｜ 被 cli、commands 依赖（TUI 经 `TuiBridgeEvent` 消费桥事件）

## 1. 定位

**远程控制桥（Remote Control bridge）**：把本地 Claurst CLI 连接到 claude.ai Web UI，使移动端/网页可以发起和控制本地会话。架构与 TypeScript 版 `bridgeMain.ts`/`bridgeApi.ts` 一一对应，移植为 tokio channels + reqwest 的 Rust async 实现。

## 2. 依赖

- **关键外部**：tokio/tokio-stream/futures/`tokio-util`（CancellationToken）、reqwest、uuid、sha2/base64/hex、hostname、dirs。

## 3. 内容分区（单文件按注释分区组织）

| 区域 | 内容 |
|---|---|
| JWT 工具 | `JwtClaims`（sub/exp/iat/device_id/session_id；客户端侧解码**不验签**，仅用于过期检查与展示；识别并剥离 `sk-ant-si-` 会话入口前缀）；`jwt_is_expired` |
| 设备指纹 | `device_fingerprint()`：hostname + 登录用户 + home 目录做 SHA-256，与 TS `trustedDevice.ts` 算法一致 |
| 配置 | `BridgeConfig`：enabled / server_url（默认 `https://claude.ai`）/ device_id / session_token / polling_interval_ms=1000 / max_reconnect_attempts=10 / session_timeout_ms=24h；`from_env()` 读 `CLAURST_BRIDGE_URL`、`CLAURST_BRIDGE_TOKEN`/`CLAUDE_BRIDGE_OAUTH_TOKEN` 等；`validate_id()` 防路径穿越 |
| 消息/事件协议 | `BridgeMessage`（Web UI → CLI：UserMessage/PermissionResponse/Cancel/Ping）；`BridgeEvent`（CLI → Web UI：TextDelta/ToolStart/ToolEnd/PermissionRequest/TurnComplete/Error/Pong/SessionState）；`PermissionDecision`（Allow/AllowPermanently/Deny/DenyPermanently）；`BridgeAttachment`、`BridgeUsage`、`BridgeSessionState` |
| 会话 | `BridgeSession`：后台 tokio 任务跑轮询循环（register → poll → upload events → deregister，指数退避 + 取消） |
| 管理器 | `BridgeManager`（配置 + 共享 HTTP client 的高层包装） |
| 公开 API | `start_bridge(config, cancel) -> (msg_rx, event_tx, session_id)`（有界通道 64/256，背压防内存膨胀）；`start_bridge_session(token) -> BridgeSessionInfo{session_id, session_url, token}`（生成 UUID、注册、返回可分享 URL）；`poll_bridge_messages` / `post_bridge_response` / `post_bridge_event` |
| TUI 集成 | **`TuiBridgeEvent`**（桥工作线程 → 主循环：Connected/Disconnected/Reconnecting/InboundPrompt/Cancelled/PermissionResponse/SessionNameUpdate/Error）；**`BridgeOutbound`**（查询循环 → 桥 → Web UI）；**`run_bridge_loop`** —— 高层任务入口：注册重试退避（auth 错误致命不重试）、双向翻译消息、`{server_url}/remote?session={id}` 生成会话 URL |
| 收尾 | `trusted_device` / `jwt` 公共子模块 + 单元测试（serde 往返、指纹稳定性、路径穿越拒绝等） |

## 4. 关键类型与 API（已验证）

- **枚举**：`BridgeMessage`（Web→CLI：UserMessage/PermissionResponse/Cancel/Ping）、`BridgeEvent`（CLI→Web：TextDelta/ToolStart/ToolEnd/PermissionRequest/TurnComplete/Error/Pong/SessionState）、`BridgeState`、`BridgeSessionState`、`PermissionDecision`（Allow/AllowPermanently/Deny/DenyPermanently）、`TuiBridgeEvent`、`BridgeOutbound`
- **结构**：`BridgeConfig`（`from_env()`/`is_active()`/`validate_id()`）、`BridgeSession`（`register()`/`deregister()`/`run_poll_loop()`）、`BridgeManager`（`start()`）、`BridgeSessionInfo{session_id, session_url, token}`、`JwtClaims`（`decode()`/`is_expired()`/`remaining_secs()`）、`BridgeAttachment`、`BridgeUsage`
- **函数**：`device_fingerprint()`（SHA-256）、`start_bridge(config, cancel) -> (msg_rx, event_tx, session_id)`、`start_bridge_session(token)`、`poll_bridge_messages`/`post_bridge_response`/`post_bridge_event`、`run_bridge_loop`
- 另 `pub use reqwest;` 方便下游免直连依赖。

## 5. 设计要点

1. **纯通道/任务架构**（无 trait）：后台轮询任务 + 双向 mpsc channel，TUI 与查询循环各自消费一侧。
2. **背压设计**：有界通道（64/256）防止 Web 端风暴撑爆内存。
3. **重试策略**：指数退避重连，auth 类错误直接致命不重试。
4. **与 query/tui 的对接**：`run_bridge_loop` 把 `BridgeOutbound` 序列化上传、把 `BridgeMessage` 翻译为 `TuiBridgeEvent` 交主循环处理。

# `claurst-api` — 多 Provider LLM 统一适配层

> 路径：`crates/api` ｜ 约 **19,300 行 / 34 个源文件** ｜ 依赖：`claurst-core` ｜ 被除 core/buddy/mcp/plugins 外的所有 crate 依赖（bridge、cli、commands、query、tools、tui）

## 1. 定位

项目的**多 Provider LLM 统一适配层**。从早期 "Anthropic API client"（SSE 流式）演化为覆盖 Anthropic / OpenAI / Google 等 40+ 上游的统一 Provider 抽象层，包含流式 SSE 解析、模型目录（models.dev）、推理努力档位（effort ladder）、Provider 感知错误处理、以及 BoringSSL TLS 指纹伪装。

核心设计目标：**query/TUI 层只面对 `LlmProvider` trait + `StreamEvent` 流**，厂商差异（端点、认证、线格式、行为怪癖）全部在本 crate 内消化。

## 2. 依赖

- **内部**：仅 `claurst-core`（types、`ProviderId`、error、constants、config、auth/codex_oauth）。
- **关键外部**：
  - 异步与流式：`tokio`（full）、`tokio-stream`、`futures`、`async-trait`、`async-stream`
  - HTTP：`reqwest`（json/stream/rustls/multipart/form/query）；**`wreq` + `wreq-util`**（钉死 `=6.0.0-rc.29` / `=3.0.0-rc.12`，BoringSSL TLS 指纹伪装客户端，专用于 Anthropic 通道）
  - 序列化/杂项：`serde`/`serde_json`、`anyhow`/`thiserror`、`tracing`、`bytes`、`once_cell`、`regex`、`parking_lot`、`chrono`、`sha2`/`hex`/`base64`/`urlencoding`/`url`/`uuid`、`hmac 0.12`
- **资源**：`assets/models-snapshot.json`（models.dev 编译期快照，约 118 provider / 4500 model）。
- **features**：未声明任何 feature。

## 3. 核心抽象（trait）

### 3.1 `LlmProvider`（`provider.rs`，核心抽象 Phase 1B）

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {          // 可放 Arc<dyn LlmProvider>
    fn id(&self) -> &ProviderId;
    fn name(&self) -> &str;
    async fn create_message(&self, ...) -> Result<ProviderResponse, ProviderError>;
    async fn create_message_stream(&self, ...)
        -> Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>, ProviderError>;
    async fn discover_models(&self) -> Result<Vec<ModelInfo>, ProviderError> { Ok(vec![]) }
    async fn health_check(&self) -> Result<ProviderStatus, ProviderError>;
    fn capabilities(&self) -> ProviderCapabilities;
}
```

`ModelInfo`（同文件）：静态模型元数据，含 `release_date` / `status`，驱动 `/model` picker 按日期倒序。
注：`discover_models` 默认返回空——目录型 provider 不覆盖它，模型列表来自 `ModelRegistry` 只读投影，仅本地运行时（ollama 等）等动态端点才实现。

### 3.2 `AuthProvider`（`auth.rs`）

```rust
pub struct LoginFlow { pub auth_url: String, pub instructions: String, pub method: ... }
#[async_trait]
pub trait AuthProvider: Send + Sync {
    async fn get_credentials(&self) -> Result<AuthMethod, ProviderError>;
    async fn is_authenticated(&self) -> bool;
    async fn login(&self) -> Result<LoginFlow, ProviderError>;
    async fn logout(&self) -> Result<(), ProviderError>;
}
```

自动刷新 OAuth token 的逻辑内聚在各 provider 的 `get_credentials` 实现中。

### 3.3 `StreamParser`（`stream_parser.rs`）

```rust
pub trait StreamParser: Send + Sync {
    fn parse(&self, response: reqwest::Response)
        -> Pin<Box<dyn Stream<Item = StreamEvent> + Send>>;
}
```

- `SseStreamParser` / `JsonLinesStreamParser`：两个标记实现。
- **`SseByteDecoder`**（#228）：字节缓冲行解码器，`push(&[u8]) -> Vec<String>` + `flush() -> Option<String>`——只在最后一个 `\n` 前解码，解决跨网络 chunk 的多字节 UTF-8 码点被 `from_utf8_lossy` 撕裂的历史 bug。

### 3.4 `MessageTransformer`（`transform.rs`，Phase 4）

```rust
pub trait MessageTransformer: Send + Sync {
    fn to_provider(&self, request: &ProviderRequest) -> Result<serde_json::Value, ProviderError>;
    fn from_provider(&self, json: &serde_json::Value) -> Result<ProviderResponse, ProviderError>;
    fn apply_caching(&self, request_json: &mut serde_json::Value, _model: &ModelInfo) {} // 默认 no-op
}
```

### 3.5 `LineStreamDecoder`（`protocol/mod.rs`，sans-IO 协议层 #228）

```rust
pub trait LineStreamDecoder: Send {
    /// 喂入一行，产出事件；返回 true 表示流结束（如 `data: [DONE]`）
    fn feed_line(&mut self, line: &str, out: &mut Vec<StreamEvent>) -> bool;
    /// 流尾补发尾事件（如 MessageStop）
    fn finish(&mut self, out: &mut Vec<StreamEvent>);
}
```

解码器不碰网络，可脱离真实端点单测。目前只下沉了 OpenAI-Chat 的**流解码**半边（`protocol/openai_chat.rs` 的 `OpenAiChatDecoder`，494 行，服务 OpenAI 及 ~35 家兼容厂商），其余格式（AnthropicMessages、OpenAiResponses、Gemini、BedrockConverse）留有 `TODO(#228)`。thinking 块用专用索引 `THINKING_BLOCK_INDEX = usize::MAX - 100` 避免与 text(0)/tool(1+) 冲突。

## 3A. 统一类型层（`provider_types.rs`，600 行，Phase 1A）

### `ProviderRequest`（Provider 无关的规范化请求）

字段：`model`、`messages`、`system_prompt`、`tools`、`max_tokens`、`temperature`、`top_p`、`top_k`、`stop_sequences`、`thinking`、`provider_options`（任意 JSON，承载无法用公共字段表达的 provider 专属参数）。

### `StreamEvent`（统一流事件，11 个变体）

`MessageStart{id,model,usage}`、`ContentBlockStart{index,content_block}`、`TextDelta{index,text}`、`ThinkingDelta{index,thinking}`、`InputJsonDelta{index,partial_json}`、`SignatureDelta{index,signature}`、`ContentBlockStop{index}`、`MessageDelta{stop_reason,usage}`、`MessageStop`、`Error{error_type,message}`、`ReasoningDelta{index,reasoning}`（部分 provider 的思考别名）。

### `StopReason`

`EndTurn`（默认）、`StopSequence`、`MaxTokens`、`ToolUse`、`ContentFiltered`、`Other(String)`。

### `StreamBlockAccumulator` / `PartialBlock`

流式内容块累积器：`Text` / `Thinking{thinking_buf,signature_buf}` / `ToolUse{id,name,json_buf,thought_signature}`（Gemini `thoughtSignature` 透传，#311）/ `Passthrough(ContentBlock)`（RedactedThinking、图像等整块到达的情形）。

### `ProviderCapabilities`（能力声明，10 个字段）

`streaming`、`tool_calling`、`thinking`、`image_input`、`pdf_input`、`audio_input`、`video_input`、`caching`、`structured_output`、`system_prompt_style`。

### 其他枚举

| 枚举 | 变体/说明 |
|---|---|
| `SystemPromptStyle` | `TopLevel`（Anthropic 顶层 system 字段）/ `SystemMessage`（OpenAI 首条 role=system 消息）/ `SystemInstruction`（Gemini） |
| `ProviderStatus` | 端点健康状态（health_check 返回，serde tag="status"） |
| `AuthMethod` | 凭证形态（API key / OAuth Bearer 等） |
| `ApiKeyHeader` | API key 头部形态（`x-api-key` vs `Authorization: Bearer` 等） |

## 4. 统一错误类型（`provider_error.rs`，210 行）

`ProviderError` 共 **10 个变体**，各 provider 都映射到它：

| 变体 | 字段 | 语义 |
|---|---|---|
| `ContextOverflow` | `provider, message, max_tokens: Option<u64>` | 超出模型上下文窗口 |
| `RateLimited` | `provider, retry_after: Option<u64>` | HTTP 429 或等价限流信号 |
| `AuthFailed` | `provider, message` | API key / 凭证被拒 |
| `QuotaExceeded` | `provider, message` | 配额耗尽 |
| `ModelNotFound` | `provider, model, suggestions: Vec<String>` | 模型不存在/不可访问，附替代建议 |
| `ServerError` | `provider, status: Option<u16>, message, is_retryable` | 5xx 等服务端错误 |
| `InvalidRequest` | `provider, message` | 请求本身畸形/参数无效 |
| `ContentFiltered` | `provider, message` | 被内容安全系统拦截 |
| `StreamError` | `provider, message, partial_response: Option<String>` | 流开始后出错，携带已收到的部分内容 |
| `Other` | `provider, message, status, body: Option<String>` | 兜底变体 |

方法：`is_retryable()`、`provider_id()`；实现 `From<ProviderError> for ClaudeError` 以对接 core 错误体系。

## 5. 文件清单与职责（按行数）

| 文件 | 行数 | 职责 |
|---|---|---|
| `model_registry.rs` | 2114 | **模型注册表（Phase 3）**，见 §7 |
| `lib.rs` | 1666 | crate 入口 + 遗留核心（见 §6）+ 进程级请求超时管理（issue #175/#185）+ 全部 re-export |
| `providers/bedrock.rs` | 1299 | Amazon Bedrock Converse Streaming 适配 |
| `providers/openai_compat.rs` | 1278 | 通用 OpenAI 兼容适配器 + `ProviderQuirks` |
| `providers/copilot.rs` | 1257 | GitHub Copilot 适配 |
| `providers/google.rs` | 1255 | Gemini API 适配 |
| `providers/openai.rs` | 1093 | OpenAI Chat Completions 适配 |
| `providers/codex.rs` | 960 | OpenAI Codex（Responses API）适配 |
| `variants.rs` | 719 | effort 阶梯推导（opencode 移植），见 §7 |
| `registry.rs` | 699 | `ProviderRegistry` + 工厂函数，见 §7 |
| `providers/cohere.rs` | 693 | Cohere v2 chat 适配 |
| `providers/free.rs` | 669 | `FreeProvider` 免费回退链 |
| `providers/openai_compat_providers.rs` | 633 | 35 家兼容厂商工厂 |
| `provider_types.rs` | 600 | 统一类型层，见 §3A |
| `providers/minimax.rs` | 561 | MiniMax（Anthropic 兼容协议）适配 |
| `protocol/openai_chat.rs` | 494 | sans-IO OpenAI Chat SSE 解码器 |
| `providers/azure.rs` | 482 | Azure OpenAI 适配 |
| `effort_support.rs` | 364 | effort 阶梯注册表感知入口 |
| `error_handling.rs` | 336 | Provider 感知错误处理 |
| `providers/anthropic.rs` | 334 | `AnthropicProvider`（包装 `AnthropicClient`） |
| `stream_parser.rs` | 316 | 流解析抽象 + `SseByteDecoder` |
| `transformers/anthropic.rs` | 246 | Anthropic 线格式转换器 |
| `codex_adapter.rs` | 245 | Codex schema 适配器 |
| `provider_error.rs` | 210 | `ProviderError` |
| `providers/request_options.rs` | 185 | `provider_options` JSON 合并 |
| `providers/message_normalization.rs` | 151 | 消息归一化辅助 |
| `provider.rs` | 122 | `ModelInfo` + `LlmProvider` trait |
| `transformers/openai_chat.rs` | 68 | OpenAI Chat 转换器 |
| `auth.rs` | 58 | `AuthProvider` trait |
| `transform.rs` | 52 | `MessageTransformer` trait |
| `protocol/mod.rs` | 46 | `LineStreamDecoder` trait |
| `bun_tls.rs` | 44 | Bun TLS 指纹伪装 |
| `providers/mod.rs` / `transformers/mod.rs` | 43/11 | 模块声明 + re-export |

### 5.1 `lib.rs` 内联遗留模块

**`types` 模块**（Anthropic 线格式类型，仍是 Codex 适配器与 `AnthropicProvider` 的请求载体）：
- `CreateMessageRequest`（+ `CreateMessageRequestBuilder`：`messages/add_message/system/system_text/tools/temperature/top_p/top_k/stop_sequences/thinking` 链式构造）
- `ThinkingConfig`（`enabled(budget)` / `adaptive()`）
- `SystemPrompt`（字符串或 `Vec<SystemBlock>`，`SystemBlock` 含 `CacheControl::ephemeral()` prompt 缓存标记）
- `ApiMessage`、`ApiToolDefinition`、`CreateMessageResponse`、`ApiErrorResponse`/`ApiErrorDetail`

**`streaming` 模块**：
- `AnthropicStreamEvent` 枚举：MessageStart / ContentBlockStart / ContentBlockDelta / ContentBlockStop / MessageDelta / MessageStop / Error / Ping——**TUI 消费流的事实标准事件类型**
- `ContentDelta`：text_delta / input_json_delta / thinking_delta / signature_delta
- `StreamHandler` trait + `NullStreamHandler`；`SseLineParser`/`SseFrame`（行 → SSE 帧）
- **`StreamAccumulator`**：`on_event(&AnthropicStreamEvent)` 逐事件累积，`finish() -> (Message, UsageInfo, Option<String>)`

**`client` 模块**：
- `Provider` 枚举（Anthropic / Codex）；`ClientConfig`（api key、base URL、OAuth 标记等）
- **`AnthropicClient`**：持有 `wreq::Client`（Bun TLS 指纹），方法 `new/from_config/api_key_is_empty/is_oauth/create_message/create_message_stream/fetch_available_models`；429/529 指数退避重试、每客户端稳定 session-id、OAuth Bearer 支持
- `AvailableModel`：`fetch_available_models` 返回的模型条目

**进程级超时**：`set_request_timeout_secs` / `request_timeout_secs` / `request_timeout` / `stream_idle_timeout`（全局 OnceLock，#175/#185）。

### 5.2 `bun_tls.rs` 与 `codex_adapter.rs`

- `bun_tls.rs`：用 `wreq` 复刻官方 Claude Code 客户端（Bun）的 TLS ClientHello 指纹——17 个 BoringSSL 密码套件、ALPN 仅 http/1.1、目标 JA4 `t13d1714h1_5b57614c22b0_7baf387fc6ff`（在 tls.peet.ws 上验证过）。函数：`bun_tls_options()`、`build_anthropic_client(timeout)`。
- `codex_adapter.rs`：`CODEX_RESPONSES_ENDPOINT` 常量（`https://chatgpt.com/backend-api/codex/responses`）；`anthropic_to_openai_request()` / `parse_openai_response()` / `build_anthropic_response()` 三个方向的 schema 互转。

## 6. `registry.rs`（699 行）— Provider 注册与工厂

### `ProviderRegistry`

```rust
pub struct ProviderRegistry { /* HashMap<ProviderId, Arc<dyn LlmProvider>> + 默认 provider */ }
```

方法：
- `new()` / `register(Arc<dyn LlmProvider>)` / `get(&ProviderId)`
- `set_default(&ProviderId)` / `default_provider()` / `default_provider_id()` / `provider_ids()`
- `check_all_health() -> Vec<(ProviderId, ProviderStatus)>`：并发健康检查
- 构造辅助：`with_anthropic(config)`、`from_config(&Config)`、`from_environment(anthropic_config)`、`from_environment_with_auth_store(...)`（从 AuthStore key 组装可用 provider）
- **按需注册链**（key 已配置才注册）：`with_google_if_key_set` / `with_openai_if_key_set` / `with_azure_if_configured` / `with_bedrock_if_configured` / `with_copilot_if_configured` / `with_codex_if_configured` / `with_cohere_if_key_set` / `with_available_providers`

### 工厂函数

| 函数 | 说明 |
|---|---|
| `provider_from_config(...)` | 按 provider-id 从 auth-store 的 key 构造实例（Codex 从存储 token 文件加载；free 特殊双 key 处理） |
| `build_free_provider()` | 遍历 `FREE_CATALOG` 从 auth store 取 key 组装免费回退链 |
| `runtime_provider_for(provider_id)` | 运行时 provider 查找 |
| `resolve_provider_api_base(base)` | base URL 归一化（`/v1` 后缀处理） |

## 7. `model_registry.rs`（2114 行，Phase 3）— 模型目录

**三层兜底数据源**：编译期嵌入 models.dev 快照（`include_bytes!("../assets/models-snapshot.json")`，约 118 provider / 4500 model）→ 启动时 `load_cache()` 可选磁盘缓存覆盖（`with_cache_path`）→ `refresh_from_models_dev()` 从 `https://models.dev/api.json` 刷新（URL 可用 `MODELS_DEV_URL`/`CLAURST_MODELS_DEV_URL` 覆盖）。**不再硬编码模型列表**；所有网络/解析失败均非致命（快照兜底）。

### 类型

| 类型 | 说明 |
|---|---|
| `Modality` | 输入/输出模态标记 |
| `ModelStatus` | 模型生命周期（含 `is_listed_by_default()`——beta/过期模型默认不在 picker 显示） |
| `InterleavedReasoning` | 交错推理能力 |
| `ProviderOverride` / `ModelOverride` | 用户模型元数据覆盖（issue #309） |
| `ExperimentalMode` | 实验模式标记 |
| `CostTierCondition` / `CostTier` / `CostBreakdown` | 成本分层定价（按条件匹配的每百万 token 单价） |
| `ProviderEntry` | provider 元数据（npm 包名、api 字段、排序等） |
| `ModelEntry` | 模型条目：info + 辅助查询 `vision()` / `audio_input()` / `pdf_input()` / `video_input()` |

### `ModelRegistry` 方法

- 查询：`get(provider, model)`、`resolve("provider/model") -> (ProviderId, ModelId)`、`find_provider_for_model(model_name)`、`list_by_provider` / `list_visible_by_provider`、`best_model_for_provider` / `best_small_model_for_provider`（fast mode 用）、`list_all`、`len/is_empty`
- provider 级：`provider(id)`、`list_providers()`、`provider_count()`
- 数据源：`refresh_from_models_dev()`、`load_cache(path)`、`apply_model_overrides(&config.model_overrides)`
- `effective_model_for_config(...)`：按配置解析最终模型 id（含各 provider 默认回退）

## 8. effort 阶梯（`effort_support.rs` + `variants.rs`）

- **`effort_support.rs`**（#267）：`/effort` 命令与 `/model` picker 内联选择器共同走这里。
  - `variant_ladder(provider, model, Option<&ModelRegistry>)` —— 返回该模型的 effort 阶梯
  - `supported_efforts(...)` —— 在阶梯基础上追加 claurst 的 `Ultracode` 档
  - `model_is_reasoning(provider, model, registry)` —— 是否为推理模型
- **`variants.rs`**（#268）：opencode `ProviderTransform.variants()` 的逐分支忠实移植——从 npm 包名 / 模型 id / `release_date` / provider id / reasoning 能力推导推理档位（none/minimal/low/medium/high/xhigh/max）。
  - OpenAI 日期门控常量：`OPENAI_NONE_EFFORT_RELEASE_DATE = "2025-11-13"`、`OPENAI_XHIGH_EFFORT_RELEASE_DATE = "2025-12-04"`（早于/晚于该日期发布的模型支持的档位不同）
  - glm / minimax 特例；核心函数 `variant_efforts(...)`

## 9. `error_handling.rs`（336 行，Phase 6）— Provider 感知错误处理

- `is_context_overflow(message)`：**29+ 条**各主流厂商上下文溢出错误模式表（正则匹配错误文案）
- `parse_error_response(status, body, provider) -> ProviderError`：HTTP status + body → 正确的错误变体
- `RetryConfig`：带 jitter 的指数退避，`delay_for_attempt(attempt) -> Duration`

## 10. `providers/` 子目录（各支持 Provider）

| 文件 | Provider / 说明 |
|---|---|
| `anthropic.rs` | `AnthropicProvider`：把 `AnthropicClient` 包进 `LlmProvider`（ProviderRequest → CreateMessageRequest，AnthropicStreamEvent → StreamEvent） |
| `openai.rs` | `OpenAiProvider`：Chat Completions（`/v1/chat/completions`），适用于任何 OpenAI 兼容端点；支持工具调用、`GET /v1/models`、健康检查、流式 SSE；对外暴露公开转换助手（`to_openai_messages_pub` 等）供其他适配器复用 |
| `google.rs` | `GoogleProvider`：Gemini API（generativelanguage.googleapis.com）；generateContent / streamGenerateContent?alt=sse、functionDeclarations 工具调用、systemInstruction、Gemini 2.5+/3.0+ thinking、inlineData 图像/视频输入 |
| `minimax.rs` | Anthropic 兼容协议的 MiniMax 适配（`X-Api-Key` 头，非 Bearer）；base 可用 `MINIMAX_BASE_URL` 覆盖 |
| `openai_compat.rs` | `OpenAiCompatProvider` 通用可配置适配器（builder 设置 base/auth/headers）+ **`ProviderQuirks`**（行为怪癖：`tool_id_max_len`——如 Mistral 限 9 字符、纯字母数字 tool id、overflow 错误模式、`stream_options.include_usage`） |
| `openai_compat_providers.rs` | **35 家 OpenAI 兼容厂商工厂**（`provider_for_id` 分发；key 缺省时 provider 仍可构建但 health_check 返回 Unavailable）：ollama、lm_studio、llama_cpp、deepseek、groq、xai、deepinfra、cerebras、together_ai、perplexity、venice、qwen、mistral、openrouter、sambanova、huggingface、nvidia、siliconflow、moonshot、zhipu、zai、nebius、novita、ovhcloud、scaleway、vultr_ai、baseten、crof、friendli、upstage、stepfun、fireworks、opencode_go、opencode_zen、synthetic |
| `free.rs` | `FreeProvider`：组合式"免费"provider——把多个免费档上游堆叠在合成模型 `free/auto` 后。未流式前失败即换下一个上游重试；流中失败原样抛出。路由规则：`free`/`free/auto`/`auto` 按目录顺序轮询；`<upstream_id>/<rest>` 钉住指定上游。**`FREE_CATALOG`** 当前 12 个上游：groq、cerebras、google、mistral、sambanova、nvidia、cohere、openrouter、opencode-zen、zai、zhipuai 等（理念源自 freellmapi 项目） |
| `cohere.rs` | Cohere v2 chat API（Command R/R+），类 OpenAI 消息数组但自有流式事件信封 |
| `azure.rs` | Azure OpenAI：同 Chat Completions 线格式但 URL 结构不同（`{resource}.openai.azure.com/openai/deployments/{deployment}/chat/completions?api-version=`）、认证用 `api-key` 头（非 Bearer）、deployment==model |
| `bedrock.rs` | Amazon Bedrock Converse Streaming API（`bedrock-runtime.{region}.amazonaws.com/model/{id}/converse-stream`）；双认证：`AWS_BEARER_TOKEN_BEDROCK` 或 AWS SigV4；官方仅支持 Bedrock 上的 Claude 模型 |
| `copilot.rs` | GitHub Copilot（api.githubcopilot.com）；特殊头（`Openai-Intent: conversation-edits`、`x-initiator`）；GPT-5 级模型走 Responses API、其余走 Chat Completions（与 OpenCode 同规则）；`GITHUB_TOKEN` 环境变量 |
| `codex.rs` | OpenAI Codex（`chatgpt.com/backend-api/codex/responses`，Responses API），OAuth Bearer（token 存 `~/.claurst/codex_tokens.json`，过期自动 refresh_token）；无 /models 路由，模型列表用 core 的 `CODEX_MODELS` 静态常量 |
| `message_normalization.rs` | 内部辅助：`remove_empty_messages`、`normalize_anthropic_messages`、`scrub_tool_ids`（Anthropic tool id 清洗） |
| `request_options.rs` | `provider_options` JSON 合并：`merge_root_options`（递归根合并）、`merge_openai_compatible_options`（`reasoningEffort→reasoning_effort`、`textVerbosity→verbosity` 键名转换）、`merge_google_options`/`merge_bedrock_options` |

## 11. `transformers/` 子目录（Phase 4）

- `anthropic.rs`（246 行）：**`AnthropicTransformer`** —— Anthropic 线格式的"恒等"转换器（ProviderRequest → Anthropic v1 messages JSON、响应解析回 ProviderResponse；复用 `normalize_anthropic_messages`，纯 JSON↔类型映射，不拥有 HTTP 客户端）。
- `openai_chat.rs`（68 行）：**`OpenAiChatTransformer`** —— 产出 OpenAI Chat Completions JSON；把消息/工具转换委托给 `OpenAiProvider` 的公开助手方法，格式逻辑单一来源。

## 12. 关键公开 API 汇总

| 类别 | 条目 |
|---|---|
| **Trait** | `LlmProvider`、`AuthProvider`、`StreamParser`、`MessageTransformer`、`LineStreamDecoder`、`StreamHandler` |
| **客户端/注册表** | `AnthropicClient`、`ClientConfig`、`ProviderRegistry`、`ModelRegistry`、`ModelEntry`、`ProviderEntry`、`ModelInfo`、`AvailableModel` |
| **请求/响应** | `ProviderRequest`、`ProviderResponse`、`CreateMessageRequest`（+Builder）、`CreateMessageResponse`、`ThinkingConfig`、`SystemPrompt`、`ApiMessage`、`ApiToolDefinition`、`StreamAccumulator` |
| **流** | `StreamEvent`、`AnthropicStreamEvent`、`ContentDelta`、`SseByteDecoder`、`SseLineParser`/`SseFrame`、`OpenAiChatDecoder`、`StreamBlockAccumulator` |
| **错误** | `ProviderError`（10 变体）、`RetryConfig`、`parse_error_response`、`is_context_overflow` |
| **枚举** | `StopReason`、`ProviderCapabilities`、`SystemPromptStyle`、`ProviderStatus`、`AuthMethod`、`ApiKeyHeader`、`Modality`、`ModelStatus` |
| **effort** | `variant_ladder`、`supported_efforts`、`model_is_reasoning`、`variant_efforts` |
| **资源** | `assets/models-snapshot.json`（models.dev 编译期快照） |

## 13. 设计要点

1. **事件类型单一事实标准**：`AnthropicStreamEvent` 是 TUI 消费流的事实标准；query 层（runner/stream.rs）负责把所有 provider 的 `StreamEvent` 归一到它，TUI 无感。
2. **厂商塌缩为配置**：trait + protocol 分层后，一家厂商 = 端点 + 认证方式 + 线格式协议 + 模型来源四项配置；`ProviderQuirks` 吸收残余的行为差异。
3. **模型列表不硬编码**：models.dev 快照编译期嵌入 + 磁盘缓存 + 网络刷新三层兜底，用户元数据覆盖（#309）最后叠加。
4. **sans-IO 协议层**：`LineStreamDecoder` 与网络解耦，可脱离真实端点单测；`SseByteDecoder` 解决 UTF-8 跨 chunk 撕裂（#228）。
5. **TLS 指纹伪装**：Anthropic 通道使用 BoringSSL 指纹模拟官方客户端（JA4 校验），`free` 等合成 provider 提供无 key 回退链。
6. **secure/fallback 语义**：`discover_models` 默认空、目录型 provider 不实现；`check_all_health` 并发探测；限流/过载切换 `fallback_model` 的决策留给 query 层。

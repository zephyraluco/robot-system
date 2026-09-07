# `claurst`（crates/cli）— 主可执行程序

> 路径：`crates/cli` ｜ package name：`claurst` ｜ 依赖图顶端节点，依赖全部 9 个库 crate ｜ 产物：`claurst`（4,899 行）、`rsctl`（71 行）、`rsvm`（344 行）三个二进制 + 3 个库模块（oauth_flow 545 行、codex_oauth_flow 296 行、upgrade 591 行）

## 1. 定位

整个 workspace 的主可执行程序 crate，把所有库 crate 组装成最终 CLI 产品——命令行参数解析、配置装载、MCP 工具接入、headless 单次查询、交互式 TUI REPL、OAuth 登录、自升级，以及独立小工具 `rsctl`/`rsvm`。

## 2. 依赖

- **内部**（几乎全量）：core、api、tools、query、tui、commands、mcp、bridge、plugins。
- **关键外部**：`clap` + `clap_complete`、`tokio`/`tokio-util`、`reqwest`、`crossterm`（原始按键）、`tracing-subscriber`、`base64`/`sha2`（PKCE）、`open`、`xxhash-rust`（模型缓存键）；Unix 下 `nix`（禁止 root 使用 `--dangerously-skip-permissions`）。
- **build-dependencies**：`chrono`。

## 3. build.rs

编译期注入构建元数据环境变量（`bin/claurst.rs` 用 `env!()` 读取）：
`BUILD_TIME`（UTC RFC 3339）、`GIT_COMMIT`（`git rev-parse --short HEAD`）、`PACKAGE_URL`、`FEEDBACK_CHANNEL`、`ISSUES_EXPLAINER`；声明 `rerun-if-changed=.git/HEAD`。

## 4. 三个可执行程序（src/bin/）

### (1) `claurst.rs`（4899 行，主程序）

入口 `main()` 采用**多级 fast-path + clap 兜底**结构：

1. `--version` fast-path
2. `claurst auth <login|logout|status>`（Anthropic OAuth/账号管理，含多账号 label 切换）
3. `claurst codex <login|logout|list|switch|remove>`（OpenAI Codex 账号管理，与 auth 对称）
4. `claurst accounts`（跨 provider 列出所有存储账号）
5. `claurst upgrade`（自升级）
6. `claurst models [provider] [--refresh] [--json]`（列出快照+缓存模型，含 TTL 缓存刷新机制）
7. **named commands fast-path**：在 `Cli::parse()` 之前拦截第一个位置参数查 `find_named_command()`，构建 pre-session `CommandContext` 执行后直接退出
8. **clap `Cli` 解析**（40+ 选项）：positional prompt、`-p/--print`、`-m/--model`、`--permission-mode`、`--resume`/`--continue`、`--output-format text|json|stream-json`、`--api-key`、`--dangerously-skip-permissions`（别名 `yolo`，root 下拒绝）、`--mcp-config`、`--trust-project-mcp`、`--auto-commits`、`--effort`、`--allowed-tools/--disallowed-tools`、`--max-budget-usd`、`--fallback-model`、`--provider`、`--api-base`、`-A/--agent` 等
9. **日志初始化**：EnvFilter，压制 rmcp/free provider/query 噪音
10. **配置装载**：`Settings::load_hierarchical(&cwd)` 层级合并，CLI 参数覆盖进 `Config`
11. **MCP**：`McpToolWrapper` 把 MCP 工具包装成原生 `claurst_tools::Tool`（权限级别视为 Execute）；`filter_tools_for_agent()` 按 agent 过滤
12. **两条主运行路径**：
    - **`run_headless()`**：单次查询输出到 stdout，支持 stream-json 输入/输出、预算、fallback model
    - **`run_interactive()`**（约 2500 行）：完整 TUI REPL——slash 命令执行（调 `claurst_commands::execute_command`）、TUI overlay 拦截（hooks/import-config/rewind）、provider 运行时重建（`refresh_provider_runtime_state`）、bridge 配置解析、`handle_exit_key`（Ctrl+C + 取消 token）

### (2) `rsctl.rs`（71 行，独立工具）

`rsctl status|restart|stop|start <PKG>` 直接转发 `systemctl <action> <pkg>.service`；`completions` 生成补全脚本。与 workspace 内其他库无耦合。

### (3) `rsvm.rs`（344 行，独立工具）

读取 `/etc/robot-system/robot-system.conf`（TOML）的包管理器：`install`（FTP 下载安装包）、`list`（对比配置与 `dpkg-query` 已装版本，输出 OK/MISMATCH/None）、`history`/`info`、`reload`（symlink 服务文件进 systemd 并 daemon-reload）、`init`（无 SUDO_USER 时自动 sudo 重执行）、`completions`。

## 5. 库模块（src/lib.rs 及 3 个模块）

- **`oauth_flow.rs`（545 行）— Anthropic OAuth 2.0 PKCE 登录**
  - 复用 Claude Code 客户端 ID（"impersonate Claude Code"，配合 `AnthropicClient::apply_oauth_stealth`）。
  - 流程：PKCE verifier/challenge/state → 本地随机端口 TCP 监听 → 授权 URL + 开浏览器 → 等待回调（60s 超时或终端手贴 code）→ 换 token → Console 流程再创建 API key → 存 `~/.claurst/oauth_tokens.json`。
  - 公开 API：`run_oauth_login_flow(_with_label/_tui)`（TUI 版经 `mpsc::Sender<DeviceAuthEvent>` 推进度）、`refresh_oauth_token()`。
- **`codex_oauth_flow.rs`（296 行）— OpenAI Codex（ChatGPT）OAuth PKCE 流程**
  - 公开 API：`generate_code_verifier/compute_code_challenge/generate_state/build_auth_url/run_oauth_flow(_with_label)`；产出 `CodexTokens`。
- **`upgrade.rs`（591 行）— `claurst upgrade` 自更新**
  - 从 GitHub releases 下载（`--version`/`--force`），shell 出 `tar`/PowerShell 解压（刻意不引入 flate2/tar/zip 依赖），`swap_binary()` **原子替换正在运行的二进制**，macOS 清 quarantine。
- **`system_prompt.txt`（38 行）— 内嵌默认系统提示词**
  - 核心原则、可用工具清单、工作流指引；`--dump-system-prompt` 可导出，`--system-prompt-file` 可覆盖。

## 6. 设计要点

1. **装配层**：依赖全部 9 个库 crate，无任何 crate 反向依赖它；`rsctl`/`rsvm` 独立实现、不依赖 claurst_* 库。
2. **fast-path 优先**：版本、auth、named command 等高频路径在 clap 全量解析前拦截，构建 pre-session 上下文快速执行。
3. **命令执行统一在 CLI 层**：TUI 的 `App::run` 提供输入/渲染，REPL 主循环驱动 `run_query_loop` 与 `execute_command`。
4. **"命令语义在 commands，程序组装在 cli"**：与 `claurst-commands` 形成明确分工。

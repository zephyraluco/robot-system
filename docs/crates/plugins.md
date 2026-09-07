# `claurst-plugins` — 插件运行时

> 路径：`crates/plugins` ｜ 约 **2,649 行 / 7 个源文件** ｜ 依赖：`claurst-core`（单向）｜ 被 cli、commands、query 依赖；与 `claurst-mcp` 互不依赖

## 1. 定位

插件运行时（Cargo description: *"Plugin runtime for the Claurst CLI"*）：插件发现、manifest 解析、hook 注册、命令/agent/技能/MCP/LSP 资源挂载、能力（capability）强制、marketplace 安装管理，以及 `/plugin` + `/reload-plugins` 命令支撑。lib.rs 头部明确依赖序：**cc-plugins → cc-core 单向依赖；cc-commands → cc-plugins（不得反向）**。

## 2. 依赖

- **关键外部**：`tokio`/`async-trait`/`futures`、`walkdir`（目录递归）、`reqwest`（marketplace 下载）、`sha2`+`hex`（下载 hash 校验）、**`zip = 2`**（插件压缩包解压）。

## 3. 文件清单与职责

### lib.rs
- **能力强制（capability enforcement）**：`check_plugin_capability(def)` —— 按动作所需能力判定；manifest 无 `capabilities` 字段 = 旧式插件全量信任（向后兼容），有字段（即使为空列表）则必须显式授予，否则返回错误。
- **进程级全局注册表**：`GLOBAL_PLUGIN_REGISTRY` / `GLOBAL_HOOK_REGISTRY`（`OnceLock`），`set_global_registry()`/`set_global_hooks()` 及全局 pre/post tool hook 执行入口——使 slash command 与 tool 无需逐层传参即可访问插件系统。

### manifest.rs
`plugin.json`/`plugin.toml` 清单类型 + 标准插件目录布局（`commands/ agents/ skills/ hooks/ output-styles/ .mcp.json`）：
- `PluginMcpServer`（插件内联 MCP server 声明）、`PluginLspServer`（含 extension→language 映射、崩溃重启策略）。
- **`HookEventKind` — 27 种生命周期事件**：PreToolUse、PostToolUse(±Failure)、PermissionDenied、Notification、UserPromptSubmit、SessionStart/End、Stop(±Failure)、SubagentStart/Stop、PreCompact/PostCompact、PermissionRequest、Setup、TeammateIdle、TaskCreated/Completed、**Elicitation/ElicitationResult**、ConfigChange、WorktreeCreate/Remove、InstructionsLoaded、CwdChanged、FileChanged 等。
- `PluginHooksConfig`（hooks/hooks.json 或 manifest 内联）、`PluginUserConfigOption`（插件用户配置项声明）。
- **`PluginManifest`** — 主清单：name/version/author/commands/agents/skills/mcp_servers/lsp_servers/hooks/user_config/marketplace_id/**capabilities**。

### loader.rs
- `default_user_plugins_dir()` → `<claurst home>/plugins`；`project_plugins_dir()` → `<project>/.claurst/plugins`。
- **扫描优先序**：用户全局 → 项目本地 → 额外路径（`--plugin-dir`）。
- `discover_plugins(search_dirs, source)`：深度 1 遍历，子目录或裸 manifest 文件均为候选，返回 `(Vec<LoadedPlugin>, Vec<PluginError>)`——坏插件不阻断其余加载。

### plugin.rs
- **`PluginSource`**：User / Project / Extra / Inline（SDK 程序化注入）/ Builtin，带 `label()`。
- **`LoadedPlugin`**：name/path/source/`source_id`（"name@source"）/manifest/enabled/资源子目录路径/hooks_config。
- **`CommandRunAction`**：插件命令可执行动作（静态响应 / 运行 shell 等），`required_capability()` 供能力强制检查。
- `PluginCommandDef`（注册为斜杠命令的定义）、`PluginError`、`ReloadDiff`（/reload-plugins 的 added/removed/updated 对比）。

### registry.rs
**`PluginRegistry`** — 会话内插件中央存储：`insert()` 同名不同路径报 DuplicateName 且**先到先得**（与 TS 一致）；`enabled()` 只回已启用插件、`all()` 含禁用；启停由 settings 的 `enabled_plugins/disabled_plugins` 驱动。

### hooks.rs
- **`RegisteredHook`**（带 plugin 上下文）+ **`HookRegistry = HashMap<事件名, Vec<RegisteredHook>>`**。
- `register_plugin_hooks()` 展开合并 hooks 配置；同步 dispatch：JSON `HookContext` 经 stdin 执行 shell 命令，blocking hook 非零退出即阻断操作（与 core 用户钩子机制同构，但带插件元数据与来源隔离）。

### marketplace.rs
插件市场（registry：`https://registry.claude.ai/plugins`）：
- `MarketplaceEntry`（含 download_url 与 **hash**）、`InstalledPlugin`。
- `marketplace_search(_filtered)`、`marketplace_install`（zip 解压 + sha2 校验）、`marketplace_update`、`marketplace_check_updates_all`、`list_installed`、`marketplace_uninstall`。

## 4. 关键公开 API（已验证）

| 类别 | 条目 |
|---|---|
| **能力强制** | `check_plugin_capability(def) -> Result<(), String>` |
| **全局注册表** | `set_global_registry()` / `global_plugin_registry()`、`set_global_hooks()`、`run_global_pre_tool_hook()` / `run_global_post_tool_hook()` |
| **命令解析** | `PluginSubCommand`（枚举）、`parse_plugin_args()`、`format_plugin_list()`、`format_plugin_info()`、`format_reload_summary()` |
| **安装** | `install_plugin_from_path()` |
| **运行时类型** | `PluginSource`（User/Project/Extra/Inline/Builtin，`label()`）、`LoadedPlugin`、`CommandRunAction`（`required_capability()`）、`PluginCommandDef`、`PluginError`（`message()`）、`ReloadDiff` |
| **注册表** | `PluginRegistry`（insert/extend/enabled/all） |
| **hooks** | `RegisteredHook`、`HookRegistry`、`register_plugin_hooks()` |
| **发现** | `discover_plugins()`、`default_user_plugins_dir()`、`project_plugins_dir()` |
| **manifest** | `PluginManifest`、`HookEventKind`（27 种）、`PluginHooksConfig`、`PluginMcpServer`、`PluginLspServer`、`PluginUserConfigOption` |
| **marketplace** | `marketplace_search(_filtered)`、`marketplace_install`、`marketplace_update`、`marketplace_check_updates_all`、`list_installed`、`marketplace_uninstall` |

## 5. 设计要点

1. **单向依赖纪律**：plugins → core，commands → plugins，禁止反向。
2. **安全模型**：能力强制（capability）+ 旧式全量信任兼容；插件声明的 MCP server 经 core `McpServerOrigin::User` 标记（插件启用即视为受信）后交由 mcp crate 连接。
3. **坏插件隔离**：发现/加载阶段错误收集为 `PluginError`，不阻断其他插件。
4. **hook 机制与 core 同构**：stdin JSON + 退出码语义，额外携带插件来源元数据。

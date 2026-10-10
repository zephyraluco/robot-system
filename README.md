# robot-system

![Rust](https://img.shields.io/badge/Rust-2024%20edition-orange?logo=rust)
![Platform](https://img.shields.io/badge/platform-Ubuntu%2024.04-E95420?logo=ubuntu)
![License](https://img.shields.io/badge/license-MIT-blue)

Ubuntu 24.04 上的自定义软件包与运行状态管理系统，Rust 实现。

本系统复用 dpkg / APT、systemd、journald 等系统原生能力，在其上补齐**部署编排、
软件包与服务关联、运行实例追踪、资源指标与异常事件记录**。

> 定位：这是“自定义软件包部署与运行管理器”，**不是**重写一套操作系统包管理器。
> dpkg / APT 负责软件包安装能力，systemd 负责服务生命周期，journald 负责原始日志；
> 自研部分聚焦编排、关联、追踪与恢复。

## ✨ 特性

- **复用原生设施**：dpkg / APT 负责安装能力，systemd 负责服务生命周期，journald 负责
  原始日志；自研部分只做编排、关联、追踪与恢复。
- **职责分离**：`rsctl` 按需执行变更，常驻服务只采集与记录；二者不共享 Rust 库、无 IPC，
  一方不可用时另一方不受影响。
- **可追踪**：运行实例以 `run_id + pid + /proc/<pid>/stat 启动时钟值` 标识，免疫 PID 复用。
- **不伪造数据**：无法确认退出原因记 `unknown`；部署失败区分「已回滚」与「需人工处理」，
  退出码只在真正成功时为 0。
- **可部署**：支持构建 `amd64` / `arm64` 的 Debian 包，随包安装统一 target 与常驻服务。

---

## 🧩 1. 组成

系统由**两个相互独立的程序**组成（见架构文档 §1）。二者不共享 Rust 库、不通过 IPC
通信，唯一的耦合点是共享的 SQLite 数据库结构以及 systemd、journald、`/proc` 等系统设施。

| 程序 | 运行方式 | 职责 |
|---|---|---|
| `rsctl` | 前台按需执行 | 软件包管理、部署编排、服务启停、安装校验，以及运行状态的**只读**查询 |
| `robot-system-daemon` | 常驻服务 | 进程监听、资源指标采集、日志整理、错误事件记录，HTTP 请求任务入口（**占位**） |

> **文档指引**：本文面向使用者，介绍组成、安装、CLI 与数据；实现架构（模块分解、
> 关键机制、时序图、设计权衡与已知限制）见 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)。
> 下文出现的「架构文档 §X.Y」均指该文档。

两个程序都**不持有业务服务的 unit 文件**：业务服务由各自的 DEB 提供，经
`WantedBy=robot-system.target` 归属，受管集合由 systemd 的依赖关系决定，而不是由本项目
维护一份清单副本。

- 数据库只有常驻服务**写入**，`rsctl` 只读，无写入竞争。
- 变更类操作只由 `rsctl` 在 root 下执行；常驻服务不可用时其操作不受影响。

### 代码位置速查

各关注点的模块路径、关键类型与文件系统布局见[架构文档](docs/ARCHITECTURE.md)附录 A。

---

## 🚀 2. 快速开始

### 2.1 构建与测试

```bash
cargo build --release --locked
cargo test                     # 112 个单元测试
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

### 2.2 打包与安装

```bash
cargo build --workspace --release --locked
sudo apt-get install -y fakeroot
./scripts/make_deb.sh 0.1.0 /opt/robot-system
sudo apt-get install -y ./dist/robot-system_0.1.0_amd64.deb
```

`make_deb.sh` 使用已有的 release 二进制生成 DEB（需 `dpkg-deb`、`fakeroot`，无需 root），
产物写入 `dist/`，架构由构建主机自动识别。GitHub Release 会在 `v*` 标签推送后，为
`amd64` 和 `arm64` 分别构建并附加 DEB 文件。

### 2.3 本地验证（不安装到系统）

两个程序都支持用 `ROBOT_SYSTEM_PREFIX` 重定向全部路径：

```bash
export ROBOT_SYSTEM_PREFIX=/tmp/rs-root
mkdir -p "$ROBOT_SYSTEM_PREFIX/opt/robot-system/migrations" \
         "$ROBOT_SYSTEM_PREFIX/opt/robot-system/config"
cp migrations/*.sql "$ROBOT_SYSTEM_PREFIX/opt/robot-system/migrations/"
cp etc/config/robot-system.conf \
   "$ROBOT_SYSTEM_PREFIX/opt/robot-system/config/"   # 可选：改配置就编辑这份

./target/release/robot-system-daemon               # 启动常驻服务（Ctrl+C 退出）
./target/release/rsctl system status                # 另开终端查询系统管理状态

# 不拷配置文件也可以：两个程序在配置缺失时使用同样的默认值（可用键见架构文档 §5.5）。
# 需要观察软件包归属显示时，自行在 packages/ 下放一份服务清单（格式见同一节）。
```

---

## 🛠️ 3. 使用

### `rsctl`

全局选项：`--json`（机器可读输出）、`--opt-root <DIR>`（覆盖程序根目录）。

```bash
# --- 软件包与部署任务（变更类操作需要 sudo）---
rsctl package list
rsctl package info robot-lidar
sudo rsctl package install ./robot-lidar_1.2.0_amd64.deb
sudo rsctl package upgrade ./robot-lidar_1.3.0_amd64.deb
sudo rsctl package remove robot-lidar
rsctl package task list
rsctl package task show <task-id>
rsctl package history robot-lidar

# --- 统一 target 与服务 ---
rsctl system status
rsctl service list                 # 清单声明的受管服务
rsctl service list --all           # 系统中全部 systemd service 单元
rsctl service status robot-lidar.service
sudo rsctl service start|stop|restart|enable|disable robot-lidar.service

# --- 运行状态（只读）---
rsctl process list
rsctl process history robot-lidar.service
rsctl metrics robot-lidar.service
rsctl error list [--event-type <type>] [--service <unit>]
rsctl logs robot-lidar.service --since 10m --priority err
```

**退出码约定**：部署任务只有在 `committed` 时才视为成功。失败但已回滚（`recovered`）
或需要人工恢复（`recovery_required`）都会返回非零退出码，避免脚本把失败当成成功。

### `robot-system-daemon`

由 systemd 启动的常驻后台进程，**不接受任何命令行参数**（在架构文档 §1 中被定义为只做
采集与记录的服务）。路径由配置文件与 `ROBOT_SYSTEM_PREFIX` 环境变量决定，后者仅用于
本地验证时重定向全部路径（`opt/robot-system`、`var/lib/robot-system`、
`run/robot-system`、`var/log/robot-system`）。

程序版本在启动日志中输出；日志标识由 systemd unit 的 `SyslogIdentifier` 提供，与业务
服务日志区分开（见架构文档 §4.4）。

---

## 💾 4. 状态与数据

### 4.1 运行数据库（`/var/lib/robot-system/state.db`）

只保存常驻服务采集的运行数据：`process_runs`（运行实例：`run_id`/PID/启动时钟值/退出码
/时长）、`process_metrics`（CPU%、内存、线程数）、`error_events`（运行类错误事件）。
**软件包、服务关系与部署任务状态不进入数据库**；表结构与连接参数见架构文档 §5.1。

### 4.2 部署任务文件（`/var/lib/robot-system/transactions/*.json`）

由 `rsctl` 以“临时文件 + 原子 rename”持久化每次状态跃迁与步骤事件；执行新变更前会检查
未完成任务并拒绝继续（见架构文档 §4.5、§5.2）。

### 4.3 状态来源优先级

各事实来源保持最终一致（见架构文档 §8）：软件包以 **dpkg** 为准，受管服务集合与启用/
运行状态以 **systemd** 为准，当前进程与资源取自 **`/proc`**，原始日志取自 **journald**，
运行历史/指标/事件取自 **SQLite**。数据库缺失时 `rsctl` 不报错，而是回退直接查询
systemd / journald / `/proc` 并提示“历史不可用”。

---

## ⚠️ 5. 关键约束

- **不提供业务服务 unit**：业务服务随各自的 DEB 安装，以 `WantedBy=robot-system.target`
  归属；本项目只提供 target 与常驻服务的 unit（见架构文档 §1、§2）。
- **服务清单只用于归属展示**：`/opt/robot-system/packages/*.toml` 缺失也不影响服务管理；
  robot-system 自身不预置任何清单或业务配置，以免出现“幽灵软件包”（见架构文档
  §4.1、§5.5）。
- **包与服务非一一对应**：一个包可含多个服务，一个服务可被多个包声明（共享服务）；
  卸载共享服务时不停用该服务。
- **进程身份不能只用 PID**：运行实例由 `run_id + pid + /proc/<pid>/stat 启动时钟值`
  共同确定，服务重启即生成新 `run_id`（见架构文档 §4.2）。
- **不伪造退出原因**：无法确认退出码/信号时记 `unknown`（见架构文档 §8）。
- **变更锁是应用级的**：`/run/robot-system/lock` 只约束遵守它的 `rsctl` 进程，无法阻止
  直接用 APT / dpkg / `systemctl`；因此变更前后会重新核验实际版本与状态（见架构文档 §4.6）。
- **部署事务 ≠ 可回滚事务**：maintainer scripts 可能产生不可撤销的副作用，无法恢复时
  进入 `recovery_required`，绝不谎报回滚成功（见架构文档 §4.5）。
- **HTTP 入口是占位**：所有请求返回 `501`，不执行任何变更（见架构文档 §10）。

---

## 🗺️ 6. 现状与后续工作

已实现：软件包查询 / 安装 / 升级 / 卸载、部署任务状态机与失败恢复、统一 target 与
服务管理、进程运行实例追踪与资源采集、错误事件记录、日志查询、数据保留策略、
JSON 输出，以及两个程序的完整测试覆盖。

尚未实现（属于后续阶段）：

1. HTTP API 的真实接口与变更执行路径（见架构文档 §10，当前仅占位）；
2. 受控 APT 仓库与签名校验（当前支持本地 DEB 与 `apt-get` 依赖解析，但未实现仓库签名验证）；
3. 使用 systemd D-Bus API 替代 `systemctl` 命令调用；

---

## 🤝 贡献

欢迎提交 Issue 和 Pull Request。开始之前请先：

1. 阅读 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)，了解模块边界与关键机制；
2. 为行为变化补充测试或可复现步骤；
3. 确保 `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、
   `cargo test` 与 `cargo build --release --locked` 全部通过；
4. 在 PR 中说明变更原因及验证方式。

GitHub Actions 会在推送和 Pull Request 时运行格式检查、Clippy、测试与 release 构建；
推送形如 `v0.1.0` 的标签会自动创建 GitHub Release 并附上两个架构的 DEB 包。

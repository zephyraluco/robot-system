# Robot System

[![CI](https://github.com/zephyraluco/robot-system/actions/workflows/ci.yml/badge.svg)](https://github.com/zephyraluco/robot-system/actions/workflows/ci.yml)
[![Release](https://github.com/zephyraluco/robot-system/actions/workflows/release.yml/badge.svg)](https://github.com/zephyraluco/robot-system/actions/workflows/release.yml)
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)

**Robot System** 是一组面向 Linux 机器人设备的轻量级命令行工具，用于查看系统状态、控制运行状态，以及安装、检查和维护机器人软件组件。

它由两个命令组成：

- `rsctl`：控制机器人系统的启动、停止、重启和状态查看。
- `rsvm`：管理机器人系统版本、Debian 软件包和服务组件。

## ✨ 特性

- **简单**：通过清晰的子命令完成日常系统操作。
- **可靠**：使用 Rust 构建，并在 CI 中执行格式检查、Clippy、测试和 release 构建。
- **可维护**：从 Debian 包数据库读取已安装版本，并保留组件安装历史。
- **可部署**：支持构建适用于 `amd64` 和 `arm64` 的 Debian 软件包。
- **易集成**：支持生成 Bash、Zsh、Fish、PowerShell 等 Shell 补全脚本。

## 🚀 安装

### 从 GitHub Releases 安装

在 [Releases](https://github.com/zephyraluco/robot-system/releases) 页面下载与你的设备架构匹配的 `.deb` 文件：

```bash
# x86_64 / amd64
sudo dpkg --install robot-system_<version>_amd64.deb

# ARM64
sudo dpkg --install robot-system_<version>_arm64.deb
```

安装后，`rsctl` 和 `rsvm` 会被放置到 `/usr/bin`。

### 从源码构建

构建需要 Rust 工具链和 Cargo：

```bash
git clone https://github.com/zephyraluco/robot-system.git
cd robot-system
cargo build --release --locked
```

编译产物位于 `target/release/`：

```bash
./target/release/rsctl --help
./target/release/rsvm --help
```

也可以使用项目提供的脚本构建：

```bash
./tools/build.sh
```

### 构建 Debian 软件包

构建 Debian 包需要 `fakeroot` 和 `dpkg`：

```bash
sudo apt-get install --no-install-recommends fakeroot dpkg
cargo build --release --locked
./tools/make_deb.sh
```

默认包会生成在 `dist/`，默认安装前缀为 `/usr`。也可以传入版本号和安装前缀：

```bash
./tools/make_deb.sh 0.1.0 /usr
```

## 🛠️ 使用

### 控制系统

```bash
rsctl status <pkg>
rsctl start <pkg>
rsctl stop <pkg>
rsctl restart <pkg>
```

这些命令会分别调用 `systemctl` 控制对应的 `<pkg>.service` 服务。

查看所有命令：

```bash
rsctl --help
```

### 管理组件

首次使用时，初始化配置：

```bash
sudo rsvm init
```

查看系统及组件状态：

```bash
sudo rsvm list
```

安装指定版本的组件：

```bash
sudo rsvm install <package> <version>
```

查看组件信息和安装历史：

```bash
sudo rsvm info <package>
sudo rsvm history <package>
```

重新扫描服务并更新组件记录：

```bash
sudo rsvm reload
```

查看所有命令：

```bash
rsvm --help
```

> `rsvm install` 会从配置的 FTP 地址下载 `<package>_<version>.deb`，然后调用 `dpkg` 安装。执行安装、重新加载和读取系统配置通常需要 root 权限。

### Shell 补全

补全命令在帮助信息中隐藏，但可以直接调用：

```bash
# Bash
rsctl completions bash > ~/.local/share/bash-completion/completions/rsctl
rsvm completions bash > ~/.local/share/bash-completion/completions/rsvm

# Zsh
rsctl completions zsh > ~/.zfunc/_rsctl
rsvm completions zsh > ~/.zfunc/_rsvm
```

支持的 Shell 由 `clap_complete` 提供，包括 Bash、Elvish、Fish、PowerShell、Nushell、Zsh 等。可运行以下命令查看当前版本支持的选项：

```bash
rsctl completions --help
rsvm completions --help
```

## ⚙️ 配置

默认配置路径为：

```text
/etc/robot-system/robot-system.conf
```

配置文件使用 TOML 格式：

```toml
[normal]
version = "1.0.0"

[ftp]
host = "packages.example.com"
port = 21
username = "admin"
password = "change-me"
remote_path = "/deploy"

[pkg]
robot-core = "1.2.3"
robot-navigation = "4.5.6"
```

字段说明：

| 区块     | 字段          | 说明                 |
| -------- | ------------- | -------------------- |
| `normal` | `version`     | 当前机器人系统版本   |
| `ftp`    | `host`        | 软件包 FTP 服务地址  |
| `ftp`    | `port`        | FTP 服务端口         |
| `ftp`    | `username`    | FTP 用户名           |
| `ftp`    | `password`    | FTP 密码             |
| `ftp`    | `remote_path` | 软件包所在的远程目录 |
| `pkg`    | `<package>`   | 组件名称及期望版本   |

请妥善保护配置文件权限，尤其是其中的 FTP 密码：

```bash
sudo chmod 600 /etc/robot-system/robot-system.conf
```

## 🤝 贡献

欢迎提交 Issue 和 Pull Request 来改进 Robot System。

开始贡献前，请先：

1. 阅读现有代码和配置约定。
2. 为行为变化补充测试或可复现步骤。
3. 确保格式检查、Clippy、测试和 release 构建全部通过。
4. 在 Pull Request 中说明变更原因及验证方式。

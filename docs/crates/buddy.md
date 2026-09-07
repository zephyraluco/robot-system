# `claurst-buddy` — 电子宠物伴侣系统

> 路径：`crates/buddy` ｜ 单文件 `lib.rs`（**1,118 行**）｜ 仅依赖 `claurst-core` ｜ **目前 workspace 内没有任何 crate 依赖它**（已移植完成但尚未接线到 TUI/CLI）

## 1. 定位

**电子宠物/桌面伴侣系统（Tamagotchi/Buddy companion）**——娱乐性功能，与 AI 助手能力无关。文件头自述：*"claurst-buddy: Tamagotchi/Buddy companion system for Claurst. Ported from src/buddy/ (TypeScript)."*

核心设计：
- **骨骼（Bones）**——物种、稀有度、属性、眼睛、帽子、闪光——全部由 **user-ID 经种子化 PRNG 确定性推导**，用户无法手工篡改。
- **灵魂（Soul）**——名字、性格、孵化时间——首次孵化时由 AI 生成，持久化在 `{config_dir}/companion.json`。

## 2. 依赖

- **内部**：`claurst-core`（`path = "../core"`）。
- **外部**：极轻——`serde/serde_json`、`hex`、`chrono`、`anyhow`；dev-dep 仅 `tempfile`。**无 tokio/reqwest**（纯计算 + 文件读写）。

## 3. 内容分区

| 区域 | 内容 |
|---|---|
| PRNG | `Mulberry32`（32 位小 PRNG，与 TS 版逐位一致）；`seed_from_user_id()`：FNV-1a 32 位哈希，匹配 Bun 的 `hashString` |
| 枚举 | `Species`（**18 种**：duck、goose、blob、cat、dragon、octopus、owl、penguin、turtle、snail、ghost、axolotl、capybara、cactus、robot、rabbit、mushroom、chonk）；`Rarity`（5 档，含 `stars()`）；`Eye`（6 种，`glyph()`）；`Hat`（8 种含 None） |
| 骨骼/灵魂 | `CompanionStats`（debugging/patience/chaos/wisdom/snark，1–100，按稀有度 roll）；`CompanionBones::from_user_id(user_id)` 确定性生成；`CompanionSoul{name, personality, hatched_at}`；`Companion{bones, Option<soul>}` |
| 精灵图 | `SpriteFrame`（每帧固定 5 行 ASCII，`{E}` 占位符渲染时替换为眼睛字形）；`get_sprite_frames(species)` —— 全部 18 物种 × 3 动画帧逐字表（照抄 TS `BODIES` 表）；`animation_frame(tick)` —— 500ms 步进的闲置动作序列 |
| 渲染 | `render(companion, tick) -> String`（替换眼睛、注帽子）；`render_face(bones)`（对话气泡用） |
| 持久化 | `StoredCompanion`（companion.json 的实际内容——**骨骼永不落盘，每次读取重新推导**）；`load_companion_soul`/`save_companion_soul`；**`get_companion(user_id, config_dir)`** —— 主入口 |
| 提示词 | `companion_intro_text(name, species)` —— 伴侣激活时注入的系统提示片段："它坐在输入框旁偶尔冒泡，当用户直呼其名时气泡回答它，你（主模型）保持一句话以内、别抢戏" |
| 测试 | PRNG 范围、种子/骨骼确定性、属性值域、动画周期、全物种渲染不 panic、持久化往返等 |

## 4. 关键公开 API

| 类别 | 条目 |
|---|---|
| **主入口** | `get_companion(user_id, config_dir)`（对应 TS `getCompanion()`） |
| **渲染** | `render(companion, tick) -> String`、`render_face(bones)`、`animation_frame(tick)`、`get_sprite_frames(species)` |
| **生成** | `seed_from_user_id()`（FNV-1a）、`Mulberry32` PRNG、`CompanionBones::from_user_id()` |
| **持久化** | `load_companion_soul` / `save_companion_soul`、`StoredCompanion` |
| **提示词** | `companion_intro_text(name, species)` |
| **类型** | `Companion`（`display_name()`）、`CompanionBones`、`CompanionSoul`、`CompanionStats` |
| **枚举** | `Species`（18 种，`stars()`/`glyph()`/`hat_line()`）、`Rarity`（5 档）、`Eye`（6 种）、`Hat`（8 种） |

## 5. 设计要点

1. **确定性生成**：骨骼完全由 user-ID 决定，无随机存储需求；灵魂（AI 生成部分）才落盘。
2. **零网络依赖**：纯计算 + JSON 读写，可独立单测。
3. **待接线状态**：workspace 内无消费者，预计未来由 TUI/commands 挂载（渲染进 TUI 气泡 + 注入系统提示）。

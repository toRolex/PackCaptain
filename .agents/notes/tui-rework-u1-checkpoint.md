# TUI rework U1/U1b checkpoint（poteto-agent，2026-10-13）

## 状态
- 已完成最小方案实现：`src/tui.rs` 重写 + 新增 `src/tui_theme.rs`（Catppuccin Mocha/Latte）。
- `cargo build`：通过（仅 2 条 dead_code warning：`nav_key`/`NavAction::Quit|ToggleTheme` 为纯测试辅助，可 U2 加 `#[cfg(test)]` 或 `#[allow(dead_code)]` 消除）。
- `cargo test --bin packcaptain`：10/10 tui 单测通过（scope 过滤唯一、跨 scope 拒绝、set_scope 重置、jk/T/q 文本输入隔离、主题 RGB、hint、人名预览、Esc 取消作废 Plan）。
- 全量 `cargo test`：124 通过，10 失败——全部为旧 PTY 测试（tests/tui_pty.rs / tui_extended_pty.rs），按旧 UI 流编写，约定 U2 迁移，本单元不改。失败清单见 `/tmp/packcaptain-u1-logs/after-u1-cargo-test.log`。

## 备份与日志
- 原始 tui.rs 备份：`/tmp/packcaptain-u1-backup.bc25/tui.rs.orig`
- 基线日志（备份后、编辑前）：`/tmp/packcaptain-u1-logs/baseline-cargo-test.log`（全绿，exit=0）
- U1 后日志：`/tmp/packcaptain-u1-logs/after-u1-cargo-test.log`

## 数据形状（已落地）
- `scope: domain::Scope`（Global/Project），`set_scope` 重置 cursor/selected_key/checked/filter/draft/pending/preview。
- `Screen` 扩展：ScopeRoot（仅 全局+各 Snapshot.projects+扫描认领入口，无 agent 子行）、PresetList（presets + 新建 + Skills 管理）、PresetActions（开启/关闭/编辑共享成员）、SkillPick、CreateName/CreateMembers、EditMembers（含"仅保存定义，不同步目标"）、Targets（仅当前 scope）、Confirm、ProjectRoot/ProjectCandidates、EditPolicy。
- `Draft`（typed）：TogglePreset/ToggleSkill/CreatePreset/EditMembers；最终 Command 在选中 targets 后经 `command_for` 一次性构造，并二次校验 scope。
- Confirm 保持 `preview_plan` 配对 + `PreviewChanged` 重确认 + closed-members keep/restore + Discovery 错误语义不变。
- 渲染：全帧主题背景、单高亮（List highlight_style + highlight_symbol=""，无手绘 `>`）、页脚不重复整段预览、面包屑（scope 可见）、诊断独立在头部不覆盖 notice、jk 仅非文本输入、q 仅非文本输入退出、Esc 单级返回并作废 Plan、根页 Space 无绑定无提示。

## U1b 清理（2026-10-13，仅 PTY 迁移前的微小接缝）
- 删除 `allocated_name`（thread_local + Box::leak）：新增 `Row::Project(ProjectId, String)` owned 行标签，ScopeRoot 项目行与 Enter 分支改用 typed id；静态 `Row::Action` 匹配不变。
- Targets "缺失 N"：`merge_into` 仍每 refresh 一次，内层 per-skill 重复合并已改为复用顶部 `merged` 快照。
- dead_code 消除：`nav_key` 加 `#[cfg(test)]`（仅测试用）；`NavAction::Quit/ToggleTheme` 变体加 targeted `#[cfg_attr(not(test), allow(dead_code))]`（仅 nav_key 构造，event loop 直接处理 q/T）。
- checkpoint 日期/标题修正（2025 → 2026-10-13，标注 U1b）。
- 备份：`/tmp/tui.rs.u1b-orig`（U1 后字节级副本）；日志：`/tmp/packcaptain-u1b-logs/{build,test}.log`。结果：build 0 warning exit=0；test 10/10 exit=0。聚焦 diff vs U1 后状态约 65 行改动。

## 遗留问题（U2 接手）
1. 迁移 10 个 PTY 测试到新导航（保留行为断言：fresh confirm、pending journal、manual-close policy、partial failure、cancel 语义、link 校验）。
2. Targets 行的"缺失 N"统计：per-skill 重复 `merge_into` 已在 U1b 修复（单次合并快照复用）。
3. `allocated_name` 已在 U1b 删除，改为 `Row::Project` owned 标签。
4. Skills 管理/新建入口在每个 scope 的 PresetList 内（满足需求）；根页无重复入口。
5. n/m 别名已实现；真实 PTY 人工烟测未跑（本单元禁真实 HOME）。

## 基础设施事件（上报）
- 编辑过程中 pi-lens 自动 fix 多次重排文件，导致一次 edit 部分应用并引入短暂语法错误（已修复，最终 rust clean）；所有后续编辑前已重读盘上内容。

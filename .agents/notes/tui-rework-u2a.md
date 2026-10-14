# TUI rework U2a checkpoint（poteto-agent，2026-10-13）— 中止于迁移开始前

## 状态
- **未做任何测试迁移编辑**。父级 orchestrator 中途指令：收到生产回归审查发现后立即停止，不做迁移、不修生产、不改断言。
- 已完成：读取 poteto-mode、U1 checkpoint、src/tui.rs 关键路径（nav/handle/refresh/preview/confirm/draw）、两份 PTY 测试全文；备份；基线日志。
- 测试文件字节级未动（md5 与备份一致）。

## 备份与日志
- 测试备份：`/tmp/packcaptain-u2a-backup/{tui_pty.rs,tui_extended_pty.rs}`
- 观察用 tui.rs 快照（只读副本，非本单元改动）：`/tmp/packcaptain-u2a-backup/tui.rs.observed`
- 基线日志：`/tmp/packcaptain-u2a-logs/baseline-pty-tests.log`，exit=101，2 passed（pty_screen 支持测试）/ 10 failed / 0 ignored —— 与 U1 报告的 10 个待迁移用例一致，全部为导航序列按旧 UI 流编写。

## 已核实（代码路径直读）的生产回归（与父级审查发现吻合）
1. **open_edit_members 勾选被清空**：`open_edit_members` 先填 `self.checked`（当前成员），随后调用 `reset_list_state()` 清空 `checked`。EditMembers 进入后无预勾选成员。src/tui.rs `open_edit_members`（约 L1050）。
2. **执行结果被 cancel 覆盖**：`confirm_execute` 成功/部分失败先设 `self.notice`，随后 `cancel_operation()` 将 notice 覆盖为 "已取消，未写入"，成功报告（含 部分失败/成功/失败计数）永不显示。src/tui.rs `confirm_execute` 尾部（约 L1210-1230）。
- 其余三项（draw 缺 input 渲染、Esc 销毁 edit draft 后无法继续、header 诊断第二行被 height=3 裁剪）为父级转述，本单元未复现（停止前未跑 PTY 新序列），留给生产修复单元验证。

## 迁移分析结论（供下一单元直接使用，已验证自代码）
- 新流：ScopeRoot → Enter 作用域 → PresetList → Enter preset=PresetActions（开启/关闭/编辑共享成员并同步）或 Skills 管理=SkillPick（e/d per skill、/ 搜索）→ Targets（仅当前 scope 目标，Space+Enter）→ Confirm 预览（Enter 执行、Esc 取消）→ EditPolicy（keep 默认 / restore）。
- **Targets 仅显示 `target.scope == 当前 scope`**（scope_targets，src/tui.rs L137）：全局 scope 无 project 目标。旧测试的跨 3 项目一次全选必须改为按项目 scope 逐个进入执行。这是所有 extended 测试迁移的核心结构改动。
- Fixture 3 个 project target → 每个 project scope 的 Targets 恰 1 行；select 辅助可大幅简化。
- `n` 新建、`m`/Enter 打开 PresetActions；CreatePreset 直接 preview（无 targets）。
- 屏幕可按 hint 文案判别：root="进入作用域"、PresetList="查看 Preset 操作"、Confirm 含"影响预览"、EditPolicy 行含 keep/restore。
- 手动关闭语义保留：SkillPick `d` 直接关闭 skill 记入 manual_closed（引擎行为，测试断言可原样保留）。
- 大清单测试入口改为：root → 全局 → PresetList down×2 → Skills 管理（90 skill 滚动/搜索逻辑不变，注意 SkillPick 面包屑也含 "Presets"，Esc 断言谓词需改用 PresetList 专属 hint）。

## 下一步（fresh 生产修复 owner + 迁移 writer 串行）
1. 先由生产 owner 修复上述回归（至少 #1 #2，验证 #3-#5）并跑通 10 个 tui 单测。
2. 迁移 writer 按本文件"迁移分析结论"重写两测试的按键序列，保留全部行为/FS/状态断言；备份已在 /tmp。

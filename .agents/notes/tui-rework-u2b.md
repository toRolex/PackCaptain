# TUI rework U2b checkpoint（PTY 迁移，poteto-agent，2026-10-13）— 中止于 tui_pty 挂起

## 范围与禁令遵守
- 仅改 `tests/tui_pty.rs` 与本记录；`tests/tui_extended_pty.rs` 字节未动（md5 d01475b7… 与备份一致）。
- 未派发、未操作 Git、未提交、未发布、未触真实 HOME（已给 PTY 子进程显式 `HOME=sandbox home`）。
- 无生产代码改动；未宣称 demo ready。

## 备份与日志
- 测试字节备份：`/tmp/packcaptain-u2b-pty-logs/backup/{tui_pty.rs,tui_extended_pty.rs}`（tui_pty 备份 md5 977c64b4…，extended d01475b7…）。
- 基线：`00-baseline.log`，exit 101，2 passed（pty_screen 支持）/ 10 failed（与已知旧序列一致）。
- `01-tui_pty.log` 首跑 FAILED（cancel 断言 manual_closed=0）；`01b-debug.log` 临时把该断言换成 DEBUG 打印后定位按键错位；`02-tui_pty.log` 第二跑 >60s 无进展被父级终止（exit 143）。

## 已验证的新 UI 导航事实（来自 debug transcript /private/tmp/packcaptain-frontend-pty-30287/pty-transcript-30287.bin）
- ScopeRoot 行序：[全局, project…, 扫描并认领项目]；认领后回到 ScopeRoot。
- 项目作用域 Targets 不止 1 行：该项目的 3 个 agent 行（claude-code/codex/pi，按 id 排序），首行 Claude Code。
- CreatePreset 经 `n` → 名称 → 成员（/ 搜索 + Space）→ 直接预览（无 Targets）。
- PresetActions 行：[开启（选择 agents）, 关闭（选择 agents）, 编辑共享成员并同步]。
- PresetList 与 PresetActions 的 Esc 逐层回退；在文本输入/过滤屏 q 被当字符吞掉。

## 已迁移内容（tests/tui_pty.rs 当前状态）
- 新序列：认领项目 → 项目作用域 `n` 建 PTY Preset → 开启 preset 于 Claude Code 目标 → Skills 管理 `d` 手动关闭 → 取消一次 toggle（断言 manual_closed 仍 1）→ 再开启触发 EditPolicy → restore → 最终断言（presets 1 / enabled_skills 1 / manual_closed 0 / 项目 .claude/skills 1 项 / transcript 含 PTY、Skill、影响预览、执行成功、keep、restore）。
- ⚠️ 未完成恢复：cancel 断言当前仍是 `eprintln!("DEBUG canceled_row=…")`（L195），原 `assert_eq!(…, 1, "cancel changed the manually closed state")` 尚未写回。下一单元必须先恢复此断言再跑。
- transcript 改为 child.wait 后立即落盘并 eprintln 路径，便于失败诊断。

## 挂起（STOP 原因）
02 跑 >5 分钟无输出。tui_pty.rs 无看门狗（原设计如此），child.wait 无超时。最可能根因（推断，未复现）：按键流把应用送入文本输入屏（ProjectRoot / CreateName / 过滤），后续 q 被当输入字符，应用不退出，child.wait 永久阻塞。02 的 transcript 目录为空（落盘在 wait 之后），无法确认实际停留屏。

## 下一单元建议
1. 恢复 L195 原断言。
2. 不要盲目扩按键序列：先把 transcript 落盘提前到每次 key 后（或加 per-key 小延迟已足）＋给 finish/wait 加超时看门狗，失败时能看到真实停留屏再修序列。
3. 逐块校验序列（每块后可用 CLI state 快照锚点），再跑 `cargo test --test tui_pty`。
4. 之后按同法迁移 tui_extended_pty.rs（10 个旧序列；适配要点：Targets 按作用域内 agent 行选择、EditMembers 仅列当前作用域已开启目标、partial failure 改为 global scope 双 agent（claude-code 成功 + Pi 阻挡）、members/cancel 逐作用域执行并文档化适配）。
5. 全部通过后 `cargo test` 全量 + `cargo build`，日志存 /tmp/packcaptain-u2b-pty-logs。

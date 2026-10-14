# U2c 单例 PTY 修复

范围仅 tests/tui_pty.rs 与本记录。禁止生产代码、扩展套件、Git 和派发。

数据形状采用共享原始终端字节与已完成帧的 Screen。读线程持续写独立日志文件，主线程按渲染屏幕等待，独立看门狗持有精确子进程 killer。

工作清单遵循 Bug fix。
1. Reproduce it yourself on the matching surface via the control skill.
2. Binary-search the cause.
3. Plan the fix. 本单元禁止派发，直接修复测试。
4. Verify on the same surface.
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止 Git。
6. Run Opening a PR. skip，禁止发布。

吞吐检查点为先恢复断言并备份，再添加日志与超时，再修一个交互序列。最多三次外部 90 秒限时运行，失败即保留检查点。

已从 U2b 原始备份恢复 cancel manual_closed == 1。原实现等待 child.wait 后才落盘，错误导航会让 q 落入文本输入并永久挂起。源码只读确认操作成功回 PresetList，取消回操作来源屏幕。后续不经 ScopeRoot 重绕。

## STOP 检查点

备份目录 `/tmp/packcaptain-u2c-9dmN4M/backup/tui_pty.rs` 是本单元入场字节，仍含前任 DEBUG。断言随后首先从 U2b 原始备份恢复。

运行上限三次，目前使用两次。
- `01-test.log` 编译失败，Ruby 写入的换行被写成字面 `\n`。已修复并重读自动格式化结果。
- `02-test.log` 编译成功，真实 PTY 0 passed / 1 failed / 2 filtered，4.39 秒结束。证据为等待“输入绝对路径”时 last screen 仍是作用域屏。原 replay 只把 Hide 当完成帧，而文本输入实际用 Show，观察方法漏帧。
- 第二次 transcript `/tmp/packcaptain-u2c-pty-wT4mjQ/transcript.bin` 已持续落盘，panic 有最后屏幕。精确子进程已由 Drop kill/try_wait 清理。未运行第三次。

最后一次 edit 发生 partial apply，按任务硬门停止。五项已写，未写项是两处同文 `assert!(tui.screen().selected().contains("Claude Code"));`。已重读确认当前字节。该 helper 的 selected() 只识别旧 `> [`，当前生产渲染为 `› [ ]`，因此两处应改成当前屏幕行断言。禁止把此检查点称为通过。

已写修复包括本地 replay 将 Show 归一成 Hide、文本输入锚点改为实际标题、过滤中锚点改为“搜索：”。后续还应把批量文本输入等待条件设为完整字段/完整过滤，避免仅字符前缀完成时就发送下个事件。当前 PTY 最终序列尚未验证。

只读源码确认 closed-members-policy-required 由 UpdatePreset 产生，SetPresetEnabled 不提供 restore 策略。本单例保留取消 preset 开启预览并验证 manual_closed==1，随后改为编辑共享成员、同步同一 Claude Code 目标、选 restore。没有生产语义绕过。

原最终字面断言保留。额外加入取消前后 targets JSON 相等及目标 skills 目录为空。未改扩展测试，未运行全量、build 或发布。剩余为两处选中行断言、一次最终受限 PTY 运行，以及 compile/build 验证；第三次若仍红必须再次 checkpoint。

应用原则 Model the Domain 令会话持有生命周期资源与原始字节，Fix Root Causes 依据真实渲染定位观察器漏帧，Prove It Works 禁止把编译成功报告成 PTY 通过。

# U2d STOP 检查点

范围仅 tests/tui_pty.rs 与本记录。未改生产代码、扩展测试或 launcher。未使用 Git、发布或派发。

入场备份 `/tmp/packcaptain-u2d-OEgfnC/tui_pty.rs.before`。本记录入场不存在，无旧记录可备份。

读取 U2c 记录、当前完整单例、只读终端解析器与 src/tui.rs 的目标标签和渲染逻辑。读取 `/tmp/packcaptain-u2c-9dmN4M/02-test.log`。实际 transcript `/tmp/packcaptain-u2c-pty-wT4mjQ/transcript.bin` 有 Hide 2 次、Show 1 次。当前 Session 已把 Show 归一成 Hide，按已完成帧解析；本单元没有修改它。

数据形状为解析后的当前终端屏幕行。Test Behavior, Not Implementation 原则令两处旧 selected() 子串检查变为完整选中目标行相等检查，包含项目名称、技能数量、手动关闭与缺失数量。字面期待依据生产渲染代码，尚未获得本轮 PTY 验证。

一次 edit 请求触发 partial apply。两处目标断言已应用，完整搜索字段等待修改未应用，因为 oldText 在两个搜索步骤中重复。按任务硬门立即停止，没有重放、后续修复、编译或运行。已重新读取自动格式化后的 255–320 行，并用入场备份 diff 确认只改变两处目标断言。

编译次数 0。PTY 运行次数 0。没有新增测试日志或 GREEN 结果。Prove It Works 原则禁止把静态检查当通过。此前 U2c 结果仍是 0 passed / 1 failed / 2 filtered。

原 cancel manual_closed == 1、cancel 前后 targets 相等、目录为空、最终 preset / enabled / manual_closed / 文件系统字面断言全部保留。显式目标、取消、共享成员 restore、keep/restore 证据与 source fixture 均未改。

残留风险为两处完整行字面期待尚未运行，搜索等待仍是 PTY Skill 子串而非完整搜索字段。后续若接手，应先读实际当前文件，使用唯一定位的小 edit，再单独受限编译和单例运行。全套测试仍待验证。不得称为 demo ready。

# TUI U3 checkpoint，未完成

仅修改 src/tui.rs 和本记录。不派发、不操作 Git、不访问真实 HOME。不宣称演示就绪。

## 执行顺序
1. Reproduce it yourself on the matching surface via the control skill. 已通过 App.event / draw TestBackend 复现。
2. Binary-search the cause. 五处机制与失败断言一致。
3. Plan the fix. 沿用 Screen / Draft / checked / pending / preview_plan。静默清理和用户取消分开，不引入框架。
4. Verify on the same surface. 未完成，编辑工具部分应用后按要求立即停止。
5. Stage the commits so the failing repro lands before the fix in git history. skip，本单元禁止提交。
6. Run Opening a PR. skip，本单元禁止发布。

原则。Fix Root Causes 用真实事件及渲染先复现。Prove It Works 要求同一测试 GREEN，但本单元尚未获得。Model the Domain 用命名 cleanup 区分 preview 作废、操作结束和用户取消。

## 备份与证据
编辑前字节备份 /tmp/packcaptain-u3-logs/tui.rs.before。SHA256 20654555b4e53a6aab2552987da61dca9034dcb50c18564e1a40112b2f18705c，与编辑前源码一致。
部分应用后备份 /tmp/packcaptain-u3-logs/tui.rs.partial-checkpoint。SHA256 e83d73acc19883a7f0ceac07ee7c9aa9411085116bf5ae73b51881fad392b7f4。
当前源码 diff /tmp/packcaptain-u3-logs/source.diff。

RED /tmp/packcaptain-u3-logs/red.log。五组实际事件/渲染测试，0 passed，5 failed。
- noop edit 的 checked 为 {}，原成员应保留。
- 执行 notice 为 已取消，未写入，期望 执行成功：1 个目标。
- jkTq 不在实际 TestBackend buffer。
- Targets Esc 实际 PresetList，期望 EditMembers。
- 实际 buffer 无 U3 literal scan diagnostic。

扩展 RED /tmp/packcaptain-u3-logs/red-all.log。8 failed，含 partial/error/fresh-confirm 三个沙箱 engine 测试。fixture 在 tempfile 内创建真实 SKILL.md，调用 NativeAdapters engine，没有真实 HOME 或项目写入。第一轮编译出现 PresetId From<String> 不存在，已改为 From<&str>，后续 RED 为运行时断言而非编译失败。

## STOP 原因
最后一次 functions.edit 含一个无操作替换，oldText Screen::Confirm => { 两处匹配。工具报告 PARTIAL APPLY，其他十个替换已提交，且自动格式化。按指令立即停止，不重试已应用编辑。已读回关键源码并保存 checkpoint。未在部分应用后运行 cargo test 或 cargo build。

## 已应用源码修复，尚未验证
- open_edit_members 先 reset，再读 definition 和 seed checked。
- invalidate_preview / clear_operation / cancel_operation 分离。confirm_execute 改静默清理，保留 notice。
- Confirm / Targets / EditPolicy 回退先取 source，只清 preview，重置行并从 Draft 重建成员 checked。
- CreateName / ProjectRoot 渲染 input 并设光标。
- 有 diagnostics 时 header 5 行，并启用 wrap。
- 八个 U3 回归测试在 src/tui.rs 内。

## 下一单元必须先处理
1. 先读当前盘上源码，不重放已应用 edits。运行 cargo test --bin packcaptain u3_。
2. 回归测试有待修正的 fixture 假设。u3_error_report 当前 RED 停在 Confirm 前，refresh 保留 selected_key 时 noop 的 Down 可能仍停在 skill，须通过实际选择 Action 修正导航后确认错误字符串。u3_preview_changed 的外部重复开启可能产生相同 Plan，不应把该失败视为产品缺陷，需构造确实改变计划的物理状态。
3. 原始五组测试 GREEN 之后，再验证 partial/error 字面报告、PreviewChanged、Esc 后续 Enter 和取消零写入。必要时补目标 scope 不扩大的实际事件断言。
4. 当前输入超长横向滚动及极窄终端诊断可见性未验证。
5. 跑 cargo test --bin packcaptain 和 cargo build，保存 GREEN 日志，独立审查 diff。
6. 不在本单元迁移 PTY。下一 PTY 单元迁移十个已知旧序列失败，并用沙箱 HOME 做真实终端本地 demo。没有全量 green 或 ready 声明。

## U3c 完成记录

本轮先运行 U3 测试，再读取当前源码并备份。未重放之前 edits。只修改本记录与 src/tui.rs。未派发、操作 Git、提交、发布或使用真实 HOME。

吞吐检查点。限制为删除一个不受支持动作和修正三个失败 fixture，使用已有 Screen / Draft / checked / pending / preview_plan。验证顺序为 U3、binary unit tests、build。PTY 和旧导航迁移留给 U2。

原则。Fix Root Causes 使修复针对空 sync_targets 被 engine 拒绝，而非放宽 engine。Model the Domain 保留现有 Draft 状态和当前作用域已启用目标集合，不新增自动开启规则。Prove It Works 要求 App.event / draw 加真实沙箱 engine 回归通过。Unslop 要求报告直接列事实与未验证范围。

### 日志和备份

新目录 /tmp/packcaptain-u3c-G2PiFq。
- tui.rs.before 是本轮编辑前字节，SHA256 69d26ea0a9c1c50343fa43fd74b4a64dd1477842bc79e78e25531e9906c6883a。
- notes.before 保留之前记录。
- 01-u3-red.log 为首跑，5 passed，3 failed，10 filtered out。三个失败都在空目标保存 fixture 的 Confirm 断言。
- 02-u3-check.log 为第一轮修改后，9 passed，2 failed。中文宽字符的 TestBackend cell 含占位空格。删除 skill 的 fixture 触发 macOS /var 符号链接防护。
- 03-u3-green.log 文件名保留但该轮未 GREEN，10 passed，1 failed。engine 实际错误 Display 不含 stale-preset-revision code 前缀。
- 04-u3-green.log 为最终 U3，11 passed，0 failed，10 filtered out。
- 05-bin-test.log 为 cargo test --bin packcaptain，21 passed，0 failed。
- 06-build.log 为 cargo build，exit 0。
- source.diff 为本轮相对字节备份的 diff，人工读完 328 行，无阻断发现。

旧 /tmp/packcaptain-u3b-provider-failure/tui.rs.snapshot 与 /tmp/packcaptain-u3-logs 均保留。未改变 engine 文件或 confirmed API。

### 本轮决定与证据

移除“仅保存定义，不同步目标”行及其事件分支。成员编辑只进入当前作用域已开启目标列表，显式 Space 选择后才预览。空列表在实际 draw 中显示“当前作用域无已开启目标，无法保存成员；Esc 返回编辑”，Enter 不产生 pending / preview，Esc 保留成员并能继续返回。引导文案说明仅支持同步保存。CreatePreset 仍使用原有无 target 命令。

成功编辑 fixture 先通过 engine 真正开启全局 Claude Code，然后真实事件选择目标。noop 成员与 checked seed 保留。Targets / Confirm Esc 后再次 Enter、Space、Enter 可重建预览，取消前后 snapshot 不变。scope 测试同时存在全局和项目已启用 target，列表分别只显示当前作用域的一项，未自动选中。

沙箱路径 canonicalize 消除 macOS /var 到 /private/var 的符号链接，仅修 fixture，不更改 engine 安全检查。错误测试使用带明确 target 的外部 UpdatePreset 造成真实 revision 冲突，匹配实际 Display 字面文本。宽字符渲染断言只去掉 TestBackend 占位空格，仍检查完整限制文本。

PreviewChanged fixture 本轮未改动。它物理新增 external/SKILL.md，并断言旧新 Plan 不同、第一次确认不写入、Esc 不写入、重新预览与 fresh Plan 相等、再次确认成功。成功、部分失败、错误报告均通过原事件路径而非手工赋值。部分失败仍由 .pi 外来文件阻挡造成真实 per-target report。

新增三个测试覆盖无已启用目标限制、双作用域隔离、新建与 q 未确认不写入。原 literal jkTq / backspace、diagnostic、success、partial、error 和 fresh-confirm 回归全通过。

### U2 接续

本轮 binary 的十个原有 unit tests 也通过，它们不是之前十个旧导航序列失败。旧 PTY/集成导航序列本轮未迁移也未运行，仍按先前未解决项处理。下一 U2 需加载沙箱 HOME 的真实 PTY，迁移这十个旧序列并演示。不要宣称 demo ready。

未验证长输入横向滚动、极窄终端布局、真实 PTY 输入和退出恢复。源码关注点为 refresh 的 Targets scope/enabled filter、EditMembers / Targets event、go_back、goto_targets、confirm_execute 和 draw。

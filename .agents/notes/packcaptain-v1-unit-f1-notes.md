# F1 native config errors

## 状态与 checkpoint

- 复现，done。E=/private/tmp/packcaptain-f1-gOOv2M。原 config-repro-v2 复现 exit0，原安全断言 exit1。
- 修复，done。adapters/engine/cli/tui、tests/config_errors.rs 和一项真实 PTY 回归。typed target errors 保留 stored intent/ownership。
- 验收，done，仅 F1 seam。focused-final exit0 共58 passed，all-targets exit0 共90 passed，均0 failed/ignored。fixed binary verify5-fixed 100/100，原安全断言 exit1 到 exit0。不是最终安全完成。

## 清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 已执行原真实 CLI。
2. Binary-search the cause. 配置解析失败在 discovery 映射 false，merge 清 enabled_skills，无 effects 的 execute 提交关闭 intent。
3. Plan the fix. 单 writer，禁止派发，串行比较设计。
4. Verify on the same surface. 原 config-acceptance 字节不变，固定新 binary 的 config probe 后 exit0。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止提交。
6. Run Opening a PR. skip，禁止发布。

## how 与设计

数据形状为 DiscoveryReport.target_errors 的 TargetId 到 DiscoveryError 映射。未知不属于 active=false。config 在 target 扫描边界先读并解析，失败 target 不合并 entries 或 observed_enabled，保留最后可信 stored intent/ownership。选择此形状而不把整个 discover 改成 Result，后者会取消既有多 target 部分失败契约。正常 targets 继续独立执行；失败 target 在任何本 target merge/prepare/journal/effect 前拒绝。pending recovery 遇到受影响 target 则整次拒绝，不消费已有 journal。preview 与严格 TUI 确认拒绝未知 target。

Fix Root Causes 决定先保留原 CLI RED。Model the Domain 决定以 typed target error 表达 unknown。Boundary Discipline 决定集中在配置读边界，不新增全局验证层。Prove It Works 决定保存 binary SHA 与原退出码。

吞吐检查点按备份和 RED、最小接口与 GREEN、focused 与 fixed artifact 验收顺序。walker 全部错误传播不纳入 F1，精确交接。planned shared entry 风险保留 F2。编辑前 E/before 是全部 src/tests/Cargo 逐字节备份。不会执行 Git/jj、全仓格式化、clippy fix、真实 auth 或真实 agent 配置操作。

## 裁定与实际实现

D2 既有真实 PTY 把第二 target 的 .claude 替换成普通文件。新的 config 边界收到 ENOTDIR。全局拒绝会破坏原两 target 部分失败契约。主管批准健康 Plan 投影严格确认。fresh 和 confirmed 同时去掉 unknown target 的 effects/target_updates，健康投影完全相等才执行；不同则整个本命令零业务写。unknown target 明确 failed，结果 Plan 不含其 mutation。全 unknown 的确认 typed 拒绝，UI pending Command 保留。含受影响同步 target 的 preset definition 更新保守整体拒绝，未引入新的跨 target 定义更新策略。

持久 recovery journal 与 UI pending Command 不混同。confirmed 命令只要存在任一 journal 就整体拒绝，不消费 recovery。CLI 自动 recovery 在 read-only snapshot/journal 配置边界核验后才进入 StateStore::open；recover_pending 再核验待恢复 targets。claim_project 的恢复入口也先读边界核验。list 对 unknown 加 observation 与 diagnostic，state 是最后可信 stored intent，不声称当前 enabled 已知。preview 拒绝 unknown target。

原 config-repro-v2 含“假成功必须存在”的缺陷复现断言，修后不可能完整通过，未改原件。新临时 config-probe-green 仅移除这四项旧缺陷存在断言并改变隔离 fixture 名，其余 link/source/identity 断言保留；原 config-acceptance 安全断言逐字节复制并独立运行。verify 副本仅定向 BIN/RUNS 两行，所有100项断言保持。

## 证据与失败保留

E 下 logs 含每次真实 command/log/exit/binary。首次 focused exit101 是 recovery 证据目录未建，四项 cp 探针失败；focused2 exit101 是上述 D2 兼容冲突。focused3 exit101 包含 TUI refresh 用新扫描诊断覆盖 partial-failure notice，以及新增 test 用旧流文本观察被 ANSI delta 分割；focused4 exit101 仅新增成功 notice 流文本断言错误，改为真实屏幕模型，未放宽产品断言。既有 tests 没有删改断言。focused-final exit0，58 passed。all-targets exit0，90 passed。其中3项顶层 helper 直接返回，不能当独立恢复场景。recovery logs 含真实 child 与 SIGKILL。

初次固定 binary cp 因 E1 复制的只读 mode 失败，导致 config-acceptance-green exit1 和 verify5 exit0 都仍测旧 binary，不计 GREEN。移旧 copy 为 packcaptain-red，再新建 fixed artifact，config-acceptance-fixed exit0，verify5-fixed 100/100。旧日志全保留，不隐藏挑绿。最终 fixed 和 Cargo CLI cmp exit0，SHA256 均为 758832ae5a2f61c9aa487b81d00cb755f7b72dbc74592e574712fb2d4ca70c06。RED SHA 为 a55747bccdc3dccbb55fe12541db86c9aa20ae01b0657b37b791d306ab38a693。

config_errors 六项实际涵盖 parse/read失败、snapshot/entries/owned dev-inode-readlink/source/config字节保持、空目标三个 adapters invalid config、两target部分失败、健康确认 Plan 变化零写和相等后健康成功、持久 Applying payload字节及snapshot/link零消费。read失败是 config 路径为目录，非权限chmod探针。真实 PTY 新项证明错误不消费 UI pending Command，修 config 后同一 Command 可收敛。既有 D2 pending journal PTY 全通过。

禁止文件 domain/storage/main/lib/Cargo 与其余测试逐字节一致，protected-files.exit=0。改动仅批准的四src、config_errors、必要tui_extended_pty迁移和本notes。没有Git/jj或暂存区核验，不声称index为空。未运行全仓fmt/clippy fix。编辑hook仅格式化编辑文件，逐次重读。LSP六文件仅cfg hints，5项inconclusive，不作clean证明。

## 未测与精确交接

walker 和其他 scan 错误仍不完整传播。下一单元可在 discover_target 的 WalkDir Err、root.exists/is_file、canonicalize、fingerprint_entry/source Err 分支写同一个 target_errors map，并确保 fail target 不合并。fingerprint失败仍可能 active=false，F1没有扩修。需要独立真实EACCES、walker和missing/read分辨fixture，不可声称全IO安全。

三个adapter parse错误边界已测，其他adapter native成功写仅沿既有回归，未运行真实Codex app-server或Pi加载验证。未测权限失败、Linux/Windows、跨state-dir、非协作writer hash到rename/unlink微窗口、掉电。planned共享入口风险保留F2。config路径NotFound保持原missing语义，dangling/父组件竞态没有完备证明。

runtime-evidence.json 读取本run首assistant metadata，2026-10-11T18:36:51.785Z，cliproxy/gpt-6.1-sol/openai-completions。仅Pi运行时声明，上游实际model/effort未知。证据在/tmp可能被清理。

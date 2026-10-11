# 最终验证证据索引

当前状态是 E2 有界独立再验收通过，不是发布授权、全仓安全保证或真实 Agent 加载证明。权威记录是 [final-revalidation](../.agents/notes/packcaptain-v1-final-revalidation.md)，运行步骤是 [README](../README.md)。[历史交接](../.agents/notes/packcaptain-v1-handoff.md)保留旧失败，不代表当前状态。

## 固定产物与源码身份

E2 证据根为 [/private/tmp/packcaptain-e2-yXzBP0](file:///private/tmp/packcaptain-e2-yXzBP0/)。它是测试机器的外部临时目录，可能被系统清理。不存在时不能假定原日志仍可复查。仓库不收录 binaries 或巨量 transcript。

| 对象 | 身份或结果 | 证据 |
| --- | --- | --- |
| 测试后固定 CLI | SHA256 `88cb8dbbedacd332e27ccc8b088643f43e175434584c9fea9cc52eb2aa2fdb44` | [bin/packcaptain](file:///private/tmp/packcaptain-e2-yXzBP0/bin/packcaptain)、[fixed-compare.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/fixed-compare.log) |
| build 后、test 前 CLI | SHA256 `f0a405a7c7be60804bef950d9fbf44700431a049d2bcd0847d17e2f497416a34`，不冒充最终 fixed | [build.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/build.log) |
| 冻结清单 | 37 文件，src、tests、Cargo、当时 docs、AGENTS、CLAUDE 和 GLOSSARY | [source-before.sha256](file:///private/tmp/packcaptain-e2-yXzBP0/source-before.sha256)、[source-final.sha256](file:///private/tmp/packcaptain-e2-yXzBP0/source-final.sha256) |
| 清单自身 SHA256 | 前后均 `7fdcd23b0e63a710a9c04eff2fc50859700bc8f5050d725042ce89f72b44d1c0`，cmp exit0 | [source-compare.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/source-compare.log) |
| 汇总 | 21 个执行入口全部 exit0 | [summary.json](file:///private/tmp/packcaptain-e2-yXzBP0/summary.json) |

E2 固定文件 size 15924576、inode 161095779、mode 0555，mtime `2026-10-12T03:37:11+0800`。测试确实重新链接 CLI，固定副本取自测试后产物，末次与 Cargo CLI cmp exit0。E3 新增文档不属于上述 E2 的 37 文件冻结快照。

每个 E2 执行入口有 `logs/<label>.command`、`.log`、`.exit`、起止时间和前后二进制身份。E2 没有产品失败后重试或工具编译失败。E3 未重跑构建或全量测试。

## E2 实际运行

以下数量不能相加称为独立产品场景。

| 验证入口 | 实际结果 | 证据 |
| --- | --- | --- |
| `cargo build --offline` | exit0，隔离 Cargo 与 target | [build.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/build.log) |
| `cargo test --offline --all-targets -- --nocapture` | 108 passed、0 failed、0 ignored | [all-targets.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/all-targets.log) |
| 原业务黑盒 `acceptance.mjs` | 111 命令，106 exit0，5 个预期错误非零 | [acceptance.command](file:///private/tmp/packcaptain-e2-yXzBP0/logs/acceptance.command)、[黑盒 logs](file:///private/tmp/packcaptain-e2-yXzBP0/evidence/packcaptain-blackbox-run-p7xopK/logs/) |
| `verify.js 5` | 100/100 assertions，五轮双 PID overlap 均 true | [report.json](file:///private/tmp/packcaptain-e2-yXzBP0/evidence/verify-dbrf0U/report.json)、[verify5.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/verify5.log) |
| 真实 SIGKILL 和合作并发 | child PID、signal 9、持久 gate、两种锁顺序各五轮双 PID 存活和 exit0 | [all-targets.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/all-targets.log)、[recovery](file:///private/tmp/packcaptain-e2-yXzBP0/recovery/) |
| fixed 扩展 PTY | 12 passed、0 failed、0 ignored | [fixed-pty.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/fixed-pty.log)、[fixed-pty](file:///private/tmp/packcaptain-e2-yXzBP0/fixed-pty/) |
| fixed 原 PTY | 1 passed、0 failed、0 ignored | [fixed-original-pty.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/fixed-original-pty.log)、[原 PTY transcript](file:///private/tmp/packcaptain-e2-yXzBP0/original-pty-transcripts/) |
| fixed 长列表额外 3 轮 | 每轮 1 passed、0 ignored，无失败后重试 | [fixed-long-1.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/fixed-long-1.log)、[fixed-long-2.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/fixed-long-2.log)、[fixed-long-3.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/fixed-long-3.log) |

108 条含 `crash_child`、`forward_safety_child` 和 `rollback_safety_child` 三个直接返回的顶层 helpers，不能当作独立恢复场景。全量加 fixed PTY 和三轮长列表共 124 个通过测试条目，仍含 helpers，不是 124 个独立产品场景。

fixed PTY 用当前 test 源码直接编译，断言未改。其嵌入 fixed CLI 路径及依赖身份见 [harness-inputs.sha256](file:///private/tmp/packcaptain-e2-yXzBP0/harness-inputs.sha256)。它验证 D2 严格重确认、连续状态变化、Esc 零业务变化、迟到 manual-close、foreign 入口拒绝、Applying journal 不消费，以及配置错误后保留 UI pending Command。长列表执行 90 项、95 次 down/up、回顶和搜索 Skill089。这些是行为证据，help 输出不是行为验证。

## F1、F2、F3 的探针与证据边界

| 单元 | E2 实际验证 | 边界与索引 |
| --- | --- | --- |
| F1 配置 Unknown | 原 config 安全断言 exit0。invalid config 后 disable exit1，list unknown，link inode 与 source 不变 | [F1 notes](../.agents/notes/packcaptain-v1-unit-f1-notes.md)、[config-finding-v2.json](file:///private/tmp/packcaptain-e2-yXzBP0/evidence/config-finding-v2.json)、[config-safety.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/config-safety.log) |
| F1 定义同步 | fixed CLI 拒绝含 Unknown 已启用同步 Target 的定义更新，SQLite 主文件字节、source 与 link 身份相等 | [definition-zero-write.cjs](file:///private/tmp/packcaptain-e2-yXzBP0/scripts/definition-zero-write.cjs)、[definition-zero-write.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/definition-zero-write.log) |
| F2 shared entry | 最新 lib 关联的 probe exit0，首 Target 逻辑关闭后，第二 Target prepare 失败，真实 shared link 保留 | [F2 notes](../.agents/notes/packcaptain-v1-unit-f2-notes.md)、[shared-safe-probe](file:///private/tmp/packcaptain-e2-yXzBP0/evidence/shared-safe-probe/)、[shared-safety.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/shared-safety.log) |
| F3 scan Unknown | fixed CLI 的真实 read EACCES 与 WalkDir EACCES 两项 exit0，Unknown、preview 和执行拒绝、DB 字节保持，恢复权限后合法 disable 成功 | [F3 notes](../.agents/notes/packcaptain-v1-unit-f3-notes.md)、[scan-permission.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/scan-permission.log)、[scan-walkdir.log](file:///private/tmp/packcaptain-e2-yXzBP0/logs/scan-walkdir.log)、[read fixture](file:///private/tmp/packcaptain-e2-yXzBP0/evidence/scan-3UH3xJ/)、[WalkDir fixture](file:///private/tmp/packcaptain-e2-yXzBP0/evidence/scan-IEOKXz/) |

config 生成器沿用 F1 green probe，不是安全判定本身。它删除旧缺陷必须存在的断言，差异见 [config-generator.diff](file:///private/tmp/packcaptain-e2-yXzBP0/scripts/config-generator.diff)。真正安全判定来自逐字节不变的 E1 `config-acceptance.cjs`。

shared probe 的 ownership 和第二 Target prepare failure 是构造输入，FS、NativeAdapters 与 SQLite 真实。它不是纯 fixed CLI 的自然可达场景，也不证明真实 Agent 已停止加载。F2 有界自然 CLI 观察证明普通三个 Target 创建不同入口，外部 `.agents/skills` overlap 可见但未自动认领 owned。未穷尽所有自然 owned overlap。

[D2 notes](../.agents/notes/packcaptain-v1-unit-d2-notes.md)记录 pending journal 与 UI pending Command 的区别。TUI 确认保守拒绝持久 pending，不消费恢复。现有合法 CLI 自动恢复后还执行自己的业务意图，没有独立 recover 命令。

## 查找复跑入口

完整精确命令保存在上述 `.command` 文件中。以下是 E2 实际入口形态，不是本轮重新执行的结果。

```sh
cargo build --offline
cargo test --offline --all-targets -- --nocapture
```

黑盒实际 driver 位于 `/private/tmp/packcaptain-blackbox-20261011/scripts/acceptance.mjs`，将 fixed binary 作为参数。安全脚本副本位于 E2 的 `scripts/verify.js`，仅 BIN 和 RUNS 两行定向替换，见 `scripts/verify.diff`。扫描脚本参数是 binary 和新的 fixture 基目录。

```sh
E=/private/tmp/packcaptain-e2-yXzBP0
fresh=$(mktemp -d)
node /private/tmp/packcaptain-blackbox-20261011/scripts/acceptance.mjs "$E/bin/packcaptain"
node "$E/scripts/verify.js" 5
node "$E/scripts/cli-scan.cjs" "$E/bin/packcaptain" "$fresh"
node "$E/scripts/cli-walkdir.cjs" "$E/bin/packcaptain" "$fresh"
```

不要盲目复用 E2 的旧输出目录。复跑前检查 driver、二进制 SHA、脚本输出路径和环境隔离，将固定旧 fixture 输出路径迁移到新临时目录并保存差异。`verify.js` 的 RUNS 固定为旧目录，直接重复运行不是全新独立验收。PTY 与 shared 编译还依赖 E2 当前 lib、依赖及临时编译工具，入口见 `scripts/compile-pty-v3.sh`、`scripts/compile-original-pty.sh` 和 `scripts/compile-shared.sh`。这些临时文件缺失时只能报告无法复跑，不能猜测通过，也不能用新 help 检查替代旧行为证据。

## 保证之外

同 state-dir 合作锁不保护跨 state-dir 共 Target。非协作 writer 的最终检查到 rename 或 unlink 微窗口仍需暂停外部写入。私有 receipt 不防同 UID 恶意篡改。`NeedsRecovery` 是保守停机，不要手工乱删 journal 或 anchor。

删除 crash、多 route partial mutation、Prepared 前后更细实杀、commit 后立即实杀、orphan staging、cleanup 失败、掉电及跨平台未全覆盖。shared entry 保留不保证先关闭 Target 的真实 Agent 不加载。只实测 macOS，本轮不声称真实 Agent 新会话已经加载或卸载 Skill。暂存区未检查，不声称为空。用户禁止提交、推送和发布仍有效。

## E3 文档核验

E3 使用同一 SHA 的 fixed CLI，只运行 help 和全新 [/private/tmp/packcaptain-e3-SlFafb](file:///private/tmp/packcaptain-e3-SlFafb/) 沙盒，不操作真实默认目录。真实 CLI 验证 lists、search、project add 与 scan、create、preview 不改变目标状态、enable 与 disable、update Keep 与 Restore，以及 Codex/Pi 两个显式项目 Targets。

[examples.json](file:///private/tmp/packcaptain-e3-SlFafb/examples.json)、[examples-result.txt](file:///private/tmp/packcaptain-e3-SlFafb/examples-result.txt)保存通过记录。首次文档验证工具错误把 create 的操作报告当成 Preset 对象，产生 `preset-not-found`，保留于 [examples-tool-error.json](file:///private/tmp/packcaptain-e3-SlFafb/examples-tool-error.json)。修正为从 presets list 获取 ID 后通过，未改产品。README 中 TUI 行为引用 E2 fixed PTY，本轮 TUI 只核实 help，没有把 help 计为行为验证。

E3 禁止文件 SHA 对比和文档检查见 [unit-e3-notes](../.agents/notes/unit-e3-notes.md)。运行时声明见 E2 [runtime-evidence.json](file:///private/tmp/packcaptain-e2-yXzBP0/runtime-evidence.json)，仅证明 Pi 声明，不证明上游真实模型或 effort。

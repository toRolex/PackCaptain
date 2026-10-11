# PackCaptain 单元 0 基线

记录时间为 2026-10-12，工作目录为 `/Users/rolex/Documents/Codes/githubProject/MyProject/PackCaptain`。本单元只核验，不修复源码，不提交、推送或删除既有文件。

## 结论

`cargo build` 退出 0。`cargo test --all-targets` 退出 0，30 项通过、0 项失败。`node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5` 退出 1，五轮 100 项断言中 56 通过、44 失败。另有 `--version` 退出 2，不在这 100 项断言中。安全基线仍未通过。

全部新日志目录为 `/tmp/packcaptain-u0-20261012T000424-6TKnAP`，下文称 E。macOS 下 `/tmp` 与 `/private/tmp` 指向同一临时位置。命令 stdout、stderr 合并保存，真实退出码另存 `.exit`，没有用管道退出码替代命令退出码。

## 隔离及边界

执行 build、test、verify 前设置以下环境，原始值及工具版本见 `E/environment.log`。

- `HOME=E/home`。
- `CARGO_HOME=E/cargo-home`，从真实 Cargo 缓存只读复制 registry 与 git，实际 Cargo 写入落在新临时目录。
- `XDG_CONFIG_HOME=E/xdg-config`，`XDG_DATA_HOME=E/xdg-data`，`XDG_STATE_HOME=E/xdg-state`。
- `TMPDIR=E/tmp`，另创建 `E/state`。

Cargo 测试的真实 CLI 与 PTY 使用各自 TempDir 下显式 `--agent-home`、`--state-dir`。verify 脚本每轮使用新 `round-NN/agent-home`、`round-NN/state-dir` 及 sandbox project，子进程 HOME 也覆盖到该轮 sandbox。脚本 `--version` 子进程继承 E/home。没有执行真实 agent 配置的写入命令。

允许重建产生的 `target/**` 是 Cargo 产物。本单元的仓库交付文件只有本文件。源码、测试及 Cargo 清单的前后 SHA-256 一致，证据见下文源文件核对。测试自身会清理其新 TempDir，本单元没有删除任何既有文件。

## 二进制身份及漂移

身份命令为 `shasum -a 256 target/debug/packcaptain` 与 `stat -f 'mtime=%Sm epoch=%m size=%z inode=%i' -t '%Y-%m-%dT%H:%M:%S%z' target/debug/packcaptain`。

| 阶段 | SHA-256 | mtime | 大小 | 证据 |
| --- | --- | --- | --- | --- |
| 交接且本轮重建前 | `4c94df66df9758bf07c11e85f33cdc2c68dd3766ca3ad812ca9534c88a6a837c` | `2026-10-11T22:58:53+0800` | 14867440 | `E/prerequisites.log` |
| cargo build 后 | `f8d0b3380b81574e726e3e43faf95a07389a9ebff6e40b0dcdb08a1470174e3f` | `2026-10-12T00:04:47+0800` | 14872224 | `E/binary-after-build.log` |
| cargo test 后，实际 verify 所测 | `4bcc35b908ffffd209b004ec1f3de3830eee46c8fc97d226870fd1e9588e4ef8` | `2026-10-12T00:05:03+0800` | 14872960 | `E/binary-after-test.log`、verify report 的 `binaryIdentity` |

交接基线发生漂移。按交接约定，重建后 hash 不同即标记源码已漂移，不能再把旧二进制结果当作当前源码结果。严格而言，二进制 hash 本身不能区分源码、构建环境或编译产物变化，本单元没有旧源码快照来定位漂移文件。

`cargo test` 又重新链接了 `target/debug/packcaptain`，因此必须区分 build 身份与实际安全复跑身份。最终 `E/binary-final.log` 与 test 后身份一致。该中途 hash 变化并非本单元修改源码，`cmp E/source-before.sha256 E/source-after.sha256` 退出 0。清单覆盖 `src/**`、`tests/**`、`Cargo.toml` 与 `Cargo.lock`。

## Cargo 结果

| 命令 | 真实退出码 | 结果 | 原始证据 |
| --- | --- | --- | --- |
| `cargo build` | 0 | 编译通过 | `E/cargo-build.log`、`E/cargo-build.exit` |
| `cargo test --all-targets` | 0 | 30 通过，0 失败，0 忽略 | `E/cargo-test-all-targets.log`、`E/cargo-test-all-targets.exit` |

测试分组为 lib 1、main 0、adapter_discovery 3、adapter_links 2、cli_e2e 2、domain_rules 15、engine_transactions 5、storage_roundtrip 1、tui_pty 1。各测试名称和真实结果完整保存在测试日志中。没有 Cargo 失败项可列。

## 安全脚本及逐轮结果

脚本与旧证据均存在，未发生缺失。存在性检查 `ls -ld /private/tmp/packcaptain-cli-safety-scripts/verify.js /private/tmp/packcaptain-cli-safety-runs/verify-hKpYF4` 及脚本 `shasum -a 256` 结果见 `E/prerequisites.log`。脚本 SHA-256 为 `88e7aeb30db8a4f01abc4a2fc737b85a04723a4493a9f100f62e8d1826852b6d`，副本为 `E/verify-original.js`。

真实命令为 `node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5`。退出 1，原始输出为 `E/verify.log`，退出码为 `E/verify.exit`。

原生 runRoot 为 `/private/tmp/packcaptain-cli-safety-runs/verify-XCGmLQ`，下文称 R。原始报告为 `R/report.json`，每轮完整 stdout、stderr、JSON、events 与 round 报告位于 `R/round-01` 至 `R/round-05`。全目录保留副本为 `E/verify-XCGmLQ`。

| 轮次 | 通过 | 失败 | 并发 enable 失败进程 | 并发 disable A 退出码 | 外部替换后 disable B 退出码 | enable 窗口重叠 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 13 | 7 | A，exit 1 | 0 | 0，错误成功 | 是 |
| 2 | 10 | 10 | B，exit 1 | 1 | 0，错误成功 | 是 |
| 3 | 10 | 10 | B，exit 1 | 1 | 0，错误成功 | 是 |
| 4 | 10 | 10 | B，exit 1 | 1 | 0，错误成功 | 是 |
| 5 | 13 | 7 | A，exit 1 | 0 | 0，错误成功 | 是 |
| 合计 | 56 | 44 | 5 轮均非全成功 | 3 轮失败 | 5 轮错误成功 | 5 轮 |

逐轮退出码来自 `R/round-NN/events.json`，并发失败响应在该轮的 `concurrent-enable-A.json`、`concurrent-disable-A.json`。注意原脚本重用 `concurrent-enable-B` 标签，第二次 enable B 会覆盖该名称的单独 JSON 文件。第一次 enable B 失败必须以 `events.json`、`round.json` 或顶层 `report.json` 内 events/assertions 读取，不能仅看被覆盖后的单文件。

## 与交接 29 项失败的对照

分类使用实际断言名称，不把派生状态断言另计为新的根因。命令 `node E/summarize.mjs` 退出 0，脚本、原始输出及分类结果分别为 `E/summarize.mjs`、`E/classification.log`、`E/classification.json`，退出码为 `E/classification.exit`。旧报告仍在 `/private/tmp/packcaptain-cli-safety-runs/verify-hKpYF4/report.json`，保留副本为 `E/handoff-report.json`。

| 分类 | 交接失败断言 | 本轮失败断言 | 对照及证据 |
| --- | --- | --- | --- |
| 外部替换错误成功 | 20 | 20 | 五轮各 4 项。`actual-disable-B-after-replacement.json` 均 exit 0、ok:true，B 存储状态改为 disabled，正常 B link 被移除，无 NeedsRecovery。foreign 目录、source 和移走的原 owned link 均保留。见每轮 `round.json` 的 conflictStoredState 与顶层 report assertions。 |
| 并发失败 | 9 | 24 | 原 1/5 轮 enable 失败变为 5/5 轮，原 2/5 轮 disable A 失败变为 3/5 轮。包含链接集合和持久状态的派生失败。见每轮 events 与 report assertions。 |
| --version 退出码 | 另记，不在 29 内 | 另记，不在 44 内 | 两次均为 2，stdout 均为 `packcaptain 0.1.0`。脚本只记录 binaryIdentity.versionExitCode，不计入断言。 |
| 其他 | 0 | 0 | 两份 report 的所有失败断言均归入上述外部替换或并发类。 |

本轮比旧报告多 15 项失败断言。并发时序及二进制身份均有变化，不据此单独声称所有新增失败都是源码回归。

并发 enable 非零响应均为 `partial-failure`，摘要为 `managed skill entry path is occupied by an unowned file`。第 2、3、4 轮 disable A 非零响应均为 `partial-failure`，摘要为 `target-local disable for an existing Claude project directory is not verified; no files were changed`。这些错误原文与命令参数保存在每轮 events 中，并在 `E/classification.json` 的 nonzeroEvents 中汇总。

## 证据索引与限制

所有 E 路径均展开到 `/tmp/packcaptain-u0-20261012T000424-6TKnAP`。除上述日志外，`E/source-before.sha256`、`E/source-after.sha256`、`E/source-compare.log`、`E/source-compare.exit` 保存源码核对。查找命令为 `find src tests -type f -exec shasum -a 256 {} \; | sort`，另追加 `shasum -a 256 Cargo.toml Cargo.lock`。

源文件前后相同只能证明本单元观察窗口内无变化，不能证明上一 worker 的未提交源码稳定。安全脚本未覆盖交接中其余 IO、rollback、crash recovery 或完整 TUI 验收。本次只建立可对照基线，未作根因修复。临时证据路径未来可能被系统清理。

应用 Prove It Works 原则，实际驱动重建后的 CLI 并保留身份与退出码，没有用 Cargo 全绿替代安全证明。

## 逐项失败清单

以下由 `E/classification.json` 生成。每项原始断言及附带响应见 `R/report.json` 的 assertions，逐轮运行命令见 `R/round-NN/events.json`。

### 第 1 轮

- 并发失败。第 1 轮两项并发 enable 均成功。
- 并发失败。第 1 轮并发启用后链接 realpath 集恰为 a,s,b。
- 并发失败。第 1 轮同一 target 中 A、B 两组都存储为 enabled。
- 外部替换错误成功。第 1 轮 disable B 对被替换路径安全拒绝，不能返回成功。
- 外部替换错误成功。第 1 轮冲突后状态仍显示 B 的 stored intent，且命令未报告成功。
- 外部替换错误成功。第 1 轮失败 target 的 before 状态未变或明确标记 NeedsRecovery。
- 外部替换错误成功。第 1 轮拒绝冲突后正常 entry B 保留，foreign 不冒充 source S。

### 第 2 轮

- 并发失败。第 2 轮两项并发 enable 均成功。
- 并发失败。第 2 轮并发启用后链接 realpath 集恰为 a,s,b。
- 并发失败。第 2 轮同一 target 中 A、B 两组都存储为 enabled。
- 并发失败。第 2 轮并发 disable A 与 enable B 均成功。
- 并发失败。第 2 轮收敛后链接 realpath 集恰为 s,b。
- 并发失败。第 2 轮目标状态精确为 A=false、B=true。
- 外部替换错误成功。第 2 轮 disable B 对被替换路径安全拒绝，不能返回成功。
- 外部替换错误成功。第 2 轮冲突后状态仍显示 B 的 stored intent，且命令未报告成功。
- 外部替换错误成功。第 2 轮失败 target 的 before 状态未变或明确标记 NeedsRecovery。
- 外部替换错误成功。第 2 轮拒绝冲突后正常 entry B 保留，foreign 不冒充 source S。

### 第 3 轮

- 并发失败。第 3 轮两项并发 enable 均成功。
- 并发失败。第 3 轮并发启用后链接 realpath 集恰为 a,s,b。
- 并发失败。第 3 轮同一 target 中 A、B 两组都存储为 enabled。
- 并发失败。第 3 轮并发 disable A 与 enable B 均成功。
- 并发失败。第 3 轮收敛后链接 realpath 集恰为 s,b。
- 并发失败。第 3 轮目标状态精确为 A=false、B=true。
- 外部替换错误成功。第 3 轮 disable B 对被替换路径安全拒绝，不能返回成功。
- 外部替换错误成功。第 3 轮冲突后状态仍显示 B 的 stored intent，且命令未报告成功。
- 外部替换错误成功。第 3 轮失败 target 的 before 状态未变或明确标记 NeedsRecovery。
- 外部替换错误成功。第 3 轮拒绝冲突后正常 entry B 保留，foreign 不冒充 source S。

### 第 4 轮

- 并发失败。第 4 轮两项并发 enable 均成功。
- 并发失败。第 4 轮并发启用后链接 realpath 集恰为 a,s,b。
- 并发失败。第 4 轮同一 target 中 A、B 两组都存储为 enabled。
- 并发失败。第 4 轮并发 disable A 与 enable B 均成功。
- 并发失败。第 4 轮收敛后链接 realpath 集恰为 s,b。
- 并发失败。第 4 轮目标状态精确为 A=false、B=true。
- 外部替换错误成功。第 4 轮 disable B 对被替换路径安全拒绝，不能返回成功。
- 外部替换错误成功。第 4 轮冲突后状态仍显示 B 的 stored intent，且命令未报告成功。
- 外部替换错误成功。第 4 轮失败 target 的 before 状态未变或明确标记 NeedsRecovery。
- 外部替换错误成功。第 4 轮拒绝冲突后正常 entry B 保留，foreign 不冒充 source S。

### 第 5 轮

- 并发失败。第 5 轮两项并发 enable 均成功。
- 并发失败。第 5 轮并发启用后链接 realpath 集恰为 a,s,b。
- 并发失败。第 5 轮同一 target 中 A、B 两组都存储为 enabled。
- 外部替换错误成功。第 5 轮 disable B 对被替换路径安全拒绝，不能返回成功。
- 外部替换错误成功。第 5 轮冲突后状态仍显示 B 的 stored intent，且命令未报告成功。
- 外部替换错误成功。第 5 轮失败 target 的 before 状态未变或明确标记 NeedsRecovery。
- 外部替换错误成功。第 5 轮拒绝冲突后正常 entry B 保留，foreign 不冒充 source S。

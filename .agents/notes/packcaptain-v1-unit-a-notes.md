# PackCaptain 单元 A 记录

## 根因与选择

discovery 用新 canonical source 和指纹覆盖了历史 owned 身份。foreign 目录导致旧 S 不进入 observed_enabled，planner 省略关闭 S 的 effect，随后移除正常 B 并存储 B=false。同源外部 symlink 则继承 ownership 和新指纹。这两个分支都把外部替换变成合法状态。

继续沿用 SkillEntry 的 source_path、entry_fingerprint、present、active 和 ownership。没有引入 engine 或 domain 接口。discovery 遇到同路径 owned 身份变化时不接受新身份，保留历史 route 和 active，让 prepare/apply 的前置条件核验拒绝冲突。foreign 不能作为原 source 的替代。prepare 在任何实际写入前检查所有 effects，故正常 B 保留，stored preset intent 不被关闭。

mixed native/owned routes 现在安全拒绝。旧实现将整个 skill 路由给 native writer，跳过 owned link 操作，却将全部 routes.active 写成 false。未证明 native 开关覆盖 alias 的精确语义，因此没有以合并两种写入机制取代假成功。

pending/missing 只证明预期路径 absent，不提供认领现存同源 symlink 的权限。创建使用 no-clobber symlink，已有入口安全拒绝。删除也不能凭 pending/missing 跳过真实指纹核验。

## 恢复契约与限制

apply_target 接受 entries_after 中持久化的实际 owned fingerprint。若所有 after routes 已达到 desired，且真实 no-follow identity、source 和 present 都匹配，则幂等成功，返回当前真实指纹。pending 不属于实际指纹。同源新 inode 不能冒充原 owned entry。六项真实 TempDir 回归包含这个恢复分支。

创建成功与实际 fingerprint 持久化之间崩溃时，journal 若仍为 pending，就无法区分本进程创建的 link 和外部同源替换。必须安全拒绝并由 engine 明确报 NeedsRecovery，不能用 canonical source 相等自动认领。B/C 后续 actual-state 协调须遵守这个边界。本单元没有修改 EffectExecutor 签名。

原生 JSON config writer 在写临时文件前和 rename 前检查内容 hash，拒绝已观测到的更改并使用原子 rename。最终 hash 检查与 rename 之间仍有非协作写入窗口。当前操作系统调用没有 compare-and-swap rename，advisory state lock 也不能约束 agent 或编辑器。非协作 writer 在这个窗口写入可能被覆盖，不能承诺任意并发外部写都零损失。受管 link 的 verify 到 unlink 之间同样有窗口。这里没有用增加一次 hash 检查夸大保证。

协议要求 config 和受管入口变更期间，外部 writer 暂停或遵守同一个排他锁。仅 PackCaptain 参与者受 state lock 保护。回归证明的是 prepare 后、apply 前已经发生的 config 更改被拒绝，不是对最终微窗口的证明。完整消除窗口需要平台特有文件操作协议或所有 writer 协作，超出 adapters 单文件修复范围。

## 原始证据

日志目录 E 为 `/tmp/packcaptain-uA-20261012T001051`。所有命令实际退出码保存在对应 `.exit`，没有用管道退出码替代。build 和 test 使用 E/home 作为 HOME，CARGO_HOME 显式为 `/Users/rolex/.cargo`。真实 CLI 均显式传入隔离 agent-home 和 state-dir。测试在 `/private/tmp` 的 TempDir 内操作。没有修改真实 agent 配置。

| 命令 | 退出码 | 结果和证据 |
| --- | --- | --- |
| `cargo build` | 0 | E/cargo-build-before.log，E/cargo-build-before.exit |
| `shasum -a 256 target/debug/packcaptain`，首次 build 后 | 0 | `ef924344a11374030e103d9211cdee323f67ac0e6f84de6f220611ec5232f01f`，E/binary-before.sha256 |
| `node /private/tmp/packcaptain-cli-safety-scripts/verify.js 1`，修复前 | 1 | E/repro.log，E/verify-H0ZRoc/report.json。外部替换 4 项均失败，actual disable exit 0/ok:true |
| `cargo test --test adapter_replacement_safety`，修复前 | 101 | E/regression-red.log。4 项均失败，含 mixed routes 与 pending 同源认领 |
| `cargo test --test adapter_replacement_safety`，最终 | 0 | E/regression-recovery.log。6 项全部通过 |
| `cargo test --all-targets`，最新完整尝试 | 101 | E/cargo-test-confirm.log。D 单元新增 version 回归仍报 exit 2，该测试阻断后续 suites |
| `cargo test --all-targets -- --skip cli_version_exits_successfully_without_initializing_storage` | 101 | E/cargo-test-skip-version.log。A 六项、现有 domain/engine/storage suites 通过。B 三项恢复测试 ignored。D 新 PTY 长列表失败，最后项未显示 |
| `node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5`，最终 | 1 | E/verify-final.log，E/verify-xIBUJ8/report.json，E/verify-classification.log。外部替换 20/20，通过总计 76，失败 24 全为并发类，与基线并发失败数相同 |
| `shasum -a 256 target/debug/packcaptain`，最终 verify 所测 | 0 | `bb8c3ea491720dabed4daf7c6c01fb025b43c107272b4548e3aebbf53a3b44b9`，E/binary-tested.sha256 和 report.binaryIdentity |

最终 verify 原始目录为 `/private/tmp/packcaptain-cli-safety-runs/verify-xIBUJ8`，完整副本在 E。每轮 events.json 保存原命令、stdout、stderr、退出码。round.json 保存 before/after target state；每轮冲突 disable 均非零且 ok:false，stored intent 与 before state 保留，正常 B entry 保留。

skip-version 后共享二进制再次变化为 `62dfe6fdc061cf1cc1b40fd332c4391c1f790571ae0c17a27db2c2941cf545ef`，见 E/binary-post-verification.sha256。它不是最终 verify 所测的二进制，不混用证据。

## 文件边界与偏离

本单元写入 src/adapters.rs、tests/adapter_replacement_safety.rs 和用户指定的本 notes。未写 engine、storage、domain、CLI、TUI、Cargo 清单，没有提交、推送或发布。

E/source-before.sha256、E/source-after.sha256、E/source-final.sha256 保存共享目录的前后快照。B/D 等并行写入使 engine/domain/CLI/TUI 及其他 tests 发生变化，不能据此声称全仓 hash 不变。父会话还告知 D 曾对 adapters 执行 clippy --fix，少量机械增量已保留。白名单外无本单元编辑调用，但总 hash 不能归属到各 worker。Cargo.toml、Cargo.lock、storage.rs、main.rs、lib.rs 在观察窗口一致。

diagnosing-bugs 与 tdd skills 不可用。按失败复现、失败真实 FS 回归、修复、同入口复验执行。无内部再派发，遵守当前 worker 的派发权限。未达到 cargo all-targets 全绿，阻塞项已上报父会话，不能把范围外失败隐藏成通过。

应用 Fix Root Causes，先证实 discovery 丢失身份的机制再修改边界。应用 Model the Domain，保留现有 typed route 身份而不添加平行布尔状态。应用 Test Behavior, Not Implementation，真实 CLI 和 TempDir 断言退出语义、stored intent 与实际入口。应用 Prove It Works，记录真实二进制 hash 与逐轮原始结果。

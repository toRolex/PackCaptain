# C3 concurrent disable

## 清单与 checkpoint

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 已备份到 `/tmp/packcaptain-c3-disable-5KLuzO/before`，新增真实 CLI 回归在未改生产的现实现 GREEN。
2. Binary-search the cause. 已分类 guard，managed误判属于 stale discovery，现实现无该症状。
3. Plan the fix. 没有真实产品 RED，生产零修改，保留合法 guard。
4. Verify on the same surface. 两种锁顺序各五轮、guard零业务写、focused、verify5、all-targets全部通过。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止提交。
6. Run Opening a PR. skip，禁止发布。

数据形状沿用真实 CLI fixture、TargetId、SQLite Snapshot 和 SkillEntry ownership。Model the Domain 决定不新增生产状态。Test Behavior, Not Implementation 决定检查真实进程、links 目的地与重开 SQLite。Prove It Works 决定保留退出码与 binary 身份。Laziness Protocol 决定现实现 GREEN 时只交付测试。

checkpoint 为备份/现实现回归、guard 分类、focused/verify5/all-targets。先验证再决定是否修复，禁止扩展 TUI 与跨 state-dir。

## how 与设计

prepare_target 仅 non-PackCaptainLink routes 调用 ensure_native_scope_safe。Claude project 原生目录与 non-owned symlink 的 target-local 开关未验证，合法拒绝必须保留。managed routes 使用严格 verify_route 和 owned link writer。C3-enable 已将 CLI 实际扫描放入 state lock，使用最新 snapshot，避免另一进程创建的 managed links 被旧 discovery 标成 unmanaged。

初始候选用 SIGSTOP 强制指定 lock 顺序，实际不成立。macOS 会将已排队的 stopped waiter 授锁，另一个进程阻塞 flock。sample 保存于 E/hang-sample.log。首次测试工具180秒超时，debug重跑随后手动终止，均不是产品 RED，不伪造 Cargo exit。已恢复临时 child 并确认退出。

最终使用现有 lock barrier，预期先执行进程先入队300ms，再 spawn 第二个 CLI，再等300ms。双 PID 均 try_wait=None 后释放锁。不能仅信任入队顺序，所以用真实返回的 disable plan 验证。初始仅 A enabled，disable先执行产生 a/s 两个 effects；B先启用后 disable 只产生 a 一个 effect。每种顺序五轮均符合2/1，证明实际先后，而非只声称 spawn 顺序。最终没有信号调度、生产钩子或语义修改。

## Deviations

how/architect 串行比较，任务明确禁止子派发和动态 workflow。没有架构变更。

## 结果与根因分类

E 为 `/tmp/packcaptain-c3-disable-5KLuzO`。仅增加 tests/concurrent_cli.rs 的两项测试与本 notes，src、其他tests、Cargo逐字节不变，见 E/boundary.diff。原enable测试保留。没有修生产，无RED-to-GREEN生产diff。

baseline的旧错误有两个不同含义。真实DirectDirectory或UnmanagedSymlink进入ensure_native_scope_safe为合法native拒绝。managed入口被锁外旧discovery误标unmanaged才是错误调用该guard。后者已被C3-enable锁内fresh scan消除，本轮两种顺序各五轮无误拒绝。此结论限当前CLI场景，不声称静态discovery API也修复。

真实CLI最终无需repeat/repair就精确得到A=false、B=true、enabled_skills={s,b}、两个symlink且canonical目的地恰s/b。SQLite重新open后检查三项owned metadata，a为absent/inactive，s/b为present/active，journal数量0。HOME全树文件内容、symlink目的地、目录inode均保持一致，证明source未移除或替换。双PID、alive、exit与实际effects证据见final-regression.log。

合法guard分别真实DirectDirectory与UnmanagedSymlink，两者exit1、ok:false，原错误原文保持。测试预存discovery snapshot，避免把合法inventory refresh当成写入失败。比较home/project/state全树字节、类型、inode、路径列表，settings.local.json、正常A links、SQLite均不变，snapshot重开相等且journal0。这里证明零持久业务写，不声称无文件open、atime或同字节SQL调用。A六项另保留mixed、foreign、pending/missing及同源替换拒绝。

## 命令与证据

Cargo环境使用临时HOME=E/home、XDG_CONFIG_HOME与XDG_STATE_HOME，CARGO_HOME=/Users/rolex/.cargo仅既有工具缓存。CLI fixtures另用独立临时home/state/project。没有auth读取、真实agent配置写、既有skills删除、提交或发布。hook仅格式化测试与markdown，逐字节边界核验无越界。未全仓fmt或clippy fix。

| 命令 | exit | 结果 | E下证据 |
| --- | --- | --- | --- |
| cargo test --test concurrent_cli -- --test-threads=1 --nocapture | 0 | 3 passed，0 failed，0 ignored，两顺序各5轮，原enable5轮，两个guard | current-green.log/.exit与final-regression.log/.exit |
| cargo test --test adapter_replacement_safety --test recovery_real --test recovery_forward_safety --test recovery_rollback_safety --test recovery_receipts --test storage_roundtrip -- --test-threads=1 --nocapture | 0 | 38 passed，0 failed，0 ignored | focused.log/.exit |
| cargo test --all-targets | 0 | 两次均76 passed，0 failed，0 ignored，PTY真实执行 | all-targets.log/.exit、final-all-targets.log/.exit |
| node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5 | 0 | 两次均100/100 | verify5.log/.exit、final-verify5.log/.exit |
| cmp E/binary-tested.sha256 E/binary-final.sha256 | 0 | verify前后binary一致 | binary-compare.log/.exit |
| git diff --cached --name-only | 0 | 空，无暂存文件，只读检查 | staged-files.log |

focused包含A六项全部通过，以及forward/rollback真实kill/reopen回归，日志含signal9及持久journal。38包含仅在子进程环境运行的helper，不把它解释为38个独立产品场景。LSP最终0诊断但1 inconclusive，不算confirmed clean，实际Cargo提供编译证据。

verify原目录为 `/private/tmp/packcaptain-cli-safety-runs/verify-PrHBgm` 与 `/private/tmp/packcaptain-cli-safety-runs/verify-8mZUwI`，完整副本在E。每轮events保留命令、PID、窗口、stdout/stderr/exit，report保留100项断言。binary SHA-256为 `9b506711e70a6b32348a7c02a0f2cec5306ccdc64cd5bcf7d9c1e4bf93b8c72b`，与C3-enable相同。最终report身份见verify-identity.log，versionExitCode=0。

当前runtime metadata来自本run `5301e3d1-e26a-4c8f-a760-4d3ad6953024_poteto-agent_transcript.jsonl` 首assistant，timestamp=2026-10-11T17:41:43.411Z，provider=cliproxy、model=gpt-6.1-sol、api=openai-completions、thinkingLevel=high，见runtime-evidence.log。仅证明Pi metadata，上游模型身份与effort未知。

串行no-comments/diff复读未发现blocker，新增注释和suppressions为0，无删除/restore。测试返回plan的2/1断言保留实际order证据，未依赖FIFO假设。任务到此停止。

## 未测边界与剩余风险

仅实测本机macOS/Unix、同state-dir双CLI。跨state-dir竞争、更多并发、跨平台、静态discovery caller/TUI一般freshness未测。先入队窗口可能在极端调度下反转，测试会因真实plan不符而失败，不会误报另一顺序已验证。外部非协作writer微窗口沿用A记录，未放宽ownership、fingerprint、receipt或安全guard。PTY Skill089曾间歇失败，两次本轮all-targets均通过仍不消除既有风险，本轮不修。日志位于/tmp，可能被系统清理。

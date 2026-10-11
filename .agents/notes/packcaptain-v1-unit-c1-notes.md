# C1 回滚 actual-result 契约

## 范围与 checkpoint

仅修复完成真实 mutation 后返回错误丢失 actual entries。先复现，再迁移错误结果与调用者，最后同入口和全量测试。吞吐 checkpoint 为每个阶段保存退出码与日志，达到预算上限则停在当前完成命令后。备份和日志位于 `/tmp/packcaptain-c1-k3jdAP`。

## 执行清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. 已用真实 FS 指定 ignored 回归复现。
2. Binary-search the cause. 已核对 wrapper 真实 disable 成功后丢弃 entries，inverse 使用旧指纹。
3. Plan the fix. 串行完成 Ground / Sketch / Agree / Implement / Scrap。禁止子代理，设计探索在本进程完成。
4. Verify on the same surface. 待执行。
5. Stage the commits so the failing repro lands before the fix in git history. skip: 用户禁止版本控制。
6. Run Opening a PR. skip: 用户禁止提交发布。

## 根因与数据形状

`Result<entries, String>` 丢失已证明的完成结果。journal 仍保存 planned disable entries 的旧 fingerprint，inverse 看到缺失 link 却要求旧 inode，因而 NeedsRecovery。

候选一用独立 resume API 和 receipts 扩展持久 schema，能处理更多 crash 阶段，但超出 C1。候选二仅把 error 改为 `ApplyFailure { message, actual_entries: Option<...> }`，复用 journal 的 entries_after。选择二。actual_entries 必须是 executor 可证明的完整实际 metadata，而非失败后扫描同源入口。None 不自动 inverse，保留 NeedsRecovery。原有成功结果不变。inverse 依据已验证缺失 route 的 missing 指纹创建，不凭 pending 认领现存入口。

原始 RED 退出 101，日志 `red.log`，失败为 `managed link disappeared after preview`。Model the Domain 决定具名错误结果。Laziness Protocol 排除 resume/schema 增量。Fix Root Causes 要求真实 RED 后才编辑。Prove It Works 要求真实 FS 与持久 journal 验证。

## Deviations

无派发和动态 workflow。遵守上层任务禁令，how 与 architect 的角色由本 agent 串行完成。首次向主管误报 hook 越界，没有工具证据，已立即撤回；没有据此改动或恢复文件。

## 实际实现与验证

修改 `src/engine.rs`、`src/adapters.rs`、`tests/recovery_real.rs`、`tests/engine_transactions.rs`。storage 未变。迁移 EffectExecutor 声明、NativeAdapters、engine 内 CountingExecutor、recovery 的 InterruptingNative、transactions 的 FakeExecutor。直接 adapter 调用者只依赖 Result 成败和成功值，不需编辑。错误 String 通过 From 转为无 actual proof。NativeAdapters 内部未新增失败后扫描认领，无法证明完整结果时安全返回 None。

engine 先持久化 error actual entries，再标记 RollingBack，随后 inverse。inverse 的 missing before 指纹证明已知缺失，不能使用旧 inode 强制创建；创建仍 no-clobber，A 的身份校验未放宽。inverse 返回实际新指纹后沿用既有 SQLite 原子提交。inverse error 若带 actual 也保存，仍保留 NeedsRecovery。

目标回归补充 inverse 调用之前重开 SQLite，精确检查 RollingBack 和已持久 actual missing entries。新增真实 FS 无 actual proof 回归，证明不执行 inverse，links 保持已移除、intent 保留且 journal 为 NeedsRecovery。

所有测试使用 E/home 的隔离 HOME、XDG 路径，fixtures 为新 TempDir，CLI 传独立 agent-home/state-dir。CARGO_HOME 与 RUSTUP_HOME 只复用 Rust 缓存，没有读取真实 auth 或修改 agent 配置。

| 命令 | 退出 | 结果 | 原始证据 |
| --- | --- | --- | --- |
| 指定目标测试带 --ignored --exact --test-threads=1 --nocapture | 101 | 原始真实 RED，0 passed、1 failed | E/red.log、E/red.exit |
| 指定目标测试取消 ignore 后 --exact --test-threads=1 --nocapture | 0 | 1 passed，含 inverse 前持久 proof 检查 | E/green-final.log、E/green-final.exit |
| cargo test --test adapter_replacement_safety | 0 | 6 passed | E/replacement.log、E/replacement.exit |
| cargo test --test recovery_real -- --test-threads=1 | 0 | 10 passed、0 failed、2 ignored | E/recovery.log、E/recovery.exit |
| cargo test --all-targets | 101 | 已运行累计 50 passed、1 failed、2 ignored | E/all-targets.log、E/all-targets.exit |

全量唯一已观察失败为 `tests/tui_extended_pty.rs:483` 的 Skill089 断言。停止于该 suite，后续 tui_pty 未在本次全量执行。不把 ignored 当通过，crash_child 的 no-child helper 返回也不计独立产品场景。没有跑 verify.js，按任务留给最终稳定单元。

首次 GREEN 后增加持久化断言时，误用 private pending_journals 被 hook 诊断拒绝。改为测试直接读 SQLite payload，不扩大 storage API。第一次新增断言使用 kebab-case status，而持久格式实际为 RollingBack/NeedsRecovery，测试退出 101；纠正字面期望后最终全绿。中间日志保存为 E/green-assertion-attempt.log 和 E/recovery-assertion-attempt.log。

二进制 SHA-256 为 `e9f3a626b9a0c49badd015cf55a8797241404b81ab36261284522c788d4b3806`，见 E/binary.sha256。E/source-before.sha256 与 E/source-final.sha256 仅四个白名单文件变化，storage cmp 为 0，未见 hook 越界写入。定向 auto-format 仅作用已编辑白名单文件，没有全仓格式化。E/*.rs.diff 保存相对开工备份的审阅差异。

LSP 检查四文件诊断 0，但四项均 inconclusive，不能当 confirmed clean。Cargo 编译与实际测试提供验证证据。没有执行 git/jj，未核验暂存区。

## 剩余风险与交接

`killed_forward_process_recovers_and_retry_converges` 与 `killed_rollback_process_recovers_recreated_fingerprints` 仍 ignored，留 C2，本轮未重新运行。真实 mutation 与 actual metadata 持久化之间的 kill 微窗口未修。journal entries_after 仍复用现有语义，没有引入独立 receipts；无法证明完整 identity 就 NeedsRecovery，不承诺自动恢复。NativeAdapters 内部 partial failure 没有增加逐 entry receipts，因此可能保守停在 NeedsRecovery。并发和非协作外部 writer 微窗口不在本轮范围。

有效 session 模型有运行时证据。E/runtime-evidence.log 摘录当前 child session 的 model_change 为 cliproxy/gpt-6.1-sol，thinking_level_change 为 high，首条 assistant metadata 的 model 同为 gpt-6.1-sol、api 为 openai-completions。仅证明 Pi session 生效配置与响应 metadata，不证明上游服务实际 reasoning effort，后者未核实。不是仅根据派发参数断言。

C1 的局部回归已验证，不称整版完成。主会话负责接受变更、处理全量 TUI 失败和 C2。

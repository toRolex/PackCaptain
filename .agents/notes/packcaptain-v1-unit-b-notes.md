# 单元 B 回滚与崩溃恢复记录

## 结论与范围

用户的 Missing 旧成员移除在源返回后重新同步不再复活。恢复后下一条命令也不再用恢复前的 discovery 覆盖刚提交的 entry metadata。后续维护者继承真实 CLI、真实 NativeAdapters、SQLite 重开及 SIGKILL 回归。

本单元修改 `src/domain.rs`、`src/engine.rs`、`tests/recovery_real.rs` 与本记录。`src/storage.rs` 不需新增修改，既有 `commit_target_and_finish_journal` 已把实际返回 entries 与 target 状态一起提交并删除 journal。

尚未完成完整 crash/rollback 验收。主管明确要求与 adapters 耦合的三项 RED 等单元 C，禁止 B 修改 adapters。它们有带原因的 `#[ignore]`，不是通过。

所有证据位于 `/tmp/packcaptain-uB-20261012T001118`，下文称 E。日志保存完整命令输出，`.exit` 保存实际退出码。本单元未提交、推送、发布或修改真实 agent 配置。CLI 明确传隔离 `--agent-home`、`--state-dir` 与 HOME，fixture 均为新 TempDir。测试中的 executor 包装真实 NativeAdapters，只在其前后阻塞或注入错误，不模拟文件系统。

## 根因与修复

1. 错误预检前恢复。进入本单元时已有修复，`execute_with_discovery` 先只读读取并构造 prospective state，再做冲突检查，锁内重复检查，之后才恢复。新增 `conflicting_policy_does_not_apply_pending_journal` 用真实链接与 SQLite pending journal 证明缺策略时零状态与文件变更。`invalid_cli_does_not_recover_persisted_pending_mutation` 在真实进程写入并 SIGKILL 后验证错误 CLI 不消耗 journal。现状通过，不宣称当前 checkout 有 RED→GREEN。
2. rollback fingerprints。进入本单元时 engine 已持久化 `apply_target` 的实际返回值。`persisted_actual_rollback_state_recreates_and_commits_fingerprints` 持久写入 RollingBack actual entries、真实移除链接、用 fresh CLI 恢复，重开数据库后与 fresh discovery 指纹比较，再次 disable 成功。仍存在 adapter 错误返回丢失实际 metadata 的缺口，下节列出。
3. 落后 preset 重开遗留移除成员。进入本单元时 planner 已遍历定义与旧 applied_members 的并集。`reopening_lagging_preset_removes_old_entry_on_disk` 用真实 CLI 在另一 target 更新全局定义，再重开落后 target，证明旧 link 被移除，数据库精确只留 a。现状通过，不宣称当前 checkout 有 RED→GREEN。
4. Missing 旧成员丢历史。planner 跳过不可用旧成员的 IO，却仍把 applied_members 覆盖为新定义，源返回后无法知道还要移除旧 link。现在 `members_with_deferred_removals` 保留不可用旧成员，直到可用时执行清理。更新与整组重开共用该结构。`missing_removed_member_keeps_history_until_source_returns` 原始 RED 为 `missing old member lost history and revived`，修后 GREEN。既有 retained manual-close 语义不变，真实新增回归 `missing_retained_manual_close_survives_real_reopen` 证明源返回后仍要求明确策略且未复活。
5. 恢复后的 stale discovery。journal 成功恢复并提交后，engine 仍合并进程恢复前扫描的 entries 与 enabled_skills，覆盖刚写回状态，下一次 prepare 将 owned links 当作 unowned。现在仅从传入 discovery 删除本次 pending journals 对应 targets 的 entries 与 observed_enabled，保留已恢复的持久状态。`killed_before_native_mutation_recovers_in_fresh_cli` 在真实独立进程持久化 journal 后 SIGKILL，fresh CLI 恢复、数据库重开与真实 link 指纹检查，再幂等重试。原始 RED 为 `managed skill entry path is occupied by an unowned file`，修后 GREEN。此修复不处理一般并发 stale discovery，留给 C。

数据形状选择采用 Model the Domain 原则。用现有 applied_members 保存未落实移除，而非添加需所有 TargetState 字面量迁移的额外字段。取舍是 applied_revision 已更新时 applied_members 可能含暂未能移除的旧成员。该集合继续表示 target 的落实快照，不强称其总等于定义。清理仅在后续该 preset 同步或重开时发生，不承诺源返回后自动后台清理。

Fix Root Causes 原则使修复落在历史丢失和 stale discovery 覆盖处，没有放宽全局 fingerprint 校验。Prove It Works 原则使证据直接来自真实 CLI、链接、SQLite 和独立 kill 进程，而非 fake executor 全绿。

## 单元 C 所需接口提案

当前缺口不能只靠 engine 修。`apply_target` 返回 `Err(String)` 时没有携带实际已完成 FS 操作后的 entries。inverse 使用 journal planned entries，真实 link 已消失或重建时旧 fingerprint 不再可用。崩溃发生在真实 mutation 与 journal 更新之间时也无法分辨 planned 与 actual。不能把所有 expected fingerprints 改为 pending，否则会把外部同源替换认领为自身操作。

建议新增具名结果并迁移所有调用者，类型放 engine，adapter 实现由 C 协调。精确草案如下。

```rust
pub struct ApplyFailure {
    pub message: String,
    pub actual_entries: Option<BTreeMap<SkillId, Vec<SkillEntry>>>,
}

pub enum ResumeDirection {
    Forward,
    Rollback,
}

fn apply_target(
    &mut self,
    mutation: TargetMutation<'_>,
    snapshot: &Snapshot,
    agent_home: &Path,
) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure>;

fn resume_target(
    &mut self,
    mutation: TargetMutation<'_>,
    direction: ResumeDirection,
    snapshot: &Snapshot,
    agent_home: &Path,
) -> Result<BTreeMap<SkillId, Vec<SkillEntry>>, ApplyFailure>;
```

`apply_target` 正常入口保留严格 preview identity guards。失败且可确定实际结果时返回 actual_entries，engine 在标记 RollingBack 前持久化该 metadata。无法证明实际 ownership 时返回 None，engine 保留 NeedsRecovery，不凭扫描认领外部变化。

`resume_target` 只能在持久 journal 已存在且锁内调用。它区分原始 planned intent 与可证明实际 entries。已证明同源且 PackCaptain-owned 的入口达到 desired 时幂等返回当前指纹，不因旧 planned fingerprint 单独报错。发现 foreign、非 link、不同 source 或不能区分同源外部替换时必须拒绝并保留 NeedsRecovery。恢复检查不能仅靠路径或 canonical source 相同认领 ownership，C 需选择持久 per-entry receipts 或明确限制的恢复协议。

建议在 journal 增加独立 `actual_entries` 与落实阶段或逐 entry receipt，而不把回滚实际 entries 覆盖到用于 forward replay 的 planned entries。rollback 返回实际 fingerprints 后可继续用现有 SQLite 原子 commit。跨 FS mutation 与 receipt 持久写仍存在不可完全消除的窗口，必须明确协议边界。

对应 RED 测试均在 `tests/recovery_real.rs`。

- `killed_forward_process_recovers_and_retry_converges`。真实 forward link 创建后 SIGKILL，fresh CLI 失败为 `skill link changed after discovery`。
- `rollback_fingerprints_persist_after_real_native_recreation`。真实 NativeAdapters 完成 disable 后注入错误，inverse 在重建前失败为 `managed link disappeared after preview`。
- `killed_rollback_process_recovers_recreated_fingerprints`。当前尚不能进入 rollback 完成后的 kill gate，子进程先因上述 inverse 问题退出 101。这项不是 kill/重启通过证据。C 修 actual-state 契约后才能到 gate，随后要确认 signal 9 与 restart convergence。

## 命令、退出码与证据

| 命令 | 退出码 | 结果 | 证据 |
| --- | --- | --- | --- |
| `cargo test --test recovery_real -- --test-threads=1 --nocapture`，修前 | 101 | 4 通过、5 失败，含 Missing 真失败 | `E/red.log`、`E/red.exit` |
| 同命令，domain 修后、engine 修前 | 101 | Missing 已通过，恢复后 stale discovery 失败 | `E/green-attempt.log`、`E/green-attempt.exit` |
| 同命令，最终 | 0 | 8 通过、3 明确忽略待 C | `E/final-recovery.log`、`E/final-recovery.exit` |
| `cargo test --test recovery_real -- --ignored --test-threads=1 --nocapture` | 101 | 3 项 adapter 耦合仍 RED | `E/coupled-red.log`、`E/coupled-red.exit` |
| `cargo test --all-targets` | 0 | 50 通过、0 失败、3 忽略，含原 30 项和其他并行单元测试 | `E/all-targets-final.log`、`E/all-targets-final.exit` |
| `HOME=E/home XDG_CONFIG_HOME=E/home/xdg-config XDG_STATE_HOME=E/home/xdg-state node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5` | 1 | 79 通过、21 失败，失败均并发，外部替换断言当前通过 | `E/verify.log`、`E/verify.exit`，原报告 `/private/tmp/packcaptain-cli-safety-runs/verify-E49FgL/report.json` |
| `shasum -a 256 target/debug/packcaptain` | 0 | 被测 SHA-256 `62dfe6fdc061cf1cc1b40fd332c4391c1f790571ae0c17a27db2c2941cf545ef` | `E/binary.sha256`、`E/binary-final.sha256` |
| `cmp E/binary.sha256 E/binary-final.sha256` | 0 | verify 与后续全量测试后二进制相同 | `E/binary-compare.exit` |
| `cmp E/storage.rs src/storage.rs` | 0 | storage 无本单元新修改 | `E/storage-compare.exit` |

全量测试日志计数包含无需 CHILD 环境时只返回的 `crash_child` helper，不把它当作独立产品场景。真实 kill PID、signal 9 与 pending count 在最终 recovery 的 nocapture 日志中。LSP 对三文件诊断为 0 但 push-only 状态 inconclusive，不能算 confirmed clean。Cargo 编译与真实测试是检查证据。

安全脚本从基线 56 通过、44 失败变为本轮 79 通过、21 失败，不据此把改善归因于 B。A、D 在共享目录并行修改 adapters、cli、tui，二进制包含其变化。并发仍未通过，也不能声称 B 已解决整包安全问题。

## 文件所有权自证与剩余风险

`E/before.sha256`、`E/after.sha256` 与 `E/protected-before.sha256`、`E/protected-after.sha256` 保存观测值。Cargo.toml、Cargo.lock、main.rs、lib.rs 一致。adapters.rs、cli.rs、tui.rs 由并行 A、D 改变，本单元没有调用编辑工具修改它们，因此不能声称整个窗口所有白名单外 hash 均相同。`E/domain.diff` 与 `E/engine.diff` 是本单元相对开工副本的精确变更，storage 相同。

未满足原四项全具 RED→GREEN 与完整 crash 恢复的验收标准。前三项已有修复的原始坏版本不在本任务中恢复，不伪造历史 RED。三项真实耦合 RED 按主管决定忽略等 C，C 完成后必须取消 ignore 并复跑。临时日志可能被系统清理。未操作 Git/jj，未核验暂存区，故不将 noStagedFiles 声称为 true。

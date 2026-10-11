# C2 SIGKILL 身份恢复记录

## 结论与 checkpoint

本单元完成真实 RED 与协议设计，未实施身份恢复协议。两条 kill 测试仍 ignored，单独 --ignored 运行均退出 101。没有取消 ignore 或放宽断言假造 GREEN。按任务的 20% 上下文边界，材料、复现与设计后交新鲜实施单元。完整实现、接口迁移、对抗测试及多阶段验证预计超预算。

已完整加载 poteto-mode、pre-implement，读取 A/B/C1 notes、AGENTS/CLAUDE、docs/agents、GLOSSARY、ADR0001 和 unknowns-map 安全入口章节。禁止派发，因此 how/architect 角色串行处理。备份与原始日志目录 E 为 `/tmp/packcaptain-c2-ElXJiC`。六个白名单源文件已逐字节 cp/cmp 备份。产品源文件无修改，仅 tests/recovery_real.rs 增加可选证据保存。

## 执行清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. 已分别执行两条指定真实 kill 测试。
2. Binary-search the cause. 已核查 NativeAdapters 返回和 engine journal 更新顺序。
3. Plan the fix. Ground、Sketch、Agree 在单进程完成，比较两种完整协议。Implement 留新鲜单元，不进行无验证草稿实现。
4. Verify on the same surface. 本单元只完成 RED 与基线验证，没有恢复 GREEN。
5. Stage the commits so the failing repro lands before the fix in git history. skip，用户禁止 Git/jj。
6. Run Opening a PR. skip，用户禁止提交发布。

吞吐 checkpoint 是 RED 和设计后核算上下文，超预计预算就保存精确方案。不跑五轮 verify.js。

## RED 与真正 kill 位置

forward 的 PID 74301 收到 signal 9，marker 为 `real native mutation completed`，SQLite reopen 后有 1 条 Applying journal。entries_after 是 pending，两条真实 symlink 已存在。fresh CLI 退出 1，报 `managed entry appeared externally before creation`。

rollback 的 PID 74336 收到 signal 9，marker 相同，SQLite reopen 后有 1 条 RollingBack journal。C1 已把 forward disable 实际 missing metadata 持久化，inverse 已创建两条新 symlink，但新指纹尚未写入 journal。fresh CLI 退出 1，报 `disabled managed entry reappeared externally`。

这两个 gate 位于 InterruptingNative 的 native.apply_target 返回后、wrapper 向 engine 返回前。不是 OS symlink syscall 中途。NativeAdapters 已完成创建和实际 fingerprint 收集，engine 的 finish_forward update_journal 或 rollback_journal commit 尚未发生。实际跨越的是完整 FS mutation/adapter metadata 返回与持久 journal metadata 之间。

E/forward-74301 与 E/rollback-74336 是 SIGKILL 后、fresh CLI 恢复前复制的完整隔离 fixtures，包含 SQLite、marker、child stdout/stderr、command 和 symlink。原绝对路径随后由 TempDir 清理，复制 fixture 的 symlink 仍指向原路径，不能直接当可重跑 fixture。SQLite payload 和原始日志是永久取证内容，inode 的原值已在持久 entries 和日志保存。

## How 结论

prepare_target 只生成计划，首次创建的指纹是 pending。engine insert_journal 先持久 Applying。NativeAdapters 严格核验 before/after identity，再执行 link changes，末尾读取真实指纹。finish_forward 在整个 executor 返回后更新 journal。rollback_journal 使用 C1 的 actual entries 作为 before，原 entries_before 作为 inverse 计划，inverse 返回后直接 SQLite 原子提交。

A 的严格边界是对的。canonical source 相同不能证明 entry 所属；pending 与 missing 不能认领现存链接。C1 的错误 actual proof 也应保留。仅增加重试或 recovery 分支不产生缺失的历史证据。

## 两种候选与选择

候选一是在每个 mutation 后、adapter 返回前持久 actual receipt。可关闭目前 wrapper gate 对应窗口，但 symlink 创建与 receipt 写 SQLite 之间仍可被 SIGKILL。fresh CLI 在那个窗口仍无法区分外部同源入口。拒绝把这个方案描述为完整 crash 协议。

候选二是持久 staging inode pin。先在非公开的同一文件系统私有目录创建 symlink，保留它作为 anchor。把 anchor 的 no-follow dev/inode、原始 readlink、source、destination、操作 ID、方向和 entry ordinal 持久化，再用 hard_link(anchor, destination) no-clobber 发布同一个 symlink inode。保留 anchor 至 SQLite target commit 完成。选择二作为新鲜单元的实现起点。

本机 macOS 已实测 Rust std::fs::hard_link 对 symlink 自身建立硬链接，entry 和 anchor dev/inode 相同，目标仍为 symlink；第二次发布拒绝 clobber；删除公开入口后重建同源 symlink 的 inode 不同。E/pin-proof.rs 为可复跑脚本，pin-build/pin-proof exit 均 0。样本 dev=16777233，anchor inode=160489573，replacement inode=160489576。这只证明本机语义，不泛化成 Windows 或所有文件系统支持。

## 精确实现建议

### 数据形状和接口

新增 engine 公共类型 `RecoveryDirection { Forward, Rollback }`、`MutationReceipt` 和窄 `ReceiptWriter` 边界。receipt 保存 operation ID、direction、skill/route ordinal、path、source、anchor path、no-follow dev/inode、readlink 和 action。创建 action 至少区分 PreparedCreate 与 VerifiedApplied；删除以原 owned fingerprint/pin 为证据。不要仅保存 done 布尔值，也不要把 receipts 覆盖进 planned entries。

TargetMutation 增加可选 journal context，包含 operation ID、direction 和 ReceiptWriter 引用。正常 engine forward 和 inverse 都传入。直接 adapter 测试无 context 时继续严格现有路径。InterruptingNative 传递完整 m，因此 native 在原 gate 前已有 durable receipt，不改变 wrapper kill 时机。可另加默认严格的 recovery executor 入口，NativeAdapters 在 receipt context 下启用恢复，普通 apply 不接受 canonical-source/pending 认领。所有 TargetMutation 字面量统一迁移，禁止让 wrappers 默认方法绕开真实拦截。

storage 可新增独立 additive receipt table，用 operation ID、direction、ordinal 作唯一键。保持 app_state envelope version=1 可读，journal 增 `#[serde(default)] identity_protocol: Option<u32>` 或等价协议标记。旧 journal 不含协议标记时仍只用实际 fingerprint 严格验证；旧 pending 或身份不匹配没有 receipt 必须 NeedsRecovery。receipt table 比把 receipts 放在 journal payload 更易避免 stale journal.update 覆盖 adapter 的逐 entry 更新。若选择内嵌，则 storage 更新必须事务合并且有单调阶段约束，不能最后把旧 in-memory receipt map 覆盖回去。

### 创建协议

1. state lock 内验证原 before 和 source，安全创建同 FS 私有 staging 目录。不能经 symlink parent 写入，目录权限和路径需要明确验证。
2. 创建并持久 staging anchor，读取 no-follow identity 与原始 readlink。未发布前 anchor 无 receipt 的崩溃最多遗留 orphan，不授权认领公开入口。
3. SQLite FULL 同步持久 PreparedCreate receipt，确认 commit 成功后才能发布。
4. hard_link(anchor, destination) no-clobber。目标已存在时只在 recovery 且 receipt 的 anchor 与目标 no-follow dev/inode/readlink 均匹配时接受。外部同源新 symlink、foreign 目录、不同 source、缺失或被替换 anchor 都拒绝。
5. 使用公开 path 计算原有 fingerprint_entry，不能拿 anchor path 的 fingerprint 代替，当前 digest 包含 path。返回实际 SkillEntry，保留 C1 actual failure 契约。
6. 多 entry partial failure 的逆向处理也需要 receipts，不能调用旧 set_managed_link 绕过协议。失败无法证明完整实际 state 则 None/NeedsRecovery。

### 删除、rollback 和重放

删除前对原 owned link 建立 identity pin，记录原 dev/inode/readlink 和 strict fingerprint 后才 unlink。目标原 inode 已 absent 时可继续收敛至 absent，但不能删除后来出现的同源不同 inode。验证到 unlink 仍有 A 已披露的非协作 writer 微窗口，不能承诺原子 compare-and-unlink。

rollback 使用独立 direction receipts，不能拿 forward 的 create/delete 凭证授权 inverse。C1 实际 missing entries 持久后，inverse creation 使用新的 staged inode pin。rollback SIGKILL 后 fresh CLI 用 rollback receipt 证明新入口，再提交 before intent 和新真实 fingerprints。不要再次拿旧 entries_before inode 当新链接身份，也不要让前向 planned entries 与逆向 actual entries混淆。

SQLite target commit 与删除 journal 可同事务删除 receipt rows，也可保留完成标记待清理。anchor 清理必须在 commit 后，失败最多留下 orphan，不能影响已提交 ownership。receipt 丢失时公开入口仍未提交则保守停 NeedsRecovery。禁止清理非本次 ID 或未证明归属的 staging 内容。

### 新鲜单元验证矩阵

保留原两 kill gates 和断言，实施后取消两项 ignore。fresh CLI 必须重开 SQLite、恢复并再次 retry。增加创建 receipt 后发布前、发布后 metadata 前、部分 entry 完成、inverse 创建后以及删除后 receipt 前的真实 kill 阶段。

专门对抗测试在 SIGKILL 后保留 anchor，移走公开 symlink，再创建 canonical source 完全相同的新 symlink。fresh CLI 必须拒绝，intent 和 journal 保留，外部 entry 不被删除或认领。另测 foreign directory、anchor 被替换、旧 pending 无 receipt、缺失 receipt、source 变化。继续执行 A 六项和 C1 无 actual proof/真实 rollback 测试。

## 非协作 FS 与剩余微窗口

pin 持有 inode 可阻止普通 unlink/recreate 的 inode 回收冒充，解决 mutation 到 journal metadata 的身份断层。它不提供任意非协作 writer 的原子 unlink CAS，也不保护同 UID 恶意篡改私有 anchor/state。外部 writer 需暂停或遵守同一排他锁，这与 A 的限制一致。公开入口与 anchor 可同时遭外部修改时要拒绝，不从匹配 source 推断原 owned identity。

同 FS symlink hard-link 不支持时不能 silent fallback 为 copy/symlink recreate。应在公开 mutation 前安全失败。SIGKILL 回归与机器断电不是一回事。SQLite FULL 外加 staging/parent directory fsync 的平台行为需要实施单元明确；没有验证 power-loss durability 就不能宣称它。

## 本单元命令与证据

| 命令 | 退出 | 结果 | 证据 |
| --- | --- | --- | --- |
| 单独 forward kill，--ignored --exact --test-threads=1 --nocapture | 101 | signal9、原 gate、journal1、fresh CLI RED | E/red-killed_forward_process_recovers_and_retry_converges.log/.exit |
| 单独 rollback kill，同 flags | 101 | signal9、原 gate、RollingBack journal1、fresh CLI RED | E/red-killed_rollback_process_recovers_recreated_fingerprints.log/.exit |
| rustc pin-proof.rs，执行真实 symlink hard_link sketch | 0/0 | no-follow inode pin、no-clobber、replacement 区分 | E/pin-build.log/.exit、E/pin-proof.log/.exit |
| cargo test --test adapter_replacement_safety | 0 | 6 passed、0 ignored | E/replacement.log/.exit |
| cargo test --test recovery_real -- --test-threads=1 | 0 | 10 passed、2 ignored | E/recovery.log/.exit |
| cargo test --all-targets | 0 | 52 passed、2 ignored | E/all-targets.log/.exit |

全量本次没有跳过或隐藏任何测试，tui_extended_pty 的 Skill089 已知不稳定失败本次未复现。不能由一次通过宣称已修复。crash_child 无 CHILD 时只返回，不计作独立产品恢复证据。

被测 CLI SHA-256 `e9f3a626b9a0c49badd015cf55a8797241404b81ab36261284522c788d4b3806`。E/binary-red.sha256、binary-final.sha256 的 cmp exit 为 0。

E/source-before.sha256 与 source-final.sha256 只有 tests/recovery_real.rs 变化，E/recovery_real.diff 已审阅。新增打印 gate/journal，并在 PACKCAPTAIN_RECOVERY_EVIDENCE 指向新 tmp 时复制 fixture，不改变 gate/断言。定向 hook 格式化只修改该测试文件，未见越界。未操作 Git/jj，不核验暂存区。没有读取真实 auth、改真实 agent 配置或删除已有文件。临时 HOME/XDG 隔离，Rust 只复用显式 cache。

LSP 对已编辑测试诊断0，但 push-only 状态 inconclusive，不能当 confirmed clean，cargo 编译与真实测试为证据。

E/runtime-evidence.log 从本 run transcript 提取首条 assistant metadata，provider=cliproxy、model=gpt-6.1-sol、api=openai-completions。当前 artifact 无 model/effort change events，effort 无有效 session 证据，上游实际 reasoning effort 未核实。不以派发参数代替验证。

应用 Model the Domain，采用 journal-scoped typed receipts，不添加模糊 pending 认领分支。应用 Make Operations Idempotent，逐 phase 列明 fresh recovery 的证据和拒绝规则。应用 Prove It Works，先跑真实 SIGKILL 与 pin sketch 再作结论。应用 Guard the Context Window，设计 checkpoint 后停止实施并保存可接续方案。

本单元未达到 C2 GREEN/零ignore验收，不宣称整版完成。

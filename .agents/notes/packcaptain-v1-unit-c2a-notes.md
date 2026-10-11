# C2a journal-scoped receipt 持久边界

## 范围与执行清单

只增加类型和 SQLite API，不接 adapter，不改 TargetMutation、ApplyFailure 或 recovery 行为。两项 SIGKILL 仍 ignored，不宣称完整 crash 修复。

1. `how` over the affected subsystem. 已读 storage、engine、C1/C2 notes。StateStore 拥有 SQLite，engine 先 insert journal，adapter 返回后才更新 metadata。
2. `architect` for parallel design exploration. 用户禁止派发，串行比较两种形状。内嵌 journal receipts 会被 stale payload 覆盖，独立 additive table 隔离该写入。选择独立 table。
3. Write the throughput checkpoint as four todo items.
   - Blocking first steps. 先备份白名单并核对 C1/C2 精确契约，再写类型。
   - Independent workstreams. n/a，唯一 writer，类型、storage 和测试按依赖串行。
   - Shared mutable state. receipt 独立 table，保持 journal payload 不变；finish 的跨表不变量由同一 SQLite 事务维护。
   - Smallest safe decomposition. 一个 worker 完成有界持久边界，focused 与 all-targets 后停，adapter 接线留 C2b。
4. Delegate code-writing to a subagent using your configured feature model. skip，用户禁止子派发，本 worker 自有实现并独立审阅。
5. Verify on the matching surface. 已完成真实 SQLite close/reopen 测试、事务失败注入和全量测试。
6. Rebase into small, ordered commits. skip，禁止 Git/jj 操作。
7. If the design is contested, `interrogate` before shipping. n/a，设计范围已由任务指定。
8. Run Opening a PR. skip，禁止发布。

architect 的 Ground、Sketch、Agree、Implement 已完成，Scrap 不适用。持久边界完成即停，接线留 C2b。

## 数据形状与决策

`ReceiptKey` 用 operation ID、direction、全局 route ordinal 定位一条记录。ordinal 在该 journal direction 的全部 skills/routes 中唯一，由 C2b 稳定排序分配，不是每个 skill 重置。skill、target 和 FS 凭证属于 immutable identity。

`MutationReceipt` 保存 key、identity、stage。identity 包含 skill、target、path、source、anchor、no-follow symlink dev/inode/readlink，以及 Create 或带原 owned fingerprint 的 Delete action。PreparedCreate 表示 Create + Prepared，PreparedDelete 同理。仅允许 Prepared 首次写入，再推进 VerifiedApplied。无实际 FS 观察的 storage 不证明调用者凭证真实。

Model the Domain 决定具名 key、identity、action 和 stage。Make Operations Idempotent 决定重复合法写幂等、identity 不可重绑定、阶段只前进。Prove It Works 决定用真实 SQLite 重开和注入事务失败验证，不用编译代替行为证据。

## 兼容与后续协议

app_state version=1 和 TargetJournal payload 不变。旧 pending 无凭证仍走原严格恢复，可能 NeedsRecovery。adapter 后续必须先创建私有 anchor，durable save Prepared 成功后才公开 hard_link 或 unlink，再保存 VerifiedApplied；不能只补 mutation 后 receipt。SQLite 不做 anchor 清理或真实 FS 删除。commit 后 anchor 清理留 C2b。

## 证据

备份和日志目录 `/tmp/packcaptain-c2a-u45EFG`。engine、storage、storage_roundtrip 已 cp/cmp。全源 SHA 基线已保存。HEAD 文件字面 SHA 为 `e8a06674fbc58be4bf14f25cde8893343d02f5aa`，未执行版本控制命令。

## Deviations

how/architect 的多个角色由本 worker 串行完成，遵守禁止派发。既有 finish API 从 pub(crate) 调整为 pub，供外部 SQLite 集成测试直接调用；增加 operation 存在与 target 匹配 guard，所有既有正常 engine 调用保持不变。没有 TargetMutation 签名扩散。

## C2b 可调用 API

类型均在 `packcaptain::engine`，都有 serde 与 Eq。方向和阶段序列化为 kebab-case。

- `RecoveryDirection::{Forward, Rollback}`。
- `ReceiptKey { operation_id: String, direction: RecoveryDirection, route_ordinal: u32 }`。
- `SymlinkIdentity { device: u64, inode: u64, readlink: PathBuf }`。来自 symlink_metadata，不跟随链接。readlink 是原始值，不 canonicalize。
- `ReceiptAction::Create` 或 `Delete { owned_fingerprint: String }`。JSON action 使用 kind tag。
- `ReceiptIdentity { target: TargetId, skill: SkillId, path: PathBuf, source: PathBuf, anchor: PathBuf, no_follow: SymlinkIdentity, action: ReceiptAction }`。
- `ReceiptStage::{Prepared, VerifiedApplied}`。
- `MutationReceipt { key: ReceiptKey, identity: ReceiptIdentity, stage: ReceiptStage }`。
- `ReceiptWriter::save_receipt(&self, receipt: &MutationReceipt) -> Result<(), StorageError>`，由 StateStore 实现。调用者需 import ReceiptWriter，可传 `&dyn ReceiptWriter`，无需给 TargetMutation 本轮添字段。
- `StateStore::load_receipt(&self, key: &ReceiptKey) -> Result<Option<MutationReceipt>, StorageError>`。精确查 key，journal 不存在则 None，不枚举或借用其他 route 凭证。
- `StateStore::commit_target_and_finish_journal(&mut self, target: &TargetId, state: &TargetState, entries: &BTreeMap<SkillId, Vec<SkillEntry>>, operation_id: &str) -> Result<(), StorageError>`。既有方法现为 public。
- `StorageError::Receipt(String)` 表示 journal 不存在、target 不匹配、identity 重绑定、首次非 Prepared、stage 倒退或未知 stage。实际 SQL 和 serde 错误沿用 Sqlite/Json。

首次 save 只接受 Prepared。相同 key 的全部 identity 字段不可改变，阶段允许 0 到 1 或相同值，1 到 0 返回错误且保留现值。即使 Prepared 与 VerifiedApplied 的调用者对象不同，也必须逐字段相同 identity。stage 不授权新的 inode。删除后的 inverse create 必须使用 Rollback key，不能改 forward action。

## SQLite schema 与事务

`StateStore::open` 在 WAL、synchronous=FULL 下 additive CREATE TABLE，不升级 app_state envelope。

```sql
CREATE TABLE IF NOT EXISTS recovery_receipts (
    operation_id TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('forward', 'rollback')),
    route_ordinal INTEGER NOT NULL CHECK (route_ordinal BETWEEN 0 AND 4294967295),
    identity TEXT NOT NULL,
    stage INTEGER NOT NULL CHECK (stage IN (0, 1)),
    PRIMARY KEY (operation_id, direction, route_ordinal)
);
```

identity 是 ReceiptIdentity JSON，stage 0 为 Prepared，1 为 VerifiedApplied。无 foreign key cascade，生命周期由 API 的 immediate transaction 检查和 finish 清理维护。save 用 `Transaction::new_unchecked(&Connection, Immediate)` 满足共享借用的窄 Writer 接口，此 API 不暴露 Connection 或嵌套事务。事务内读取 journal 并检查 target，检查既有 identity/stage，再 upsert，仅更新 stage。commit 返回成功才表示持久 save 完成。错误退出会 rollback。

finish 在一个 immediate transaction 中检查 journal 与 target，读取最新 snapshot，更新该 target/entries，删除该 operation 所有方向和 routes 的 receipt，再删 journal，最后 commit。任何步骤失败不能部分提交。测试 trigger 在最后删 journal 时 ABORT，已经执行的 snapshot 写和 receipt 删除均 rollback；关闭重开后检查原 snapshot、journal 和两个方向凭证完整。别的 operation 保留。

update_journal 不触碰 receipt table。测试重写旧 journal payload 后 receipt 仍完整。旧 snapshot 缺少 projects/entries 仍可读，旧 journal payload 无新增字段、字面保持不变。本轮不增加 identity_protocol，后续接线若需版本标记必须另设计兼容，不把无 receipt 的旧 pending 作为证明。

storage 只验证 key、journal target、immutability 与阶段，不扫描 FS、不验证 inode 真实性，也不决定 route 分配。C2b 必须对载入 receipt 的全部 path/source/skill/action 和预期 route 做严格比较，再验证 anchor 与公开 entry。原始路径目前遵循既有 serde PathBuf 格式，非 UTF-8 路径会编码失败，不允许丢失后 silent fallback。

## 实测与审阅

| 命令 | exit | 结果 | 原始日志 |
| --- | --- | --- | --- |
| cargo test --test recovery_receipts --test storage_roundtrip | 0 | 5 passed，0 ignored | E/focused.log、focused.exit |
| cargo test --all-targets | 0 | 56 passed，2 ignored | E/all-targets.log、all-targets.exit |
| 相同 focused，public API doc comments 调整后重跑 | 0 | 5 passed，0 ignored | E/focused-final.log、focused-final.exit |
| 相同 --all-targets，最终源码重跑 | 0 | 56 passed，2 ignored | E/all-targets-final.log、all-targets-final.exit |

E 为 `/tmp/packcaptain-c2a-u45EFG`。真实 TempDir SQLite close/reopen 覆盖 literal Create/Delete 凭证、operation/direction/route 隔离、重复写、全部 identity 字段重绑定拒绝、阶段不倒退、旧 snapshot/journal、stale payload 写、原子 finish 与失败 rollback。没有 mock storage。两项 ignored kill 未运行或改动，不能将此矩阵当作完整 crash 证明。crash_child 无 CHILD 的 helper 也不计作独立恢复证明。

两次 all-targets 的 PTY Skill089 本次都通过，不代表已修复间歇失败。无并发、PTY 或 adapter 修改。最终 CLI SHA-256 为 `d51a227591351b9530e48f2eba1c6877ba3f839df00ab698b8c3a92f842bd4ed`，见 E/binary-final.sha256。source-before/source-final 与 diff 保存真实变化，只有 src/engine.rs、src/storage.rs 和新增 tests/recovery_receipts.rs；storage_roundtrip cmp 保持相同。禁区文件 SHA 未变。没有全仓 fmt 或 clippy --fix。hook 定向格式化已重读，无越界。

LSP 主动检查三源文件，0 diagnostics 但 3 inconclusive，不当作 confirmed clean。Cargo 编译与 SQLite 测试为实际验证。独立 diff 审阅无 blocker。no-comments 串行审阅保留 public contract doc comments，没有新增叙述注释或 lint suppressions。未提交、推送、发布或核验暂存区。所有测试 HOME/XDG 指向新 E/home、E/state，仅复用显式 Rust cache。

E/runtime-evidence.log 来自本 run 的 ca608d40 child transcript。首条和后续 assistant metadata 均为 cliproxy/gpt-6.1-sol、openai-completions、thinkingLevel=high，仅证明 Pi 本 session metadata。上游真实 effort 无法核实，不从派发参数推断。没有读取 auth 或真实 agent 配置。

## 残留风险

C2a 不做 anchor 创建、公开 hard_link、删除身份 pin、fsync 或恢复接线。真实创建到 engine metadata 的两个 SIGKILL 窗口尚未修复。anchor 清理需 C2b 在 SQLite commit 后完成，不能提前删除。Prepared 的持久化顺序是调用协议，storage 不能阻止调用者先操作 FS；后续 adapter 与真实 kill gate 才能验证它。非协作 writer 的 compare-and-unlink 微窗口、跨平台 symlink hard-link 支持、掉电 durability 尚未解决或验证。

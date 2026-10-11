# C2b rollback staged create

## 执行清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. 已运行原 ignored exact rollback SIGKILL，exit101。
2. Binary-search the cause. 当前 inverse 不传 context，fresh CLI 正确拒绝已公开的新 inode。
3. Plan the fix. how/architect 的 Ground、Sketch、Agree 串行完成，用户禁止子派发。实现后再验证。
4. Verify on the same surface. 待完成。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止版本控制。
6. Run Opening a PR. skip，禁止发布。

## 数据形状与 checkpoint

复用 ForwardReceiptContext，direction 作为 receipt key 与 anchor namespace 的组成。engine 用同 operation ID、before/after union BTreeSet ordinal 给 inverse 构造 Rollback context。保留现有公开类型名称以免扩大迁移。旧 forward anchor 路径不变，rollback 用独立目录。

how 已核对现源码。engine 持久 C1 missing proof 后进入 RollingBack。NativeAdapters 在 apply_target 验证 routes，再用 anchor create，返回 public path 的真实 fingerprint。storage 独立 table 精确读写且 identity immutable。wrapper 完整转发 mutation，所以不能另建入口绕过 gate。

architect 串行比较两种形状。独立 rollback API 会绕开原 wrapper，且重复 staged protocol。通用 context direction 参数复用已验证的协议，不新增 Delete pin。选择后者。Model the Domain 决定方向隔离。Make Operations Idempotent 决定只接受同 inode 的已发布入口。Prove It Works 决定先真实 RED，再保持原 gate、signal9、fresh CLI 和 retry 断言验证。

checkpoint 为备份/RED、接线/GREEN、对抗/focused、all-targets/交付。唯一 writer，seam 完成即停，不处理并发、PTY、更多 kill 阶段和跨平台。

证据目录 E 为 `/tmp/packcaptain-c2b-rollback-ZHoX6b`。白名单已 cp/cmp。src/tests 字节 SHA 基线保存。HOME/state/agent-home 均新建临时目录。RED 保存 red.log 与 red.exit，signal9 与原 marker 已观察。

## Deviations

禁止 fanout，how/architect 由本 worker 串行比较。无产品范围变更。

## 实际交付

engine 的 receipt_context 接 direction 参数。forward 调用不变，inverse 传 Rollback。ordinal 仍来自 before/after routes 并集，读写仍 StateStore exact API。ForwardReceiptContext 的公开名称保留，不增加调用者迁移。adapter 的 receipt loader 与 anchor helper 改通用命名。forward 目录保留旧字面名称，rollback 独立 `.packcaptain-<operation>-rollback-<ordinal>/anchor`。receipt identity 不修改，不能用 Forward key 授权 inverse。

复用原 staged create，独占目录、no-follow inode/readlink、same-FS probe、Prepared SQLite commit、no-clobber hard_link、VerifiedApplied 顺序未改变。C1 forward disable 的 actual missing entries 是 inverse before，逆向创建返回公开 path 的新真实 fingerprint。commit 前 exact-load 两方向可清理 receipts，SQLite 成功 commit 后才清本 operation 的匹配 anchor。foreign 内容不递归删除。没有实现 Delete pin，也没有发现当前回归需要该协议。

原 recovery_real wrapper、native 后 gate、signal9、fresh CLI、retry 字面断言均未改。取消唯一 rollback ignore。首次 focused 暴露既有 C1 人工 journal ID `rollback-with-actual` 不满足已有 staging UUID guard，exit101。仅迁移该 fixture ID 为有效 UUID，missing proof 与所有字面行为断言保持。日志 focused.log 保留，不放宽生产 guard。

新增 recovery_rollback_safety 有7个行为测试和1个 child helper。SIGKILL 后同源新 link、foreign目录、anchor替换、无receipt、双方向互借都 fresh CLI 拒绝，旧 intent/entries 与 journal保留，foreign inode/content不变。VerifiedApplied恢复断言 fresh CLI 与 retry 复用原 receipt inode，返回 fingerprint 与 discovery一致。Prepared测试移除 public path并回退真实SQLite stage，证明发布与双方向commit cleanup，保留foreign文件。双方向cleanup中的forward row与anchor由测试构造，不冒称真实forward失败链路；Prepared不是发布前真实kill证据。child无环境仅返回，不计行为证明。

| 命令 | exit | 结果 | E 下证据 |
| --- | --- | --- | --- |
| 原 rollback --ignored --exact --test-threads=1 --nocapture | 101 | signal9 后 fresh CLI 拒绝新 inode | red.log / red.exit |
| 取消 ignore 后同 exact，不带 --ignored | 0 | 原 gate/signal9/fresh CLI/retry | green.log / green.exit |
| 六 focused suites，最终源码 --test-threads=1 | 0 | 38 passed，0 failed，0 ignored | focused-delivery.log / .exit |
| cargo test --all-targets，新增最后两测试之前 | 0 | 71 passed，0 ignored，PTY均执行 | all-targets.log / .exit |
| cargo test --all-targets，最终源码 | 101 | 已执行71 passed，1 failed，0 ignored | all-targets-delivery.log / .exit |

最终唯一失败是 tests/tui_extended_pty.rs:483 Skill089已知间歇，未修、未隐藏，后续tui_pty未在最终运行执行。先前全量exit0已执行它。没有跑verify5。binary-delivery.sha256为 `0cde55e55f7b24c40e60d71988c1df4621ed60ac1cc6aa22c941f65ebef27434`。最终src/tests SHA只改变engine、adapters、recovery_real、新rollback safety。storage与现有forward/replacement/receipt测试字节未改，禁区src保持相同。diff与新增测试已重读，无blocker。no-comments按禁止派发串行审阅，无新增注释/suppression，无删除或restore。

LSP诊断0，4 inconclusive，非confirmed clean；cargo是实际编译与行为证据。没有全仓fmt、clippy fix、Git/jj、发布、auth读取、真实agent配置写入。定向hook只格式化白名单测试，没有越界。没有核验暂存区。

## Runtime 与残留风险

E/runtime-evidence.log来自当前任务062ec5b2/run-0/session.jsonl，session 01a12bfb。model_change与首assistant为cliproxy/gpt-6.1-sol，thinking_level_change=high，api=openai-completions。仅证明Pi有效metadata，上游真实model/effort未核实。

非协作writer在verify与publish/cleanup unlink之间仍有微窗口。同UID篡改私有目录不在保证内。仅本机Unix/macOS实测，跨平台、其他FS支持、掉电durability未测。没有增加Delete pin，不保证其他删除crash阶段。多route部分失败和更多kill阶段未扩展。Prepared前orphan与commit后cleanup失败可能留目录，保守不扫描删除。seam到此停止，不宣称整版完成。

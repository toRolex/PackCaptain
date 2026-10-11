# C2b forward inode pin 接线

## 执行清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. 已运行唯一 forward 原 SIGKILL 回归，exit 101，gate 与 signal9 保留。
2. Binary-search the cause. native 返回之前 FS 已创建，journal 仍 pending，原安全拒绝正确但无持久身份。
3. Plan the fix. how/architect 串行完成 Ground、Sketch、Agree，禁止派发。独立 recovery 方法会让 wrappers 绕开 gate，故选择 TargetMutation 可选窄 context，复用 apply_target。
4. Verify on the same surface. 原 SIGKILL 及对抗 focused 已通过，最终 all-targets 的范围外 PTY 失败完整记录。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止版本控制。
6. Run Opening a PR. skip，禁止发布。

## 数据形状与吞吐 checkpoint

Model the Domain 决定 ForwardReceiptContext 携带 operation、direction、全局 BTreeMap skill/path ordinal 和窄读写接口。StateStore 不暴露 Connection。ordinal 在 before/after routes 并集按 skill/path 排序，不随完成状态变化。

先备份与 RED，再 context/anchor 实现，然后对抗与 focused/all-targets，forward seam 完成即停。唯一 writer。依赖串行，不增加 rollback/delete 协议。失败无法提供完整 actual proof 时保留 NeedsRecovery。

私有同 FS 目录位于 skill root 的父目录，避免 discovery 扫描未提交 anchor。目录独占创建且 0700，anchor symlink 的 hard-link probe 在公开 mutation 前验证支持。Prepared commit 后 no-clobber 发布；anchor/public no-follow dev/inode/readlink 一致才接受。commit 前收集清理凭证，commit 后仅删除 identity 与 operation/path 均匹配的 anchor，失败不改已提交状态。

## 证据路径

E 为 `/tmp/packcaptain-c2b-forward-h7EEki`。白名单 src/engine.rs、src/adapters.rs、src/storage.rs 与现有 tests 已 cp/cmp。source-before.sha256 保存禁区基线。red.log/red.exit 保存原始 RED。HOME、state、agent-home 全为新临时目录。未运行 Git/jj。

## Deviations

用户禁止 fanout，how/architect 两个结构候选由本 worker 串行比较。无产品范围变更。

## 实际实现与验证

engine 每次 forward apply 传完整 context，wrapper 原样传 m，因此 native 完成后 gate 不被绕开。直接 adapter 字面量填 None，继续 A 严格路径。rollback mutation context 仍 None，只有完成 SQLite commit 的共用入口清理本 operation forward anchors，不增加 inverse receipt 协议。storage 仅增加 ReceiptReader trait 实现，复用 C2a exact load。

创建在父目录独占生成私有 staging，symlink_metadata 与 read_link 记录原始身份。probe 验证 hard_link symlink 语义、dev 与公开 parent 相同，sync staging 和父目录后持久 Prepared。随后公开 no-clobber hard_link、no-follow 校验、sync parent、VerifiedApplied，返回公开 path 原有 fingerprint。路径 digest 含 path，不能用 anchor fingerprint。

对已载入 receipt，operation、direction、ordinal、target、skill、path、source、anchor、action 全匹配才校验真实身份。route 的 source 也需匹配 skill。已 Verified 的公开入口 missing 不自动重建。Prepared + absent 可以用仍验证通过的 anchor 收敛。pending 无 receipt 不能认领公开入口。部分失败不执行旧内部 inverse，而返回无完整 actual proof，engine 保留 NeedsRecovery。C1 wrapper 提供 actual proof 的原行为保留。

commit 前 exact-load 可清理元数据，SQLite commit 成功后只删除 operation/ordinal 路径与 no-follow identity 匹配的 anchor。目录仅 remove_dir，不递归。存在非本 operation 文件时保留目录。清理失败不返回错误，不把 commit 改为失败。

第一次接线 fresh recovery 成功但原 retry entries 等值断言失败。managed route 新建时 native_config_path=None，后续 discovery 填 Some。仅 forward 返回填入已有 native config 路径和实际文件 fingerprint，原断言未变。第一轮 all-targets 发现新增 safety gate 文件 exists 与完整内容之间的测试竞争，exit101。改新增 helper 为写 gate-ready 后 rename 原子发布，不修改原 recovery_real wrapper 或 gate。

| 命令 | exit | 结果 | 原始日志 |
| --- | --- | --- | --- |
| cargo test --test recovery_real killed_forward_process_recovers_and_retry_converges -- --ignored --exact --test-threads=1 --nocapture | 101 | 原 gate、SIGKILL9，fresh CLI 拒绝 pending | E/red.log、red.exit |
| 相同指定测试，取消仅 forward ignore 后，不带 --ignored | 0 | 原 gate、SIGKILL9、fresh CLI 与 retry entries 全等 | E/green-final.log、green-final.exit |
| focused 指定 recovery_forward_safety、recovery_real、adapter_replacement_safety、recovery_receipts、storage_roundtrip，--test-threads=1 | 0 | 最终源码29 passed、0 failed、1 ignored | E/focused-delivery.log、focused-delivery.exit |
| cargo test --all-targets，中间完整验证 | 0 | 64 passed、0 failed、1 ignored | E/all-targets-delivery.log、all-targets-delivery.exit |
| cargo test --all-targets，最后源码验证 | 101 | 已执行62 passed、1 failed、1 ignored，仅 PTY Skill089 失败 | E/all-targets-accepted.log、all-targets-accepted.exit |

最终失败位于 tests/tui_extended_pty.rs:483。tui_pty 在该次全量未执行，先前完整 exit0 已执行。known PTY 间歇未修复，不隐藏，不跳过。rollback kill 仍为唯一 ignored。crash_child 和 forward_safety_child 无环境变量仅返回，不计独立产品恢复场景。没有运行 verify5。

新增 safety 六项真实 FS 场景包含 crash 后公开 link 移走并新建同源 symlink、anchor 替换、receipt 删除、旧 pending 无 receipt、Prepared publish、commit cleanup 保留 foreign 文件、identity 各字段拒绝。身份字段测试多分支共一项。均 fresh CLI，保持历史 intent 和 entries，不误删外部入口。

最终 CLI SHA-256 为 `7816ff9d9c568fd7e742444b4d184142619fd085dd860adb0af0f5ec905551f8`。E/binary-accepted.sha256 对应最后 focused 和 all-targets。source-accepted.sha256、boundary-check.log 与逐文件 diff 已审阅。只有七个白名单源测试文件变化，禁区 src SHA 不变。另写本 notes。没有暂存区核验，用户禁止 Git/jj。两项 literal migration 仅定向 rustfmt，不做全仓 fmt 或 clippy --fix。hook 两次机械 let-chain 调整仅白名单，无越界。

LSP 最终 warning 级诊断0，七项 inconclusive，不当作 confirmed clean。实际 cargo 测试是编译与行为证据。no-comments 按禁止 fanout 串行审阅，无新增注释或 suppressions。Model the Domain 决定 context/typed receipts。Prove It Works 决定保留原 gate、signal9 和字面 retry 断言。

## runtime 与剩余风险

E/runtime-evidence.log 从本 run c5ad0ae2 transcript 摘录首条 assistant metadata，provider=cliproxy、model=gpt-6.1-sol、api=openai-completions、thinkingLevel=high。仅证明当前 Pi session metadata。上游真实 model/effort 未核实，不根据派发参数替代证据。无 auth 或真实 agent 配置读取。

rollback/delete pin 协议未实现，rollback kill 留 C2b-rollback。非协作 FS writer 在 verify 与 hard_link、cleanup unlink 之间仍有微窗口。同 UID 恶意篡改私有目录不在保证内。仅本机 Unix/macOS 实测，跨平台/FS 支持未验，unsupported 平台或 pin probe/dev 不一致在公开 mutation 前拒绝。目录 sync 与 SQLite FULL 是顺序机制，不代表已完成掉电验证。

公开 mutation 前的 orphan staging 和 commit 后清理失败可能留目录，不能自动遍历删除。Prepared发布前真实SIGKILL更细窗口、多个route部分失败、commit后立即SIGKILL未单独测试。Prepared恢复测试通过改真实SQLite stage并移除公开path构造，不冒称其为真实发布前kill证据。forward seam 到此停止，不称整版完成。

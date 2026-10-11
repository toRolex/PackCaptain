# C3 concurrent enable

## 清单与 checkpoint

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 已备份 src/tests，verify1 真实 RED。
2. Binary-search the cause. 已确认锁外 discovery 在锁内覆写最新 entries 与 observed_enabled。
3. Plan the fix. how/architect 由唯一 writer 串行比较两个候选，禁止子派发。
4. Verify on the same surface. 真实双 CLI barrier 五轮、focused、verify5、all-targets 均已完成。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止 Git/jj。
6. Run Opening a PR. skip，禁止发布。

数据形状沿用 DiscoveryReport、Snapshot 与 StateLock。没有新的 ownership 标志。Fix Root Causes 决定修复扫描时序而非接受 unowned 同源链接。Model the Domain 决定以 snapshot 到 discovery 的闭包表达扫描依赖。Prove It Works 决定新增真实进程和真实 SQLite/links 检查。

checkpoint 为备份/RED、接口/GREEN、focused/verify5/all-targets。enable seam 完成即停。吞吐取舍为串行依赖执行，禁止 fanout 与动态 workflow，不重调研业务。

## how 与设计

CLI discovered 读取旧 snapshot 后扫描 FS。engine preflight、锁内 preflight、恢复后 merge 都沿用该 report。即使锁内 DB 已有另一 CLI 的 actual owned fingerprints，旧 report 仍覆盖它。第二条 enable 的 prepare 将刚创建的 shared link 视为 unowned，严格拒绝正确，输入 freshness 错误。

候选一为向 CLI 暴露 lock guard 并让 engine 接受已持锁入口。它扩大锁权限接口并容易重入锁。候选二为 engine 内部统一执行闭包，原 execute_with_discovery 闭包返回静态 report，新 native 入口从最新 snapshot 扫描。选择二。CLI 只改 run_mutation 的执行调用。锁前只读 preflight 保持错误命令不恢复 journal；锁取得后重读 snapshot、重新扫描、再次 preflight；恢复后再扫描最新 snapshot，保留 B 对 recovered targets 的持久 metadata 优先。

锁仍是 state-dir/packcaptain.lock，覆盖同 state-dir 的合作进程，从 locked snapshot/scan 到 prepare/apply/journal/commit/cleanup。多个 state-dir 操作同 target 未覆盖，不承诺。外部 writer 仍遵守 A 的边界，无放宽 fingerprint、pending、unowned、receipt 校验。原静态 report API 保留给已有测试/TUI，不能声称其一般 stale input 已解决。CLI 初始 scan 仍供构造命令和 preview，非实际执行的权威扫描。

## 初始证据

E 为 `/tmp/packcaptain-c3-enable-0oayFm`，路径也记录在 /tmp/packcaptain-c3-enable-current。E/before/src 与 E/before/tests 为逐字节备份，E/source-before.sha256 为边界基线。verify1 exit1，17 passed、3 failed，全部是 enable 及派生状态。原报告 /private/tmp/packcaptain-cli-safety-runs/verify-xZkABn/report.json。disable 在该轮偶然成功不代表已经解决。HOME、agent-home、state、project 使用临时目录。

## Deviations

用户禁止子派发，how/architect 候选串行比较。无范围外修改。

## 实际编辑与验证

仅编辑 src/engine.rs、src/cli.rs、新 tests/concurrent_cli.rs 与本 notes。新增 `execute_with_native_discovery(&mut self, &Command)`，私有 `execute_with_scan` 接受 `FnMut(&Path, &Snapshot) -> DiscoveryReport`。旧公开 execute/execute_with_discovery 签名不变，EffectExecutor/TargetMutation/receipt 无迁移。CLI run_mutation 仅一行转向 fresh native 入口。adapters/storage/domain/tui/main/lib/Cargo 均未编辑，src/tests 既有文件字节比较仅 engine 与 cli 不同。E/changed-existing.log、source-final.sha256、engine.diff、cli.diff 保存证据。

真实回归只在测试进程持有现有 state lock，spawn 两个独立真实 CLI 后等待300ms，两个 try_wait 均 None 才释放。不同 PID、同一存活时刻证明窗口重叠。未加入生产 barrier 或重试。每轮重建独立临时 HOME、agent-home、state、project，断言公开目录恰三个 symlink，canonical sources 精确 a/s/b，重开真实 SQLite 确认两个 presets 均 enabled、enabled_skills 精确三项、entries三项。

RED 为 `round=1 overlap_gate_ms=373 A=None B=None exits=Some(1),Some(0)`，错误原文 `managed skill entry path is occupied by an unowned file`。GREEN 五轮均 `A=None B=None exits=Some(0),Some(0)`，见 E/green-test.log。该修复不放宽 pending/missing 或同源替换保护。

命令使用 `HOME=E/home CARGO_HOME=/Users/rolex/.cargo`。后续focused、all-targets、verify5另设 XDG_CONFIG_HOME 与 XDG_STATE_HOME 到 E/home，Cargo只使用既有缓存。真实 CLI 的 HOME 与 agent-home/state-dir 由各 fixture 再隔离。没有 auth 或真实 agent 配置读取。

| 命令 | exit | 结果 | E 下日志与exit文件 |
| --- | --- | --- | --- |
| node /private/tmp/packcaptain-cli-safety-scripts/verify.js 1 | 1 | 17 passed、3 enable失败 | red-verify.log、red-verify.exit |
| cargo test --test concurrent_cli -- --nocapture，修前 | 101 | 两进程窗口重叠但一条exit1 | red-test.log、red-test.exit |
| cargo test --test concurrent_cli -- --nocapture，修后 | 0 | 五轮双exit0、精确links与DB | green-test.log、green-test.exit |
| cargo test --test adapter_replacement_safety --test recovery_real --test recovery_forward_safety --test recovery_rollback_safety --test recovery_receipts --test storage_roundtrip -- --test-threads=1 --nocapture | 0 | 38 passed、0 failed、0 ignored | focused.log、focused.exit |
| cargo test --all-targets | 0 | 74 passed、0 failed、0 ignored，PTY两suite均执行 | all-targets.log、all-targets.exit |
| node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5 | 0 | 100 passed、0 failed | verify5.log、verify5.exit |
| cmp E/binary-tested.sha256 E/binary-final.sha256 | 0 | test后与verify后二进制不变 | binary-compare.log、binary-compare.exit |

A六项全部通过，forward/rollback原gate与signal9均通过，无ignore。38包含无需子进程环境时只返回的helpers，不能按38声称38个产品场景。all-targets本次known Skill089间歇项通过，先前C2b最后失败仍为已知风险，本单元不修PTY、不跳过测试。

verify5原报告 `/private/tmp/packcaptain-cli-safety-runs/verify-jLD6jI/report.json`，完整副本 E/verify-jLD6jI。red同样保留完整副本 E/verify-xZkABn。每条真实CLI命令、PID、时间、stdout/stderr与exit在events.json，分类位于E/red-classification.json、green-classification.json。enable五轮全部成功且窗口重叠，links与A/B状态五轮全通过。disable A与enable B、repeat B和派生状态本次也全通过，实测剩余disable错误0，不伪造失败。adapters.rs:852原target-local guard字节未变，其未覆盖分支仍留C3-disable，不把这次偶然全绿当作该事项完成。四项replacement关键断言20/20通过，全部外部保护类断言亦通过。

最终 binary SHA-256 为 `9b506711e70a6b32348a7c02a0f2cec5306ccdc64cd5bcf7d9c1e4bf93b8c72b`。binary-tested.sha256、binary-final.sha256 与verify report身份一致。未提交、推送、发布或核验暂存区。未全仓fmt或clippy fix。测试写入hook仅格式化新测试文件，已重新读取，无越界。diff与测试已重读，无blocker。no-comments按禁止派发串行检查，新增注释与suppressions均0，无删除/restore。LSP对三个文件0诊断但3 inconclusive，不能算confirmed clean，实际Cargo是编译证据。

## Runtime 与剩余风险

E/runtime-evidence.log来自当前run `14ec3ea6-6f3e-4065-ab3c-2d4c8e5e9c53_poteto-agent_transcript.jsonl`首assistant metadata，时间2026-10-11T17:33:35.839Z，provider=cliproxy、model=gpt-6.1-sol、api=openai-completions、thinkingLevel=high。只证明Pi session metadata，上游实际model/effort未知。

多个state-dir同target不在锁范围且未测。非协作外部writer仍有A记录的verify到mutation微窗口。静态discovery API与TUI未迁移，不承诺所有caller的一般并发freshness。native入口新增重复scan成本未测，不作性能承诺。仅本机macOS/Unix与此同state-dir双CLI enable场景实测，跨平台、更多并发命令、掉电安全未扩展。disable guard与PTY留后续，不算全量安全或整版完成。临时日志可能被系统清理。enable seam已完成，停止扩展。


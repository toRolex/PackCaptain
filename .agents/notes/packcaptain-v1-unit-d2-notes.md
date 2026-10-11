# D2 TUI freshness

本单元已完成真实 RED 到 GREEN。未宣称整版完成。证据目录为 `/tmp/packcaptain-d2-TxgsEy`，以下称 E。

## 清单与 checkpoint

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 已执行真实 PTY 和外部真实 CLI，red.log 退出101。
2. Binary-search the cause. Confirm 沿用静态 discovery，锁内最新 entries 与 observed_enabled 被覆盖。
3. Plan the fix. how/architect 串行比较两个候选，主管批准严格 Plan 重确认与 pending journal 保守拒绝。
4. Verify on the same surface. 新五项 PTY、原五项要求、focused 和 all-targets 全部执行。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止提交。
6. Run Opening a PR. skip，禁止发布。

吞吐 checkpoint 为备份/RED、产品裁定/接口/GREEN、focused/PTY/all-targets。本单元不子派发，不动态 workflow，不重复 verify5。编辑前 E/before/src 与 E/before/tests 为逐字节备份。

Model the Domain 决定以显式 Command 和已显示 Plan 表达确认对象，不增加 ownership 标志。Fix Root Causes 决定修复扫描时序，不认领 unowned 同源链接。Prove It Works 决定使用真实 PTY、真实 CLI、精确 links 和重开 SQLite。unslop 与 technical-writing 用于记录。no-comments 按禁止派发串行审阅，新增注释、suppression、删除与restore均0。

## how、设计与产品裁定

App.refresh 保存锁外 snapshot/discovery。原 preview_pending 用静态 discovery 合并新读 DB。原 Confirm 把同一 report 交给 execute_with_discovery，锁内克隆旧报告，故覆盖最新 observed_enabled/entries。

候选一仅迁移 native execute，fresh effect 变化会静默执行用户未见过的计划。候选二在 native 执行内部比较已确认 Plan，不同则返回 fresh Plan，TUI保留原 Command 和 targets 再展示。主管批准候选二。锁外 TUI check 再 execute 仍有 TOCTOU，不选。

`execute_confirmed_with_native_discovery` 复用现有 `execute_with_scan` 和 native scan。锁内、StateStore::open 前只读加载 snapshot/journals。存在 pending journal 则拒绝，显示“存在未完成操作，需先完成恢复后重新预览”，不消费 recovery。否则 fresh Plan 严格相等才进入既有执行流程。执行前最后一次 scan 仍比较 Plan。若不等，返回 `PreviewChanged(Box<Plan>)`，在任何本命令业务持久写与FS mutation前返回。StateLock 文件和SQLite读取不属于业务状态写入。

TUI 显示“状态已变化，请重新确认”，第一次确认仅更新内存预览，第二次确认仍执行相同锁内guard。Plan不含 receipt、entry fingerprint 或 source scan timestamp，未观察重复确认循环。每次保留原revision与明确targets，不自行改revision或扩范围。revision冲突明确失败并要求重新编辑。manual-close新冲突走Keep/Restore选择，再预览再确认。partial failure展示保留。

TUI只读预览改用 `preview_with_native_discovery`，不消费journal。CLI native入口不变，仍自动恢复。没有虚构recover命令或建议随意执行写命令。focused里的真实fresh CLI恢复已有测试证明，包括forward/rollback receipts。

## mutation 入口矩阵

| TUI入口 | Command及范围 | 实际mutation路径 |
| --- | --- | --- |
| CreateName/CreateSkills | CreatePreset，选中members，无target effects | Confirm -> execute_confirmed_with_native_discovery -> execute_with_scan |
| Home Enter/e/d | SetPresetEnabled，明确勾选targets | 同一Confirm fresh guard |
| Home u/EditMembers/EditTargets/EditPolicy | UpdatePreset，原expected_revision、明确sync_targets、显式策略 | 同一Confirm fresh guard |
| SkillInventory e/d | SetSkillEnabled，明确勾选targets | 同一Confirm fresh guard |
| ProjectCandidates Enter | 所选项目或用户明确选择的扫描根目录 | 已有claim_project，canonicalize后持锁、重读、recover、save。无静态discovery，无修改 |
| Esc、扫描、列表、搜索、preview | 无mutation command执行 | 只读refresh或preview，pending清空 |

上述五类写入口完整覆盖当前TUI。源码定位为 src/tui.rs:405 的 claim_project、src/tui.rs:632 的唯一 command执行、src/engine.rs:338 和 :362 的确认guard。静态公开API仍留既有测试使用，不承诺其一般freshness。

## RED与GREEN证据

RED 两preset共享Skill000，外部preset另含Skill001。TUI预览A启用一项effect，外CLI启用B exit0，确认A后 UI报 unowned，重开SQLite显示B intent=true但 enabled_skills为空，A intent未落地。red.log 与 red-pty/fresh-shared.bin保留完整原始证据。原输出摘录为 `stored=TargetState { enabled_skills: {}, manual_open: {}, manual_closed: {}, presets: {... enabled: true ...} }` 和 `test result: FAILED. 0 passed; 1 failed; 0 ignored`。

GREEN原输出为 `test result: ok. 9 passed; 0 failed; 0 ignored`。all-targets执行同一真实PTY场景，final-pty/fresh-shared.bin和对应assertions.txt保存证据。第一次确认后SQLite完整Snapshot与外CLI后的状态精确相等，links完全相等，A未落地。第二次确认后A/B intent同时true，enabled_skills与owned entries保留，目录恰两个pc-ID链接，各自指向 canonical Skill000/001，未选两project无链接。

新增另四个真实PTY证明：两次连续外CLI变化分别触发重确认，Esc保留所有业务状态与links；预览后外CLI手动关闭旧member，首次确认零写，必须选Keep再预览确认；预览后foreign目录替换受管link，disable安全失败，foreign内容和完整stored snapshot不变；预览后向真实SQLite插入Applying journal，确认保持payload字节、snapshot及symlink no-follow dev/inode/readlink不变。pending fixture是持久化输入构造，不是mock executor。

原成员增删、精确两个targets、完整取消、partial failure、90项长列表五项要求保持，原tui_pty还验证项目认领、创建、单项开关与显式Restore。

| 命令 | exit | 结果 | 日志 |
| --- | --- | --- | --- |
| cargo test --test tui_extended_pty pty_preview_then_external_shared -- --nocapture，修前 | 101 | 真实RED | red.log、red.exit |
| cargo test --test tui_extended_pty -- --nocapture，初次修后 | 101 | 5 passed，3测试fixture误用skill名称而非pc-ID链接名，未产品放宽 | green.log、green.exit |
| cargo test --test tui_extended_pty -- --nocapture，修正fixture后 | 0 | 8 passed、0 ignored | green2.log、green2.exit |
| focused首次误带不存在的concurrent_disable_cli target | 101 | Cargo未执行测试，随后使用真实concurrent_cli | focused.log、focused.exit |
| cargo test --test concurrent_cli --test adapter_replacement_safety --test recovery_real --test recovery_forward_safety --test recovery_rollback_safety --test recovery_receipts --test storage_roundtrip -- --test-threads=1 --nocapture | 0 | 41 passed、0 ignored，包含3 concurrent tests与helper | focused2.log、focused2.exit |
| cargo test --all-targets | 0 | 81 passed、0 failed、0 ignored，两个PTY suites均执行 | all-targets.log、all-targets.exit |

HOME、XDG_CONFIG_HOME、XDG_STATE_HOME均指向E/home。CARGO_HOME仅使用 `/Users/rolex/.cargo` 缓存。每个新增fixture再次隔离临时HOME/agent-home/state/project。真实 CLI/PTY未读取auth或修改真实agent配置。

RED binary SHA256为 `9b506711e70a6b32348a7c02a0f2cec5306ccdc64cd5bcf7d9c1e4bf93b8c72b`。GREEN与最终all-targets均为 `f81ff905e9a6bb51aacd3d0516a9b30c6672beba78aa3aff726f2f12bbaa1017`。分别见binary-red.sha256、binary-green.sha256、binary-final.sha256。新增late-policy仅测试改动，没有改变该binary。

## 文件边界与残留风险

只改 src/tui.rs、src/engine.rs、tests/tui_extended_pty.rs 和本notes。source-boundary.log、source-before.sha256、source-final.sha256与逐文件diff保存证明。adapters/storage/domain/cli/main/lib和其余tests字节未变，Cargo前后SHA相等。写入hook只格式化被编辑文件，tui既有if-let格式也被格式化，未越界，未运行全仓fmt或clippy fix。每次hook后重读文件。未提交、发布或暂存。禁止Git/jj，本单元未核验暂存区，不能声称当前index为空。

LSP三个文件0诊断但3 inconclusive，不作clean证明。实际Cargo编译与测试全绿。diff已串行重读，无本seam blocker。

known Skill089 PTY间歇本次通过，没有修、skip、ignore或放宽断言。不是间歇已消失的证明。多个state-dir共target、非协作writer在verify到mutation的微窗口、跨平台与掉电仍未证明。foreign回归证明确认前已发生替换的安全拒绝，不覆盖最终微窗口。TUI pending限制需先通过现有CLI合法业务操作完成恢复再重新预览，不自动消费，且未新增recover命令。严格Plan比较可能对无effect的target intent变化要求重确认，这是批准的安全策略。

runtime-evidence.log来自当前run fa55bd2b-bac9-41c8-ba81-802f5d990ad1的首assistant metadata，2026-10-11T17:59:11.690Z，provider=cliproxy、model=gpt-6.1-sol、api=openai-completions、thinkingLevel=high。仅证明Pi runtime metadata，上游真实model/effort未知。临时证据可能被系统清理。seam完成即停。

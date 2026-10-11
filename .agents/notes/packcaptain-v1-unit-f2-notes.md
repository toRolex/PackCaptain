# F2 shared entry

## 状态与 checkpoint

- RED，done。新 E 由 /private/tmp/packcaptain-f2-current 定位。编辑前 src/tests/Cargo 全备份。原 E1 shared-repro 字节不变复跑 exit0，证明缺陷存在，不算产品 PASS。
- 修复，done，仅 F2 seam。src/adapters.rs 与新 tests/shared_entry_transactions.rs。engine 候选额外 commit 后写已撤销，最终字节与备份一致。
- 验收，done。E=/private/tmp/packcaptain-f2-YOQqbN。focused-final2 56/0/0，all-targets-final 98/0/0，verify5-final 100/100，独立安全 probe exit0。非最终安全完成。

## 清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 已复跑原 E1 构造 probe。
2. Binary-search the cause. 已确认 shared 引用优先 planned states，后 target 未 commit。
3. Plan the fix. 单 writer，不派发。how/architect 两候选串行比较。
4. Verify on the same surface. 独立安全 assert exit0。原缺陷存在 assert 修后 exit101，原件字节未变，不称原件 GREEN。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止提交。
6. Run Opening a PR. skip，禁止发布。

## how 与设计

数据形状沿用 Snapshot.targets 的已提交 intent，SkillEntry 的 active 与 present 分别表达 target 逻辑需求和真实入口存在。TargetMutation.after 只代表本 target，planned_target_states 不授权删除其他 target 需求。prepare/apply 位于 per-target journal 内，后 target 可失败，前 target 不回滚。

候选一仅将 planned 换 snapshot，仍保留 prepare shared 拒绝。它会使两 target 全 disable 互相阻塞，拒绝。候选二为引用保留，首 target 逻辑关闭但 present=true，后 target 按更新 committed snapshot 删除最后引用。选择候选二，不引入跨 target 事务。apply 再计算 snapshot 引用，不信任过时计划。discovery 只保留同路径 shared 的已提交 inactive owned retained route。末引用删除后，通过已提交 peer 的 missing fingerprint 与真实 NotFound 校准 inactive retained route 的 present/fingerprint，不改变 ownership/intent。enabled preset 中被 manual_closed 屏蔽的成员不持有引用，否则两个 manual disable 后会永不释放。

Fix Root Causes 决定先原 RED 后修。Model the Domain 决定沿用 active/present，不新增共享布尔。Prove It Works 决定记录固定 artifact SHA、原 log 和 exit。吞吐 checkpoint 为备份/RED、最小协议/安全 assert、focused/全量/verify5。子派发与动态 workflow 因显式禁止跳过。

## 范围

原 probe 的同路径 Codex/Pi ownership 和第二 prepare 失败为构造输入。source/FS/SQLite 与 NativeAdapters 为真实。外部 writer 必须暂停或合作同一 state lock，A 的 native/owned、pending 和 identity 边界不放宽。

## 真实结果与失败保留

logs 保存 command/log/exit/start/finish/binary。shared-repro-red exit0 仅证明缺陷存在。shared-safe-red 是误调用不存在 API 的编译失败，不算产品 RED。修正测试工具后 shared-safe-red2 为 0 passed、5 failed、0 ignored。shared-safe-final2 为 8 passed、0 failed、0 ignored，含 prepare failure、无 actual proof apply failure、unknown、双成功、反序、apply 二次引用核验、重新 enable 与 preset manual-close。

focused 首轮 exit101，缺失 source 恢复回归暴露 discovery inactive 保留范围过宽，收窄同路径 shared 后既有断言未修改，focused-final2 exit0。all-targets-final 为 98/0/0，含3项直接返回的顶层 recovery helpers，不当作98个独立场景。focused-final2 为56/0/0，也含helpers。recovery 日志有真实SIGKILL，concurrent日志有双PID存活和两种锁顺序。两个全量均执行无skip/ignore。verify5-final 100/100，脚本只改 BIN/RUNS 两行，diff 在 scripts/verify.diff。

最终 binary SHA256 为 f1d4dd4adc41f7b26b0a5a0acde4b791685ee871c85bfd9f8516c8ecbf996392，E/bin 与 target/debug cmp exit0。原 shared-repro 字节相等，修后旧 assert !link.exists() 预期失败 exit101。独立 shared-safety.rs 改成入口保留 assert，最终新fixture exit0，snapshot/report位于 E/final-safe/evidence/shared-safe-probe。原RED在 E/evidence/shared-planned-repro 未覆盖。

## 有界自然 CLI 检查

adapters.rs:1933-1939 给出三adapter roots。primary_skill_root 取 first，普通 CLI 三target enable 真实创建 .claude/skills、.codex/skills、.pi/skills 中三个不同 inode 的 pc-link，E/evidence/native-overlap-Z0mkRB/paths.json 与 events.json 可复跑。Codex/Pi 扫描额外 .agents/skills，外部创建 shared link 后同一CLI discovery可见且保存为 unmanaged-symlink，shared-observation.json 明确双target；Claude 没有此 root。CLI 没有把这外部 overlap 自动认领 owned。全局/project同root、嵌套projects、root别名等未穷尽，不声称 owned shared 自然 CLI 不可达，也不声称已经可达。

## 偏离与残余

试过 engine commit 后校准 peer metadata，随后撤销，避免新增第二次 durable write 窗口。最终 engine/storage/domain/cli/tui/main/lib/Cargo 和其他tests与新备份逐字节相等，protected-files.exit=0。没有Git/jj、暂存区检查、提交、发布、全仓fmt/clippyfix、auth读取或真实配置写。编辑hook只格式化编辑文件，逐次重读。LSP两文件4项cfg hint、1项inconclusive，不当作clean证据。

共用物理link时先关闭target的intent为disabled，但其入口为peer保留，未添加native config掩蔽，不保证真实agent不再加载。末target移除后peer物理metadata下一discovery校准，不进行跨targetatomic存储。此限制属保守引用协议，不外推实际agent加载状态。最终hash到unlink/rename窗口、跨state-dir、不合作writer、删除crash、多route partial mutation、平台与掉电仍未证明。runtime-evidence.json 为本run Pi声明 cliproxy/gpt-6.1-sol/high/openai-completions，上游真实模型和effort未知。E在/tmp可能清理。

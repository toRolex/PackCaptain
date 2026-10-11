# F3 discovery IO

## 状态与 checkpoint

- RED，done。E=/private/tmp/packcaptain-f3-qDaAGx。src/tests/Cargo 备份在 E/before。discovery-red exit101，1 passed、5 failed。两个独立固定旧 binary CLI 安全断言 exit1。
- 修复，done。只改 adapters discovery seam，新增 discovery_errors.rs。沿用 F1 target_errors 与健康 Plan 投影，engine 字节不变。
- 验收，done，仅本 seam。focused-final2 exit0，73/0/0。all-targets-final exit0，108/0/0。固定 discovery suite 10/0/0，verify5 100/100。非最终全 IO 安全完成。

## 清单

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 真实 FS 与 CLI 已复现。
2. Binary-search the cause. scan IO 错误被空指纹、false 与诊断吞掉，旧成员 metadata 再猜 Missing。
3. Plan the fix. 同形临时 report，成功才合并，失败写同一 target_errors。
4. Verify on the same surface. 固定 binary CLI read EACCES 与原生 WalkDir EACCES RED 到 GREEN。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止提交。
6. Run Opening a PR. skip，禁止发布。

## how 与设计

数据形状沿用 DiscoveryReport.target_errors。Unknown 不是 disabled 或 Missing。发现目标的临时同形 report 只在 scan 与旧成员保留检查都成功后合并，避免失败目标的半份 entries/skills 覆盖已可信状态。Model the Domain 决定复用 F1 形状。Boundary Discipline 决定在真实 FS 边界分类 NotFound，其余错误向目标传播。Prove It Works 决定先真实 CLI RED，再固定二进制验证。

候选一在每处错误直接修改主 report，容易留下部分 skills 与旧成员错误推断。候选二在 per-target 边界返回 Result 并原子合并内存报告，选候选二。不会引入 durable 平行状态，不改混合目标执行协议。how/architect 串行比较，用户禁止子派发与动态 workflow。

吞吐检查点为备份/RED，最小 discovery 修复/代表性 GREEN，focused/全量/fixed binary。禁止 Git/jj、全仓 fmt、clippyfix、auth、真实配置写。权限恢复仅限本次 TempDir fixture。

## 判定与回归

NotFound 在缺失 root、SKILL.md、消失 entry、canonical source 处保持正常缺失。fingerprint 的 NotFound 只有再次确认 entry 或源定义确实消失才允许缺失，否则 Unknown。EACCES、ENOTDIR、loop 与其他错误均 Unknown。WalkDir 任意 Err 保守 Unknown。保留 WalkDir OS ErrorKind，loop 无底层 IO 时为 Other。

Dangling owned link 源缺失仍为 Missing。源恢复后同源 foreign inode 不认领，prepare 安全拒绝。健康目标仍独立失败或提交，不修改 F1 engine。两个 Unknown 的确认报告不含其 effects/target_updates，健康目标 applied。持久 Applying journal payload 精确相等，未知 CLI 和 confirmed 均不消费。

## 失败保留与因果

首次 discovery-red 中 walkdir 子目录 probe 先被无序 read_dir 选错入口触发测试工具错误。限定 pc- link 后回归通过，此项不当作 WalkDir RED。独立 cli-walkdir-red 固定旧 binary 的真实 WalkDir EACCES 安全断言 exit1，修后同脚本 exit0。

discovery-green2 exit101 的 dangling 回归把健康全局源重建导致的 inode metadata 更新误算成未知目标写，限定真实 target intent/ownership 断言，未放宽 foreign-link 身份或拒绝断言。focused 首轮 exit101 揭示按 target preserve 时旧 Missing skill 覆盖健康新 Available 观察。把旧库存 fallback 移到全部成功 observations 之后，原 missing_retained_manual_close_survives_real_reopen 不改断言，后 focused GREEN。Missing old member 源恢复清理、parse config、A replacement、shared、真实 recovery 与 concurrent 均通过。

## 证据与边界

日志位于 E/logs，每项含 command/log/exit/时间/binary。真实 read EACCES 与 root WalkDir CLI 脚本位于 E/scripts，分别 cli-permission-red/green 与 cli-walkdir-red/green。fixture 保留在 E/evidence。fixed-discovery-final 的 CLI command/stdout 在 E/evidence/fixed-discovery-final。新增 10 tests，0 ignored。权限 guard 在异常 unwind 时恢复本次创建路径权限。

fixed SHA256 为 132604e78bedae691485f64cb4da926421a558d73297a98c53aea8da05c8ea86，最终与 E/target/debug/packcaptain cmp exit0。旧 binary SHA 为 f1d4dd4adc41f7b26b0a5a0acde4b791685ee871c85bfd9f8516c8ecbf996392。verify.diff 仅 BIN/RUNS 两行，100项断言未改。全量108条含3个顶层直接返回的 recovery helpers，不当作108个独立场景。protected-files.exit0，engine 和禁止文件、其他 tests 字节不变。无 Git/jj 或暂存区核验，不声称 index 为空。没有派发、动态 workflow、全仓 fmt/clippyfix、auth 或真实配置写。

LSP 两文件4项cfg hint、1项inconclusive，不作为clean证明。runtime-evidence.json 为本child首assistant声明，2026-10-11T19:18:45.232Z，cliproxy/gpt-6.1-sol/high/openai-completions。上游真实model/effort未知。

未穷尽 read_link 独立竞态失败、确定性 canonicalize 在 metadata 成功后才失败的微窗口、Linux/Windows、跨state-dir、掉电、非合作writer、多route partial mutation与删除crash。本轮 canonicalize-loop fixture在SKILL metadata边界先识别ELOOP，另有canonicalize实调用失败证据，不声称精准强制该分支。fingerprint loop 和source read真实可达。仅解决上述 discovery seam，不宣称完整 IO 或最终hash到rename/unlink窗口解决。E位于/tmp可能清理。

# E2 独立再验收

结论为**通过本轮有界验收**。同一测试后二进制的全量回归、原业务黑盒、安全断言和固定 PTY 均通过。F1、F2、F3 组合审查未发现具体阻塞。E1 的配置假成功和构造共享入口提前删除不再复现。本结论不是发布授权，也不是全仓安全保证。

## 冻结与身份

证据根 E 为 `/private/tmp/packcaptain-e2-yXzBP0`。HOME、XDG、agent-home、state、构建 target 与业务 fixtures 均为新临时目录。Cargo registry、git 缓存复制到新 CARGO_HOME 后离线运行，不复制 auth 或真实配置。原 PTY 测试另按自身代码创建新 `/private/tmp/packcaptain-frontend-pty-<PID>`，没有改路径或断言。

- build 后 CLI SHA256 为 `f0a405a7c7be60804bef950d9fbf44700431a049d2bcd0847d17e2f497416a34`。
- all-targets 确实重新链接 CLI。测试后复制至 `E/bin/packcaptain`，固定 SHA256 为 `88cb8dbbedacd332e27ccc8b088643f43e175434584c9fea9cc52eb2aa2fdb44`。
- 固定文件 size 为 15924576，inode 为 161095779，mode 为 0555，mtime 为 `2026-10-12T03:37:11+0800`。末次与 Cargo CLI cmp exit0，SHA 与身份未变。
- `source-before.sha256` 和 `source-final.sha256` 覆盖 src、tests、Cargo、docs、AGENTS、CLAUDE 与 GLOSSARY，共37文件。清单 SHA256 均为 `7fdcd23b0e63a710a9c04eff2fc50859700bc8f5050d725042ce89f72b44d1c0`，cmp exit0。
- 原验收脚本 hash 校验全部通过。`scripts/verify.diff` 仅修改 BIN、RUNS 两行。config 安全断言、scan、WalkDir、shared 与两个 PTY 编译脚本原样复制，路径通过 E 或参数传入。

## 运行结果

每项入口有 `logs/<label>.command`、`.log`、`.exit`、起止时间和前后二进制身份。`summary.json` 记录21个入口，全部真实 exit0，没有产品失败后重试或工具编译失败。

| 入口 | 结果 | E 下证据 |
| --- | --- | --- |
| `cargo build --offline` | exit0 | `logs/build.*` |
| `cargo test --offline --all-targets -- --nocapture` | 108 passed，0 failed，0 ignored | `logs/all-targets.*` |
| 原 `acceptance.mjs E/bin/packcaptain` | 完整111命令，106 exit0，5项预期错误非零 | `evidence/packcaptain-blackbox-run-p7xopK/logs/` |
| 临时 `verify.js 5` | 100/100断言，五轮双PID overlap均true | `evidence/verify-dbrf0U/report.json` 与各轮 events |
| 原 config 安全断言 | exit0，invalid config后的disable真实exit1，list显示unknown，link inode和source不变 | `logs/config-generator.*`、`logs/config-safety.*`、`evidence/config-finding-v2.json` |
| 真实 read EACCES 与 WalkDir EACCES | 两项exit0，unknown、拒绝preview及执行、DB字节保持，恢复权限后合法disable成功 | `logs/scan-permission.*`、`logs/scan-walkdir.*`、`evidence/scan-3UH3xJ/`、`evidence/scan-IEOKXz/` |
| 最新lib重编译 shared probe | exit0，首target逻辑关闭，第二target失败后真实shared link保留 | `logs/compile-shared.*`、`logs/shared-safety.*`、`evidence/shared-safe-probe/` |
| fixed `tui_extended_fixed --nocapture` | 12 passed，0 failed，0 ignored | `logs/fixed-pty.*`、`fixed-pty/` |
| fixed `tui_original_fixed --nocapture` | 1 passed，0 failed，0 ignored | `logs/fixed-original-pty.*`、`original-pty-transcripts/` |
| fixed长列表额外固定3轮 | 每轮1 passed，0 ignored | `logs/fixed-long-1` 到 `-3.*`、`fixed-long-1/` 到 `-3/` |
| 新fixed CLI定义更新探针 | unknown已启用sync target拒绝更新，SQLite主文件字节、source、link身份相等 | `scripts/definition-zero-write.cjs`、`logs/definition-zero-write.*` |
| 冻结文件、原脚本与fixed binary核对 | 三项exit0 | `logs/source-compare.*`、`logs/original-scripts-compare.*`、`logs/fixed-compare.*` |

全量108条包含 `crash_child`、`forward_safety_child`、`rollback_safety_child` 三个顶层直接返回的helper，不当作独立恢复场景。全量加fixed PTY及三轮长列表共124个通过测试条目，不与CLI命令数或断言数相加称作产品场景。

`logs/all-targets.log` 保存真实child PID、`signal: 9 (SIGKILL)`、journal持久gate和native mutation完成gate。`recovery/` 保存 before-forward、forward、rollback实杀证据。并发日志保存两种锁顺序各五轮双PID存活，`alive=None,None` 与真实exit0。verify另有五轮双enable PID和overlap，不能用helper成功代替这些证据。

固定PTY直接使用当前test源码rustc编译，没有改变断言。`harness-inputs.sha256` 记录最新lib与依赖，两个 `harness-*-reference.log` 证明嵌入E/bin路径。`cargo-test-binary-reference.log` 证明Cargo suites嵌入隔离target CLI，该CLI测试后SHA与fixed相等。fixed断言文件覆盖D2严格重确认、连续变化、Esc取消、迟到manual-close、foreign入口拒绝、Applying journal不消费和配置错误后保留UI pending Command。长列表仍执行90项、95次down/up、回顶及搜索Skill089。

config生成器沿用F1保存的green probe，原样复制。本轮没有改断言。它与E1缺陷存在断言生成器的差异保存在 `scripts/config-generator.diff`，删除的是旧缺陷必须存在的断言，不把该生成器算安全验收。真正安全判定仍由逐字节不变的E1 `config-acceptance.cjs` 独立执行。shared探针构造ownership并注入第二target prepare failure，FS、NativeAdapters和SQLite真实，重新关联当前lib。它不是纯fixed CLI证据。

## 有界组合审查

先完成运行检查点，再串行审阅F1、F2、F3精确diff及直接消费者。diff保存在 `f1-*.diff`、`f2-adapters.diff`、`f3-adapters.diff`。当前engine、cli、tui与F2前备份相等，F2、F3没有额外改变这些消费者。

- `src/adapters.rs:181-204` 仅在完整scan与旧entry保留检查成功后合并目标局部报告。失败目标的部分skills、entries和observed_enabled不进入主报告。`:128-146` 对unknown目标跳过entries及enabled合并。`:166-172` stored skill fallback只补缺失项，不覆写成功观察。真实EACCES、WalkDir、Missing恢复、loop与foreign替换回归均通过。
- `src/engine.rs:240-268` 只删除unknown目标的effects与target_updates，同时比较fresh和confirmed健康Plan。`src/domain.rs:724-854` 只迭代command明确targets，定义同步亦只迭代sync_targets。`src/tui.rs:625-682` 保留原Command，不扩范围，Plan变化返回重新确认。健康投影不是自动扩目标的授权。
- `src/engine.rs:430-458,497-525` 对含受影响unknown target更新的preset定义计划拒绝，不写定义或该target。全unknown普通开关返回failed并投影空mutation，CLI `src/cli.rs:419-434` 将failed映射非零，不是假成功。新增fixed定义更新探针独立确认拒绝和零业务变化。
- `src/adapters.rs:829-856` shared引用只读committed Snapshot，不读planned states。manual_closed排除preset引用，仍保留真实enabled和manual_open需求。`:525-537` apply再次计算引用。`:1668-1682` 用present而非active决定物理创建或删除。首target关闭但peer仍需要时保留物理入口，最后引用释放后可再启用。构造shared probe及八项shared事务回归通过。
- `src/engine.rs:424-429,460-482,658-663` confirmed对持久pending保守拒绝，CLI恢复对unknown journal targets预检。`src/adapters.rs:1440-1467,1517-1628` receipt校验operation/direction key、ordinal、target、source、anchor及no-follow身份。无receipt旧pending不能认领公开入口。当前全量中的forward/rollback旧pending、同源新inode、foreign目录、anchor替换和方向互借拒绝回归均通过。

没有发现需要最小repro的新具体阻塞，故没有边验边修或扩大安全挖掘。no-comments在只读边界串行检查精确diff，无新增注释或suppression需处理，删除数0。

## 未测试与保证边界

以下是限制或欠验，不是本轮复现的真实缺陷。

- state lock只覆盖同state-dir合作进程。cross-state-dir操作同target、非协作writer的最终hash到rename/unlink微窗口、同UID篡改私有receipt目录不在保证内。
- 未实现Delete pin。删除crash、多route partial mutation、Prepared发布前更细实杀、commit后立即实杀、orphan staging和cleanup失败未穷尽。保留NeedsRecovery或目录不等于完整自动恢复。
- shared物理entry为其他target保留时，先关闭target的logical intent已disabled，但不保证真实agent停止加载。ownership自然CLI可达性没有在E2继续扩证。
- TUI持久pending需要先通过现有CLI合法业务操作恢复，再重新预览，不自动消费也没有新增recover命令。
- 只实测本机macOS。Linux、Windows、其他FS、跨平台终端、掉电durability、其他尺寸与combining characters未测。长列表观察模型只是当前Crossterm ANSI子集。
- 本轮没有真实agent app-server或新会话加载验证，不声称技能已被真实agent加载或卸载。read_link与canonicalize独立微窗口、非协作权限竞态未穷尽。
- 没有读取auth、写真实agent配置、删除既有skills、派发、动态workflow、Git/jj、暂存、提交或发布。暂存区未检查，不声称为空。临时证据可能被系统清理。

Prove It Works 决定以测试后fixed binary执行原验收并做末次cmp。Sequence Work into Verifiable Units 决定先构建身份、全量、fixed行为检查，再做有界源码审查。检查点全部完成，不留fixed PTY欠验。how与no-comments因用户禁派发而串行执行，unslop与technical-writing用于报告。

`runtime-evidence.json` 记录本child首assistant于 `2026-10-11T19:34:23.322Z` 的Pi声明，provider为cliproxy，model为gpt-6.1-sol，api为openai-completions。上游真实模型与effort未核实。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "提供同一测试后SHA的108/0/0全量、原111命令、verify100/100、config与真实EACCES/WalkDir、最新lib shared probe、fixed两PTY suites和长列表3轮证据，并区分保证边界与未测。"
    }
  ],
  "changedFiles": [
    "/Users/rolex/.pi/agent/sessions/--Users-rolex-Documents-Codes-githubProject-MyProject-PackCaptain--/subagent-artifacts/outputs/46ddea3b-5435-48df-9573-6f715a298546/e2-final-validation.md",
    "/private/tmp/packcaptain-e2-yXzBP0/"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {"command": "cargo build --offline", "result": "passed", "summary": "exit0，新CARGO_HOME及target。"},
    {"command": "cargo test --offline --all-targets -- --nocapture", "result": "passed", "summary": "108 passed，0 failed，0 ignored，含3个直接返回helper。"},
    {"command": "node 原acceptance.mjs E/bin/packcaptain", "result": "passed", "summary": "111命令，106成功和5项预期错误。"},
    {"command": "node E/scripts/verify.js 5", "result": "passed", "summary": "100/100断言，五轮双PID overlap。"},
    {"command": "node 原config安全断言及green数据生成器", "result": "passed", "summary": "两项exit0，fixed CLI invalid config后拒绝disable。"},
    {"command": "node 原cli-scan.cjs与cli-walkdir.cjs fixedbinary 新fixture根", "result": "passed", "summary": "两项真实EACCES安全断言通过。"},
    {"command": "rustc关联最新lib编译并执行shared-safety.rs", "result": "passed", "summary": "两项exit0，构造ownership与failure，真实FS/SQLite，非纯fixed CLI。"},
    {"command": "rustc原当前PTY测试编译两个fixed harness", "result": "passed", "summary": "两项exit0，test源码与断言未改，CLI嵌入E/bin。"},
    {"command": "fixed extended与original PTY全suite", "result": "passed", "summary": "12+1 passed，0 failed，0 ignored。"},
    {"command": "fixed长列表exact固定3轮", "result": "passed", "summary": "每轮1 passed，0 ignored，无失败后重试。"},
    {"command": "fixed CLI definition-zero-write", "result": "passed", "summary": "unknown已启用sync target拒绝定义更新并保持DB、source、link身份。"},
    {"command": "source/docs/Cargo cmp、原脚本hash校验、fixed binary cmp", "result": "passed", "summary": "三项exit0，冻结37文件和原脚本不变。"}
  ],
  "validationOutput": [
    "21个记录执行入口全部exit0，无本轮产品或编译失败。",
    "全量108加fixed PTY与三轮长列表共124个通过测试条目，含3个helper，不当作124个独立产品场景。",
    "固定SHA256 88cb8dbbedacd332e27ccc8b088643f43e175434584c9fea9cc52eb2aa2fdb44。",
    "冻结清单前后SHA256 7fdcd23b0e63a710a9c04eff2fc50859700bc8f5050d725042ce89f72b44d1c0。"
  ],
  "residualRisks": [
    "cross-state-dir、非协作writer最终微窗口、同UID篡改不在保证内。",
    "删除crash、多route partial mutation、更细实杀、跨平台与掉电欠验。",
    "shared物理entry保留不保证先关闭target的真实agent停止加载。",
    "TUI pending必须先通过现有CLI合法恢复后重新预览。",
    "未实测真实agent加载，暂存区未检查，临时证据可能清理。"
  ],
  "noStagedFiles": false,
  "diffSummary": "src/tests/Cargo/docs及入口文档37文件前后逐字节不变，只写新tmp证据和权威报告。",
  "reviewFindings": [
    "no blockers。F1/F2/F3精确diff及engine/tui直接消费者有界审查未发现具体阻塞。"
  ],
  "manualNotes": "结果是有界独立再验收通过，不是发布授权。shared probe为真实FS/SQLite加构造ownership与prepare failure。noStagedFiles=false表示没有核验暂存区，不表示发现暂存文件。"
}
```

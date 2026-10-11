# E1 独立最终运行验收

结论为**不通过最终安全验收**。全量回归、原业务黑盒和固定二进制 PTY 均通过，但独立 IO 探针复现配置错误被猜为 disabled 的假成功。另复现 planned state 提前释放共享入口的真实文件系统风险，输入 ownership 和第二目标失败由测试构造。没有修复生产或测试。此报告不是发布授权。

## 身份与冻结

证据根 E 为 `/private/tmp/packcaptain-e1-rRUpbA`。先构建、再全量测试、再复制固定二进制并驱动黑盒与 PTY，最后核对字节身份。全部 HOME、XDG、CARGO_HOME、构建 target、agent-home、state 和 project 写入均隔离到新临时目录。既有 Cargo registry/git 只复制缓存，没有复制或读取真实 auth。

- `cargo build --offline` 后 SHA 为 `b03a05e4272e08a5a7233c992c2dcfdbb4e94b480495bec5c3674dc996df2d37`。
- `cargo test --offline --all-targets -- --nocapture` 确实重新链接 CLI，测试后 SHA 为 `a55747bccdc3dccbb55fe12541db86c9aa20ae01b0657b37b791d306ab38a693`。
- 后续外部验收只用 `E/bin/packcaptain`，SHA 为上述 `a55747…`。复制后与最终 mtime 均为 `2026-10-12T02:26:56+0800`，inode 为 160712895，size 为 15625744。`logs/fixed-final-identity.*` 和 `logs/binary-compare.*` 证明最后与测试后 CLI 逐字节相同。
- `source-before.sha256` 与 `source-final.sha256` 覆盖全部 25 个 src、tests、Cargo 文件，两份清单 SHA 均为 `eaa4ec1119c9616077e28b5446530469bcba6f3a7ea947e120474712777d7ccb`。`logs/source-compare-final.exit=0`。docs 六文件清单也完全一致。
- 原 safety 与 acceptance 脚本 SHA 位于 `scripts/original.sha256`。最终原脚本比较 exit0。safety 临时副本只修改 BIN 和输出 RUNS 两行，最小 diff 位于 `scripts/verify.diff`，没有改变断言。原 acceptance.mjs 未编辑。
- Cargo suites 使用编译时 `env!("CARGO_BIN_EXE_packcaptain")`。`cargo-test-binary-references-v2.log` 保留四个真实 test executable 中的隔离 target CLI 路径。固定 PTY harness 用原测试源码直接 rustc --test 编译，编译时将该变量定向 E/bin，未复制或修改测试断言。`scripts/compile-pty-v3.sh`、`scripts/compile-original-pty.sh`、`harness-inputs.sha256` 与两份 harness-binary-reference 日志可对照。

## 运行结果与证据索引

每个执行入口均有独立 `logs/<label>.command`、`.log`、`.exit`、`.binary` 和开始/结束时间。详细命令退出码索引为 `summary.json`。不把断言、CLI 命令数和测试数相加称作产品场景。

| 执行入口 | exit | 结果 | E 下证据 |
| --- | --- | --- | --- |
| cargo build --offline | 0 | 真实构建 | logs/build.* |
| cargo test --offline --all-targets -- --nocapture | 0 | 83 passed、0 failed、0 ignored | logs/all-targets.* |
| 临时定向 verify.js 5 | 0 | 100/100 断言，五轮双 PID overlap 均 true | evidence/verify-ztQKvF/report.json，各轮 events.json |
| 原 acceptance.mjs，参数为 fixedbinary | 0 | 原 111 命令完整复跑，106 exit0，5 项预期错误非零 | evidence/packcaptain-blackbox-run-FdOkf7/logs/summary.json、commands.json |
| fixed tui_extended_pty 全 suite | 0 | 11 passed、0 failed、0 ignored | logs/fixed-pty-suite-v3.*、fixed-pty/ |
| fixed tui_pty 全 suite | 0 | 1 passed、0 failed、0 ignored | logs/fixed-original-pty.*、fixed-original-pty-transcript/ |
| fixed 长列表额外固定三轮 | 全0 | 每轮 1 passed、0 failed、0 ignored，90 项及回顶、搜索断言保持 | logs/fixed-long-v3-1 到 -3.*、fixed-long-1 到 -3/ |
| config-repro-v2 | 0 | 成功复现缺陷，不算产品 PASS | scripts/config-repro-v2.cjs、evidence/config-finding-v2.json |
| config-safety-acceptance | 1 | 新安全断言明确失败，原始日志保留 | scripts/config-acceptance.cjs、logs/config-safety-acceptance.* |
| shared-repro | 0 | 成功复现带构造输入的共享入口风险，不算产品 PASS | scripts/shared-repro.rs、evidence/shared-planned-repro/ |

全量 83 个测试中 `crash_child`、`forward_safety_child`、`rollback_safety_child` 三项顶层运行无子环境时直接返回，不能算独立恢复场景。其余为 80 项测试条目，也不等于 80 个不同产品场景。含固定 PTY 重复运行，本轮测试条目执行总计 98 passed、0 failed、0 ignored，其中三项为上述 helper。另有一项独立安全断言失败。

执行入口总计 29 次，21 次 exit0，8 次非零。七次非零是工具配置或探针失败，不能隐藏，也不是产品 RED。首次 rustc extern 多匹配 exit1，随后缺 rusqlite exit1，四次在 harness 未生成时启动 exit127，首个 IO 探针误猜主目录 `.agents/skills` exit1。修正的只是临时工具参数，原失败日志全部保留，没有改产品或断言、没有盲重试挑绿。

业务黑盒覆盖创建、精确搜索、共享成员并集、多 projects 与明确 targets、整组重启覆盖单项关闭、keep/restore、定义幂等、冲突和 preview 零语义写、错误 targets、非托管同名碰撞保护、Codex 两目标隔离关闭及 source 字节保护。它不比较 SQLite WAL 的物理字节，也不证明真实 agent 已加载技能。

真实并发测试含五轮双 enable 和两种实际锁顺序各五轮 disable/enable。日志保存双 PID、alive=None,None 及实际 effects=2/1，source 内另有 enable assert_ne PID 和双存活 barrier。verify report 另保存五轮 enable 的双 PID 与时间 overlap。不是 helper 的成功返回。

`recovery_real` 日志含真实 forward 和 rollback child PID、`signal: 9 (SIGKILL)`、`gate=real native mutation completed`、持久 journal 及 fresh CLI/retry。crash 时原 SQLite、links、gate 与 child stdout/stderr 的副本在 `recovery/`。未忽略恢复测试。

固定 PTY 的 `fixed-pty/*.assertions.txt` 证明严格二次重确认、连续变化再确认、Escape 完整取消、迟到 manual-close 必须显式 Keep、foreign 目录拒绝、真实 Applying journal payload 字节和 snapshot、symlink no-follow identity 全部未被确认消费。两个 suites 全量均执行。

## 六项 IO 欠项核对

| 欠项 | 当前实现、已测与未测 |
| --- | --- |
| mixed native/owned routes | `adapters.rs:710-723` 明确拒绝，prepare/apply 分别在 456、531-532 调用。全量 `adapter_replacement_safety` 的真实 TempDir 回归通过。没有承诺混合路线可成功关闭。 |
| planned state 提前释放共享入口 | **风险复现**。`adapters.rs:814-816` 优先使用未提交 planned state。`engine.rs:441-450,456-462` 逐目标 prepare，后一目标失败不会撤销前一目标 commit。shared-repro 构造 Codex/Pi 同路径 owned symlink 与两个 manual_open，第一目标真实 NativeAdapters 删除入口，第二目标 prepare 注入失败。真实 SQLite 保留第二目标 enabled/manual_open，但入口已删。非纯 CLI，ownership 为持久输入构造，不声称已证自然 CLI 可达。 |
| pending 创建认领同源替换 | `adapters.rs:734-741,1693-1698` 拒绝无 proof 已出现入口。create receipt 的 operation、direction、ordinal、source、anchor、no-follow inode 校验，以及 forward/rollback 同源替换、anchor 替换、无 receipt 拒绝回归通过。Prepared 分支主要由真实 SQLite stage 构造，不是所有发布前 crash 窗口的实杀证据。 |
| scan/config 错误猜 disabled | **真实 CLI 阻塞**。`adapters.rs:351-373` 将解析错误映射成 active=false，`127-140` merge 将 enabled_skills 清空。`engine.rs:422-433` 无 effect 时仍提交 disabled intent。新 HOME 中先 enable 项目 preset，再将 `.claude/settings.local.json` 写成 `{broken-json`，targets list 报 enabled_skills=[]，disable 返回 exit0、ok:true、effects=[]、preset=false，原 owned link 的 inode 与 source 内容不变。首份最小证据为 evidence/io-command-6.json 至 -8.json，完整第二次有 config-finding-v2.json 与明确失败的安全断言。权限失败、walker 错误、Codex/Pi 配置错误未穷尽。 |
| real partial failure inverse 使用计划态 | `engine.rs:495-504,548-558` 无 actual_entries 则保留 NeedsRecovery，不执行猜测 inverse。`606-678` 持久实际 metadata 并传 rollback context。真实 wrapper 在 NativeAdapters 之后报错的 rollback、SIGKILL、fresh reopen 回归通过。Native 多 routes 部分突变无完整 proof 时仍可能 NeedsRecovery，没有证明能自动 inverse 全部 native 部分失败。 |
| final hash 到 rename 窗口 | `adapters.rs:1157-1165` 最终检查后仍为非 CAS rename，owned unlink 在 `1753-1757` 也有窗口。已有回归只证明 prepare 后已发生的 config 更改被拒绝。最终微窗口未实测消除。协议限制是外部 writer 暂停或参与同一排他锁，不承诺非协作 writer 零丢失。 |

以上是有界范围核对，不是全仓安全审计完成。后续独立 review 的有界清单为共享入口自然 CLI 可达性及后目标 native 失败、权限/scan/config 错误的 Unknown 传播、多 route partial mutation 与 inverse、删除阶段实杀和最终 rename/unlink 并发窗口。此 unit 不继续修复或扩审。

## 未测与协议限制

- 锁只覆盖同 state-dir 合作进程。跨 state-dir 操作同一 target 未测且不受此锁保护。
- 非协作 writer 与同 UID 对私有 receipt staging 的恶意篡改不在保护保证内。
- Delete pin 未实现。删除中途 crash、多 route partial mutation、Prepared 发布前更细 SIGKILL、commit 后立即 SIGKILL、orphan staging 和 cleanup 失败未穷尽。保守保留目录及 NeedsRecovery 不等于完整自动恢复。
- 只运行本机 macOS/Unix。Windows、Linux、其他 FS、跨平台 terminal、掉电 durability、其他尺寸和 combining characters 未测。
- TUI pending journal 必须先由现有 CLI 合法操作完成恢复后重新预览，确认不会自动消费，未新增 recover 命令。静态 discovery API 一般 freshness 不在当前 CLI/TUI native 确认修复的保证内。
- 长列表模型为当前 Crossterm ANSI 子集，不是完整终端模拟。其他 PTY 仍有历史流文本观察和固定 sleep。
- 本轮不读取真实 auth、不写真实 agent 配置、不删除既有 skills、不派发、不动态 workflow、不执行 Git/jj、不提交或发布。暂存区未检查，因此不声称为空。
- E 位于临时目录，可能被系统清理。产品版本原生加载验证和分发授权不是本轮验收结论。

Prove It Works 决定固定真实二进制并保留失败。Build the Lever 决定保存可复跑身份、驱动和探针脚本。Model the Domain 决定按 Snapshot、Plan、receipt 和已提交 target 状态区分证据，不把 planned state 当实际状态。检查点依次为构建身份、全量回归、固定 artifact 黑盒与 PTY、IO 有界复现和最终 hash。吞吐取舍为日志写文件、串行依赖执行，未扩大研究或修复。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "提供固定二进制身份、83/0/0全量结果、100/100断言、原111命令、固定PTY两suite及三轮长列表证据，并报告一个真实CLI安全阻塞和一个带构造输入的共享入口风险。"
    }
  ],
  "changedFiles": [
    "/Users/rolex/.pi/agent/sessions/--Users-rolex-Documents-Codes-githubProject-MyProject-PackCaptain--/subagent-artifacts/outputs/1e4417a2-0d1e-47e0-9f61-a29cd5a38ddf/e1-final-acceptance.md",
    "/private/tmp/packcaptain-e1-rRUpbA/"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {"command": "cargo build --offline", "result": "passed", "summary": "exit0，隔离新CARGO_HOME和target。"},
    {"command": "cargo test --offline --all-targets -- --nocapture", "result": "passed", "summary": "83 passed，0 failed，0 ignored，含3个顶层helper。"},
    {"command": "node E/scripts/verify.js 5", "result": "passed", "summary": "100/100，只定向binary和证据目录。"},
    {"command": "node /private/tmp/packcaptain-blackbox-20261011/scripts/acceptance.mjs E/bin/packcaptain", "result": "passed", "summary": "原111命令完整复跑。"},
    {"command": "E/bin/tui_extended_fixed --nocapture; E/bin/tui_original_fixed --nocapture", "result": "passed", "summary": "11+1 passed，0 failed，0 ignored，同fixedbinary。"},
    {"command": "fixedbinary long-list exact，固定3轮", "result": "passed", "summary": "3次各1 passed，没有产品失败后重试。"},
    {"command": "node E/scripts/config-acceptance.cjs", "result": "failed", "summary": "真实CLI invalid config后disable假成功，安全断言exit1。"},
    {"command": "E/bin/shared-repro", "result": "passed", "summary": "复现真实FS提前删除风险，不是安全验收通过。ownership构造且后目标prepare注入失败。"},
    {"command": "前后source/docs/原脚本cmp与fixedbinary最终cmp", "result": "passed", "summary": "所有冻结文件字节一致，fixed SHA和mtime不变。"},
    {"command": "初次临时PTY编译与启动、首个IO路径探针", "result": "failed", "summary": "7次工具失败完整保留于logs，未计作产品RED。"}
  ],
  "validationOutput": [
    "全部执行入口29次，21次exit0，8次非零。其中1次安全断言失败，7次临时工具失败。",
    "全量加固定PTY重复共98 passed测试条目、0 failed、0 ignored，包含3个helper，不当作98个独立产品场景。",
    "固定binary SHA256 a55747bccdc3dccbb55fe12541db86c9aa20ae01b0657b37b791d306ab38a693。",
    "source清单前后SHA256 eaa4ec1119c9616077e28b5446530469bcba6f3a7ea947e120474712777d7ccb。"
  ],
  "residualRisks": [
    "invalid config被猜disabled并提交假成功，最终安全验收不通过。",
    "planned state可能提前释放其他尚未提交target所需共享入口。当前证据为真实FS和SQLite加构造ownership及失败注入。",
    "跨state-dir、非协作writer、删除crash阶段、多route partial mutation、跨平台和掉电未证明。",
    "TUI pending不自动恢复，需现有CLI合法操作完成恢复后重新预览。",
    "临时证据可能被系统清理，未核验暂存区。"
  ],
  "noStagedFiles": false,
  "diffSummary": "src/tests/Cargo/docs冻结字节均未变，只写权威报告路径和新/tmp证据。",
  "reviewFindings": [
    "blocker: src/adapters.rs:351-373 与 src/engine.rs:422-433 - invalid native config使owned route被标inactive，disable无effects却返回成功和提交disabled intent。",
    "risk: src/adapters.rs:814-816 与 src/engine.rs:441-462 - 未提交planned state释放共享入口，后目标prepare失败时仍需该入口。"
  ],
  "manualNotes": "已读规则入口、交接欠项、C3/D2/D3/C2b notes、GLOSSARY、ADR0001及已定规格。报告写入runtime权威路径，未另写用户较早指定notes路径。此结果仅为验收证据，非发布授权。noStagedFiles=false表示未检查，非发现暂存文件。"
}
```

# PackCaptain 单元 D 验证记录

2026-10-12。证据根目录为 `/tmp/packcaptain-uD-20261012T001107`，下文称 E。本单元未提交、推送或发布，也未修改真实 agent 配置。

## 结论与修复

`--version` 和 `--help` 的真实退出码均由 2 改为 0，stdout 逐字节不变。根因为 `Cli::try_parse_from` 的帮助和版本展示结果被当作参数错误，统一返回 2。`src/cli.rs` 现在先处理 Clap 的 stdout 展示结果，保留 `error.print()` 和 `error.exit_code()`。真正参数错误仍保持 JSON 错误及非零退出码。

新增两项 CLI 回归，覆盖版本、根帮助、子命令帮助、stderr 为空及未初始化 storage。`version-red.log` 和 `help-red.log` 均记录 `left: Some(2), right: Some(0)`。`cli-green.log` 记录 4 项全部通过。真实 CLI 前后 stdout、stderr、exit 与 stdout 比较结果分别在 `version-before.*`、`version-after.*`、`help-before.*`、`help-after.*`、`*-stdout-compare.exit`。

数据形状保持现有 Clap 错误分类和现有 TUI 状态机，不增加生产类型。PTY 测试以隔离 Fixture、真实 Session 和 CLI JSON 状态组成。没有发现需修复的 TUI 业务缺陷。

## 执行与二进制身份

执行顺序为 build、真实复现、失败回归、前端修复、CLI 回归、PTY 补验、全部测试、安全脚本。前三阶段证据独立保存，再开展下一阶段。未执行提交及发布步骤。

- `cargo build` 退出 0，见 `E/build.log` 和 `E/build.exit`。
- build SHA-256 为 `ef924344a11374030e103d9211cdee323f67ac0e6f84de6f220611ec5232f01f`，见 `E/binary-before.sha256`。
- `cargo test --all-targets` 退出 0，49 项通过，3 项 ignored，见 `E/all-tests.log` 和 `E/all-tests.exit`。包含原有 30 项以及并行单元当时新增的测试。ignored 为其他单元的故障测试，不属于 D 验收。
- 实际安全复跑 SHA-256 为 `62dfe6fdc061cf1cc1b40fd332c4391c1f790571ae0c17a27db2c2941cf545ef`，见 `E/binary-final.sha256` 和 `E/verify-identity.log`。
- `node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5` 退出 1，70 项通过、30 项失败。全部剩余失败断言属于并发及派生链接、状态断言。本单元未修改后端以修复它们。报告为 `/private/tmp/packcaptain-cli-safety-runs/verify-zqeBGo/report.json`，副本为 `E/verify-report.json`。脚本记录 `versionExitCode: 0`，版本 stdout 为 `packcaptain 0.1.0`，stderr 为空。

Cargo 使用显式 CARGO_HOME 缓存；HOME、TMPDIR、XDG 路径隔离配置见 `E/env.sh`。CLI 和 PTY fixtures 均使用隔离 HOME、agent-home、state-dir 和 cwd。安全脚本自带每轮隔离目录。

## TUI 五项真实 PTY 证明

新增 `tests/tui_extended_pty.rs`，使用 portable-pty 启动真实二进制并发送按键。`E/pty.log` 记录四测试全部通过。下表路径均相对于 E。每个 `.bin` 为原始终端输出，`.txt` 为去除 CSI 序列后的文本。

| 要求 | 原始 transcript | 断言结果 |
| --- | --- | --- |
| 成员增删 | `pty/members-add.bin`、`pty/members-remove.bin` | `pty/members-and-scope.assertions.txt`。添加 Skill001 后定义为两成员且两个已选 target 各两链接。移除 Skill000 后定义仅 Skill001，两个已选 target 链接均仅指向 skill-001。 |
| 多目标精准范围 | 同上 | 同一断言文件。同步仅勾选三个 Claude project targets 中两个，第三个 target 的完整 JSON state 与链接名及链接目的地在添加、移除后均与原先相同。 |
| 完整取消零业务写 | `pty/cancel-edit.bin` | `pty/cancel.assertions.txt`。修改成员并选择两个 target，在预览确认页 Esc。前后所有 preset 定义、所有 target state、三个项目全部链接名及目的地完全相等，UI 显示已取消未写入。 |
| partial failure | `pty/partial-failure.bin` | `pty/partial-failure.assertions.txt`。预览两个目标后将其中一个项目 `.claude` 放置为 foreign 普通文件。UI 显示部分失败、成功 1、失败 1 及失败 target ID。成功 target 的链接和状态落地，foreign 文件原内容保留。 |
| 长列表 | `pty/long-list.bin` | `pty/long-list.assertions.txt`。90 skills、18 行 PTY，向下到最后项目、回到顶部、搜索 Skill089、Esc 返回和 q 正常退出。终端 resize 触发完整重绘后断言最后项与搜索项实际渲染。 |

复跑入口为 `PACKCAPTAIN_PTY_EVIDENCE=<新证据目录> cargo test --test tui_extended_pty -- --nocapture`。Transcript 即使断言失败也会由 Session Drop 保留。

长列表初次文本断言失败，因为 Ratatui 增量渲染只输出改变的数字，不在串联原始文本中重复完整名称。resize 后完整重绘可观察完整项目名称。该观察修正未修改生产渲染逻辑。

## 文件边界事件与偏离

00:11:39，工具的自动 clippy 修复触及 `src/adapters.rs`、`src/domain.rs`、`src/engine.rs`，另自动格式化白名单内 `src/cli.rs`、`src/tui.rs`。本单元没有主动执行全局 clippy 或 fmt，但自动写入确实违反原白名单。

事件后当场 SHA 分别为 `b588517c9b30ea27bb13464f20ae2b5ddc98498d3f53d4d9941a6ce88f6dc6ae`、`67e090a8ac3d23b1e5afb629770567d23ecbe89d0e3c4d7a3287aa273721956e`、`b165d1b1189da29d295cc618588e4946c057c596d5b6b49a1401498133cc7db4`。

已立即报告主会话。`undo_files` 返回 Nothing to undo，未找到原始副本。主会话批准停止恢复，以避免覆盖并行 A/B 新写入，三文件增量由其所有者吸收并验证。白名单自证因此只能表述为除已披露 clippy 事件外无主动越界修改，不能称硬约束完全满足。`E/before.sha256`、`E/after.sha256` 和 `E/source-diff.log` 保存完整前后证据，后快照含并行单元变更，不能将全部差异归于 D。主会话负责聚合裁定。

本单元主动交付 `src/cli.rs`、`tests/cli_e2e.rs`、`tests/tui_extended_pty.rs` 和本记录。`src/tui.rs` 只有自动格式化变更。`src/main.rs`、`src/storage.rs`、`src/lib.rs`、Cargo.toml、Cargo.lock 前后 SHA 不变。

## 已测、未测与限制

五项 TUI 要求全部已测，没有将 mock 当作 PTY 证明。取消比较的是完整业务语义与链接身份，不将 SQLite WAL/SHM 变化算作业务写入。未证明数据库文件字节完全不变。PTY partial failure 证明真实文件系统失败后的展示，不代表所有故障类型。未测其他终端尺寸、Windows、超出 90 项的列表及真实 agent loader。本单元不重做已有 native loader 研究。

并行后端仍在变化，最终聚合验收须重新 build、记录 SHA 并复跑。临时证据可能被系统清理。

应用 Prove It Works 原则，直接运行真实 CLI 和 PTY，保留退出码、transcript 与状态比较。应用 Test Behavior, Not Implementation 原则，断言实际定义、所有 targets 状态及链接目的地，不依赖 widget mock。应用 Model the Domain 原则，测试使用 Fixture 和 Session 明确隔离资源及终端生命周期，生产逻辑沿用 Clap 分类。

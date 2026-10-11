# PackCaptain 第一版交接

> 历史交接。下文的失败与待办属于当时状态，不代表当前结果。当前状态见 [final-revalidation](packcaptain-v1-final-revalidation.md) 和 [README](../../README.md)。E2 通过有界独立再验收，不是发布授权，未测边界仍保留。

## 接续约束

- 用户已确认产品规则，不重开需求澄清。依据 `docs/unknowns-map.html`、`GLOSSARY.md`、`docs/adr/0001-presets-as-shared-composable-groups.md`。
- 用户已确认本机安装 Pi、Claude Code、Codex，并允许继续隔离运行。不要重装这些 agent。
- 禁止修改真实 agent 配置、删除既有 skills、提交、推送或发布。
- 本轮调度违反 Multi-Agent 约定：整包交给长期实现 owner，主 agent 深入探索与验收材料过多，超过预算。接续必须以新的低占用上下文进行；主 agent 仅分解、调度、收结论与关键证据，不重读完整探索材料。
- 每单元含材料与执行，控制在 20% 上下文内。独立单元并行，依赖单元顺序。派发前重读当前模型配置并核实有效模型/effort。

## 实际源码与接口

Rust/Cargo 项目已实现，二进制为 `target/debug/packcaptain`。
后台文件 `src/domain.rs`、`src/storage.rs`、`src/engine.rs`、`src/adapters.rs`、`src/lib.rs`、`Cargo.toml`。
前端文件 `src/cli.rs`、`src/tui.rs`、`src/main.rs`。
全局隔离参数 `--agent-home <path> --state-dir <path>`，JSON `{ok:true,data}` / `{ok:false,error:{code,message,impact}}`。
目标 ID 为 `claude-code:global` 或 `claude-code:project:<project-id>`，Codex/Pi 同格式。
项目认领 `projects add <absolute-path>`。Preset 定义创建不需 target；启停与同步必须显式 targets。
主要业务子命令和 API 记录于 `packcaptain-v1-implementation-notes.md`。

## 已取得的实际证据，不等于最终完成

1. 主会话独立 CLI 黑盒 111 命令通过，覆盖创建、搜索、多个 Claude 项目、共享成员保护、单项关闭后整组重开、keep/restore、旧目标快照、幂等、预览与错误路径、非托管目录保护、单 Codex 两项目启停。
   - 驱动 `/private/tmp/packcaptain-blackbox-20261011/scripts/acceptance.mjs`。
   - 通过日志 `/var/folders/d9/_0gbv97x6332wsrqhr4fcyj80000gn/T/packcaptain-blackbox-run-5e5rD5/logs/`。
   - 语义状态用规范化 CLI lists 比较；不将 SQLite WAL/SHM 的辅助文件变化误判为业务写入。
2. 主会话执行 `cargo test --all-targets -- --nocapture`，30 项通过。
   - 含真实 CLI 与 PTY，不只是编译。
   - 主会话 PTY 产物 `/private/tmp/packcaptain-frontend-pty-78670/pty-transcript-78670.bin`。
3. TUI 六项缺失/错误的只读复核已确认静态修复；PTY 已操作扫描认领、搜索、创建/启用、单项关闭、编辑冲突、Esc、keep、restore。
   - 尚缺更充分的实际成员增删、多目标策略精准范围、完整取消状态比较、长列表和 partial failure 的 PTY 验收。
4. 三个已安装 agent 的真实发现及安全入口预验证已做。Codex 独占 `.codex/skills` 用户/项目路径、Pi 独占项目 `.pi/skills` 均由主会话真实验证。
   - 原生证据索引见实现 notes；不要重新调研或安装。

## 最新真实安全测试：未通过，必须修复

独立单元 `ab917324345b10ec0` 调用了真实 CLI，共五个新 sandbox，100 项断言，71 通过、29 失败。

- 并发 A/B enable 的进程窗口真正重叠，4/5 轮均成功，1 轮 A 为 partial-failure，报路径被未拥有文件占用。
- 并发 disable A + enable B，2/5 轮 disable A 非零，提示 Claude project disable 尚未验证。
- 外部替换五轮均失败：preview 后将 owned shared link 移走，在原路径放 foreign 真实目录；实际 disable B 返回 exit 0/ok:true，改 B 状态为 disabled，移除正常 B link，没有 NeedsRecovery。foreign 未被覆盖，但不能称为完整成功或安全拒绝。
- `--version` 打印正常但退出码为 2，应修正为正常版本/帮助退出语义。

证据与复跑：
- `node /private/tmp/packcaptain-cli-safety-scripts/verify.js 5`。
- `/private/tmp/packcaptain-cli-safety-runs/verify-hKpYF4/`。
- 外部替换 `round-01/actual-disable-B-after-replacement.json`。
- 并发失败 `round-03/concurrent-enable-A.json`。
- 被测二进制 SHA-256 `4c94df66df9758bf07c11e85f33cdc2c68dd3766ca3ad812ca9534c88a6a837c`，mtime `2026-10-11T22:58:53+0800`。当前源码继续修改，需每次重建记录新 binary 身份。

## 既有审查及待收束事项

核心审查四项：pending recovery 在错误预检前写入、rollback 实际 entry fingerprints 未持久化、重新开启落后 preset 遗留移除成员、暂时 Missing 旧成员丢逻辑历史而复活。前两项及后续修复以后台交接和回归测试核实；部分域回归现已通过，不能凭报告把全部安全问题标完成。

IO 审查六项：真实 partial failure inverse 使用计划态、mixed native/owned routes 跳过但假关闭、尚未提交的其他目标 planned state 提前释放共享入口、pending 创建窗口认领同源外部替换、scan/config 错误猜 disabled、最终 hash 检查到 rename 的非协作外部写竞态。须分别给真实 TempDir/CLI 测试，不用 fake executor 代替全部 FS 证明。微窗口无法保证时明确协议及限制，不夸大安全保证。

原长期后台 owner `a808cd4614d4efe84` 已要求停止承接整包，只将当前修改收束到可编译检查点，并写 `packcaptain-v1-backend-handoff.md`。父会话尚未收到该文件的完成报告，不能猜测其结果。
前端单元 `ace959614aabd3d69` 已完成；后续修改派新鲜小单元，不续派整版任务。

## 下一轮建议单元边界

- 单元 A：`adapters.rs` 的外部替换与 mixed routes 假成功，真实 FS 故障回归。必要最小 engine 变更须协调白名单。
- 单元 B：`engine.rs` / `storage.rs` 回滚 metadata 与 crash 恢复，独立真实进程/持久化测试。
- 单元 C：同一 CLI 并发失败，锁、发现重合并、真实入口幂等回归。依赖 A/B 稳定接口。
- 单元 D：前端 `--version`/帮助退出语义、TUI 增删成员及多目标同步/取消的 PTY 补验。
- 单元 E：最终独立验收和中文 README/证据索引。只收关键证据，明确已测/未测/限制，不自行提交或发布。

目前第一版尚不能宣布完成。业务与正常 TUI 流程已通过，最新真实安全用例仍失败。

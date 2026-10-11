# PackCaptain v1 实现记录

## 状态

2026-10-11。核心实现已启动。Claude Code、Codex 与 Pi 的隔离 native discovery 及链接开关首门均有实测；Pi 项目发现通过独有名称 fixture 与一次性 `--approve` 验证。Claude/Codex 用户级原生配置能持久开关直接目录，但尚未证明对单项目 target 安全；这一边界在 adapter 实现与产品测试中显式处理，不能套用 user-wide selector 做 project-only 操作。所有验证仅限隔离目录，真实 agent 配置保持不变。

## 当前实施单元

- 测试 seams 固定为纯 planner `plan(command, snapshot)`、可执行 CLI 的 JSON 行为、PTY 驱动完整 TUI 流程，以及隔离 agent adapter 的真实 loader/入口操作。CLI/TUI 共享 engine，不模拟 CLI 按键，不用 widget test 替代 PTY。
- 第一 foundation slice 已由实现 owner 直接完成，文件为 `Cargo.toml`、`Cargo.lock`、`src/lib.rs`、`src/domain.rs`、`tests/domain_rules.rs`。首个 A/B共享关闭测试与 UpdatePreset接口测试先 RED 后 GREEN。`plan(&Command, &Snapshot) -> Result<Plan, ConflictReport>` 是纯逻辑 public seam。
- 当前domain tests覆盖建组保留同名不同source ID、明确target、目标共享并集、单项manual close、整组开恢复、成员同步 keep/restore、缺policy零计划/零状态变化、缺失skill不误算closure、按每个target applied snapshot同步、第二target用同definition revision同步旧成员快照并保持第一target不变，以及相同同步幂等。`cargo test --test domain_rules -- --nocapture` 当前11个行为测试全通过。
- 后续独立bounded slices：`storage.rs`/`engine.rs` 实现 SQLite+journal+guarded apply；`adapters.rs` 依据现已实测的 native路径与scope规则实现 discovery/target-local入口；`cli.rs` 与 `tui.rs` 仅调用共享engine。Pi 的 `settings.json.skills` negative directory过滤已由源码调查给出scope，adapter集成需用主会话隔离实验结果，不得用user-wide selector做project-only操作。

## 已确认产品约束

- Preset 是全局共享的 skill 分组。其启停状态属于具体 agent 与 scope 组成的 target。
- 同一个 target 可同时启用多个 preset。关闭其中一个 preset 时，仍被其他开启 preset 需要的 skill 必须保持开启。
- 明确开启整个 preset 时，应重新开启全部可用成员，覆盖此前单项关闭。
- 修改 preset 定义并同步，不等同于明确重新开启整个 preset。
- keep/restore 仅影响所选 targets 上仍保留且相关的旧成员。新增成员正常启用。缺失或不支持的 skill 不算手动关闭。
- 非交互更新若涉及手动关闭成员而没有明确 keep/restore 策略，必须返回影响清单并零写入。
- TUI 与非交互 JSON CLI 必须调用同一核心操作服务。
- Skill 源路径必须稳定。停用入口不得移动或隔离共享源目录，也不得使其他 target 的同源链接失效。

## 领域模型与接口草案

对 `PresetId`、`SkillId`、`ProjectId` 和 `TargetId` 使用各自独立的 ID 类型。`AgentId` 为 Claude Code、Codex、Pi 的封闭枚举。`Scope` 为全局或一个已认领项目。`TargetId` 由 agent 与 scope 组成。

`PresetDefinition` 保存 ID、名称、单调递增 revision 和成员集合。`TargetPresetState` 保存启用状态、上次应用的 revision 和 applied-members 快照。target 的 `manualClosed` 集合只记录用户明确执行的单项关闭。skill 可用性、源身份和受管入口状态必须分别建模。受管入口关闭后 `SkillId` 仍稳定；`sourcePath` 与 target 下的入口路径不是同一概念。不得从路径缺失、`exists()` 结果或悬空符号链接推断手动关闭或源丢失。源身份使用持久 ID 与经过验证的规范源路径；精确移动或重连规则仍需服从原生验证结果。

共享核心提供纯计划函数，例如 `plan(command, snapshot) -> Result<Plan, ConflictReport>`。计划包含预期的定义和 target revision、受影响入口的 no-follow 身份、源身份、具体变更和供人及机器读取的影响清单。预览与冲突判断只读。唯一写入入口 `apply(plan, ports)` 在进程间锁内再次核对前置条件，持久化恢复 journal，执行受保护的文件系统操作，验证结果后更新持久状态。

修改定义并同步时，首个写入前必须检查所有所选 targets 的冲突。存在相关手动关闭且策略为 `Unspecified` 时冲突，不得更新定义。`Keep` 仅保留相关且仍存在于新定义中的旧成员关闭状态。`Restore` 仅清除这些成员的关闭状态。新增成员不属于旧成员策略范围。

SQLite 事务不能涵盖文件系统操作，因此需要持久 journal 和启动恢复。操作入口前再次检查不跟随符号链接的入口身份和预期 link/source 信息。主 agent 隔离实测表明，将其他 target 正在链接的源目录移动到 quarantine，即使目录内容仍保留，也会使另一入口无法读取 `SKILL.md`。证据见 `/private/tmp/packcaptain-shared-source.FgJZGc/observation.json`，其中移动后源内容和另一链接对象仍在，但另一入口读取结果从 true 变为 false。因此源目录路径必须稳定，不得通过移动、改名或隔离源目录来停用入口。优先采用经原生验证的 disable 机制，或仅移除 PackCaptain 自己创建的 target 链接。无法在保持源路径不变的前提下停用时，安全拒绝并提供影响清单。不得覆盖外部替换。若回滚期间发现外部参与者已改变入口，不能覆盖该变化，应将操作标记为需人工恢复并明确报告。

首版事务边界建议为逐 target，而非承诺多个目录间的全局原子性。任何写入前先验证所有 targets 与策略，确保缺少 CLI 冲突策略时包括 preset 定义在内均零写入。之后持久化全局新定义，再将选定 targets 逐个作为独立 journal 事务同步。单个 target 失败时保留其旧 applied-members 快照；其他 target 的结果逐项报告，不得把部分失败表述为全部成功。此边界避免跨卷文件操作不可能满足的原子性承诺。只有原生集成证据显示更简单且安全的方案时，才重新评估。

## CLI 契约草案

非交互命令使用明确 enable/disable 意图，不提供反转状态的含糊 toggle。稳定契约为 `packcaptain --agent-home <path> --state-dir <path> ...`，选项可作为全局 flags。读取命令为 `targets list --json`、`skills list --search <text> --json`、`projects list --json` 和 `presets list --json`。路径认领命令为 `projects add <absolute-path> --json`，只将用户明确给出的目录登记为 Project。定义命令为 `presets create --name <name> --skill <skill-id>... --json`，它只创建全局定义，不要求 target。目标状态命令为 `presets update <preset-id> --add-skill <skill-id>... --remove-skill <skill-id>... --sync-target <target-id>... [--closed-members keep|restore] --json`、`presets enable|disable <preset-id> --target <target-id>... --json`、`skills enable|disable <skill-id> --target <target-id>... --json`。仅启停和同步成员的命令要求显式 targets，global target 也必须显式指定 ID。`--preview` 可用于目标变更命令并保证零写入。

JSON 成功统一 envelope `{ "ok": true, "data": ... }`；失败 envelope `{ "ok": false, "error": { "code": ..., "message": ..., "impact": ... } }` 且非零退出码。list 的 `data` 是数组；skill/preset/project ID 是字符串；target ID 使用 `agent:global` 或 `agent:project:<project-id>` Display 字符串。诊断只写 stderr。

`--agent-home` 在运行时替代三个 agent 的用户根目录。Claude Code 扫描 `<agent-home>/.claude/skills` 与对应用户配置；Codex 扫描 `<agent-home>/.codex/skills` 及共享 `<agent-home>/.agents/skills`，新建 PackCaptain alias 优先用隔离的 `.codex/skills`；Pi 扫描 `<agent-home>/.pi/agent/skills` 及共享 `.agents/skills`，新建 alias 优先用隔离的 `.pi/agent/skills`。Project 下对应扫描 Claude `.claude/skills`、Codex `.codex/skills` 与共享 `.agents/skills`、Pi `.pi/skills` 与共享 `.agents/skills`。以上私有 user/project roots 均有当前安装版本的真实 native loader 实测证据。项目发现仍从所选 Project 的绝对根路径解析。`--state-dir` 完整覆盖 PackCaptain SQLite、journal、lock 与临时恢复状态目录。`targets list` 输出可直接复用的稳定 target IDs。每个 CLI E2E 和 PTY fixture 必须显式传入全新 `--agent-home` 与 `--state-dir`，并显式设置项目 cwd/root，不依赖 macOS `ProjectDirs` 或当前 HOME 推断。

更新定义时必须给出 sync targets；若有相关旧手动关闭且未提供策略，返回 JSON impact/conflict 到 stdout、非零退出码，且包括定义在内零写入。成功结构化结果写 stdout，诊断写 stderr。TUI 的搜索、多选和确认调用同一个 engine interface，不模拟 CLI 按键。

## 技术选型建议

采用 Rust stable 与 Cargo。CLI 候选使用 Clap 4；TUI 使用 Ratatui 0.30 与 Crossterm；结构化输出使用 Serde 与 serde_json；本地状态使用 rusqlite；跨平台应用目录使用 directories；跨进程 advisory 锁使用 fs2；TUI 端到端测试使用 portable-pty。

以上为候选方案，尚未开始实现。已通过 Context7 查询各集成的用法，包括 Ratatui 终端生命周期与事件处理、Clap derive 与参数校验、rusqlite 事务与 busy timeout、directories 的 ProjectDirs、fs2 advisory 锁，以及 portable-pty 启动进程、输入输出和调整终端大小。Advisory 锁不能防止 agent 或编辑器修改受管路径，因此 apply 还必须在每项操作前复核文件系统前置条件。

## 候选文件边界

原生 discovery 验证门通过后，首版保持较少模块。候选路径为 `src/domain.rs`，承载 ID、状态、命令和纯计划逻辑；`src/engine.rs`，承载共享 plan/apply 服务；`src/adapters.rs`，承载三个已经验证的 agent 适配器；`src/storage.rs`，承载 SQLite、操作 journal 与恢复；`src/cli.rs`，承载 JSON 输出和退出行为；`src/tui.rs`，承载交互与展示。只有实现过程中证明边界确有价值时再拆分模块。行为测试候选为 `tests/cli_e2e.rs`、`tests/tui_pty.rs`、`tests/adapter_contract.rs` 与 `tests/transaction_safety.rs`。

以上均为拟议边界，尚未创建实现文件。

## 验证门与现有证据

- 初步 CLI 探测报告版本为 Claude Code 2.1.295、Codex CLI 0.162.1 和 Pi 1.1.0。Claude `agents --json`、Codex `mcp list`、Pi `list` 只检查了 agents/MCP/packages，不能证明 skill loader 发现。初次隔离目录没有保留命令日志，因此不构成最终证据。
- Codex CLI 0.162.1 的真实 app-server loader 发现验证通过。隔离 HOME/CODEX_HOME 下放入 `.agents/skills` 用户级与项目级 fixtures，通过 app-server `initialize` 和 `skills/list` RPC 返回两项，scope 分别为 user 与 repo，均为 enabled。输入、stdout、stderr、退出码和本地生成 schema 验证产物位于 `/private/tmp/packcaptain-agent-discovery-u26ms0/logs/codex-rpc-*`，父会话已直接读取验收。授权后重新在干净隔离环境完成共享 symlink A/B `skills/list` forceReload 测试：初始两 target 均发现 skill；移除 A 链接并刷新后 A 不再发现，B 仍发现；重建 A 链接刷新后两边都发现。app-server 退出码为 0。证据位于 `/private/tmp/packcaptain-native-clean-20261011/logs/`，包括 `codex-command.txt`、`codex-requests.jsonl`、`codex-stdout.jsonl`、`codex-stderr.txt`、`codex-exit.txt`、`codex-version.txt` 与 `fixture-tree.txt`。
- Claude Code 2.1.295 的隔离 PTY `/skills` 命令真实返回 `Skills / 2 skills`、`pc-claude-project project` 和 `pc-claude-user user`，两项均显示启用标记 `✔`，界面提示可用 Enter/空格切换。输出 artifact 为 `/private/tmp/packcaptain-agent-discovery-u26ms0/logs/claude-skills-command-pty.txt`，debug 日志为 `/private/tmp/packcaptain-agent-discovery-u26ms0/logs/claude-skills-command-debug.log`。父会话已直接读取并核对 PTY 清单和 debug 中隔离 user/project roots、`Loaded 2 unique skills` 与 `getSkills` 两项。另有隔离链接切换实测：fresh A target 从 2 项变为 3 项，移除 A 的入口后 fresh loader 回到 2 项，再建 A 链接后 fresh loader 回到 3 项；共享源仍存在，B target symlink 保留。debug 证据为 `claude-link-open-debug.log`、`claude-link-closed-debug.log`、`claude-link-reopened-debug.log`，共享保留证据为 `claude-shared-preservation-after-disable.txt`，均位于 `/private/tmp/packcaptain-agent-discovery-u26ms0/logs/`。同名 fixture 的 `/skills` PTY 实际显示 project 与 user 两个同名项，并分别标记来源，加载总计 5 个 unique skills；原始清单为 `claude-collision-pty.txt`。这要求 PackCaptain 按来源目录身份区分同名 skill，不得仅按名称折叠。上述是 Claude 隔离真实 loader 和入口刷新证据，不涉及真实配置。
- 既有直接 skill 目录的隔离停用/恢复验证位于 `/tmp/packcaptain-native-direct.OXGPIK/`。Claude 2.1.295 在隔离 user `settings.json` 用 `skillOverrides[name]` off/on 后，新进程发现数为 1→0→0→1→1；Codex 0.162.1 用原生 `skills/config/write` RPC 向隔离用户 `config.toml` 写 canonical `SKILL.md` 路径后，fresh `skills/list` enabled 为 true→false→false→true→true。10 个进程 exit 0，源路径、inode、SHA-256 不变。两种机制仅证明 user/global 配置可持久开关；没有测试 alias 或其他 target 的副作用，不能用于声称单项目 target 局部安全。按具体操作 scope，未经验证时须拒绝将 user-wide override 用作 project-only disable。artifact 索引为 `evidence-index.json`，环境为 `lifecycle-environment.json`，完整结果为 `lifecycle-results.json`，不变性检查为 `verification.json`。重放命令见索引，fixture 脚本为 `lifecycle.mjs` 和 `verify.mjs`。
- Pi CLI 1.1.0 的干净隔离用户级重测使用 `env -i`、全新 HOME/XDG/Pi 配置目录及直接调用 pnpm shim 的无模型 `get_commands` RPC，退出码 0，发现隔离 user skill。随后主会话完成精准项目级实验：环境变量 allowlist、cwd 指向隔离 fixture、审阅自建 skill 后仅本次进程使用一次性 `--approve`，没有写全局 trust/always store。三个新进程均 exit 0，原生结果顺序为 project+user、仅 user、project+user，sourceInfo.scope 明确区分 project/user，源内容不变，17 条断言通过。复跑入口 `/private/tmp/packcaptain-pi-proof.HGQRY2/prove.mjs`，原始 run artifacts 位于 `/private/tmp/packcaptain-pi-proof.HGQRY2/run-1vzDbf/`。问题根因是 Pi 1.1 `defaultProjectTrust=ask`，无交互 RPC 默认不批准项目资源；另一个早期试验把 user/project 指向同一 realpath 且同名，触发去重，不能据此判 unsupported。此前拒绝执行的原始记录仍保留在 `/private/tmp/packcaptain-agent-discovery-u26ms0/logs/pi-permission-denial.txt`。只读源码调查还确认 Pi 1.1 `settings.json` 的 `skills` 可用 `-<absolute skill directory>` 精确排除，并以数组顺序处理后续包含/排除；用户 settings 仅过滤用户自动发现根，项目 `.pi/settings.json` 仅过滤受信项目自动发现根。主会话进一步实测发现这些规则并非完全隔离：user `-shared-global-dir` 可只关 Pi global而 Codex B同源仍 enabled；但 B 项目 `settings.json -alias-dir` 后重开用户 global `skills=[]`，Pi B结果仅剩 `pc-user-proof`，同源 global `pc-shared-proof` 被项目负规则遮蔽。原始证据 `/private/tmp/packcaptain-pi-proof.HGQRY2/direct-run-8jHqDl/logs/pi-global-restored-project-does-not-mask-user-b-stdout.jsonl`。因此禁止对与用户基础 canonical source 相同的项目 alias 使用 Pi native negative rule；PackCaptain-owned project alias 优先移除自己的入口链接，让 user/global fallback继续发现。若项目 direct入口与用户基础同源且无法安全移除其逻辑入口，应安全拒绝 project-only disable，不得宣称项目配置永不影响global。不同canonical source的项目direct目录负过滤须等主会话验证后再启用。
- 首轮安全原型没有证明真正并发，也没有正确处理悬空链接。修正后的原型由协调方独立复跑，27 条断言通过，覆盖两个进程对独立 child 分支 `disableUnderLock` 的锁竞争、悬空与相对链接、missing 路径不被重写、共享链接源保护、拒绝覆盖外部替换、回滚行为和幂等。但该并发分支不是主 `disable`/回滚入口，因此只能作为机制预备证据，不能视为真实产品入口并发已验证。产品集成测试必须让并发进程调用同一个真实 CLI/engine 写入口，并覆盖 crash recovery 与源目录内容变化保护。
- 源目录迁移的实测反例已单独记录。关闭 target 入口不得靠移动源目录完成。原生机制若不能保持源路径稳定，则必须安全拒绝。
- 父会话验收记录位于 `/private/tmp/packcaptain-safe-entry-1791704656/PARENT_VERIFIED.md` 与 `/private/tmp/packcaptain-agent-discovery-u26ms0/PARENT_DISCOVERY_REVIEW.md`。原生 discovery owner 必须将实际命令、stdout、stderr、退出码、版本、隔离 HOME/config 路径与 loader 原始输出写成 artifact，即使探测失败也要保留。Claude Code、Codex、Pi 的结果必须标注为真实 loader 实测、文件系统模拟或文档核验。
- TUI 必须通过 PTY 驱动完整用户流程。Widget 单测或编译成功都不足以验收。
- 安全集成测试需覆盖预览后的外部替换、真实并发进程、幂等重试、故障回滚、悬空与有效符号链接、共享源保护，以及真实 agent 配置不变。

## 偏离与未决事项

任何技术决定不得覆盖已确认的产品语义。原生入口布局、停用与刷新行为、loader 发现行为、路径身份规则和 agent 特有边界，仍须等待隔离验证。未获授权发布 issue、提交、推送或发布版本。

## E3 最终检查点，2026-10-11

这是追加检查点，不覆盖上文历史草案或早期未验事项。当前 E2 已通过有界独立再验收，权威记录为 [final-revalidation](packcaptain-v1-final-revalidation.md)。用户运行入口为 [README](../../README.md)，最终证据索引为 [verification](../../docs/verification.md)。历史交接已在顶部注明当前状态入口。

- E3 文档交付，待父会话验收。README、证据索引、历史标记及本检查点已写入。自己的核验记录为 [unit-e3-notes](unit-e3-notes.md)。
- fixed CLI SHA256 为 `88cb8dbbedacd332e27ccc8b088643f43e175434584c9fea9cc52eb2aa2fdb44`。只读 help 与全新 E3 沙盒核实 CLI 示例，无全量构建重跑。TUI 行为采用 E2 fixed PTY 证据。
- 文档验证确认 create 返回操作报告，Preset ID 必须从 presets list 取得，README 不假设 create.data.id。首次工具误读导致 preset-not-found，保留原失败证据，修正工具后通过，不改业务。
- TUI 持久 pending 确认仍保守拒绝。合法 CLI 操作可恢复但也执行业务意图，没有独立 recover 命令，不能建议随意开关来恢复。
- Unknown 不等于 disabled。共享物理 entry 保留不保证先关闭 Target 的 Agent 停止加载。同 state-dir 合作锁、非协作 writer 最终微窗口、同 UID receipt 篡改、删除 crash、跨平台和掉电边界在 README 与索引明确保留。
- 未读取 auth、改真实配置、提交、推送或发布。暂存区未检查，不声称为空。开发版不是发布版，文档完成不外推整体安全保证。

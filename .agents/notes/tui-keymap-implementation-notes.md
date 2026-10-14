# TUI 操作统一

## 状态

- U1 操作合同调查 done。实际代码调查报告 `/tmp/packcaptain-how-report/REPORT.md` 已核对 `src/tui.rs` 的提示、输入态、列表构造。下一步完成双候选比较。
- U2 最小设计 done。两个候选与 cross-judge `e420d6fa-c741-4a36-b3aa-64d86f165b95` 已完整读取。选最小 SkillActions 基座，加入同源屏幕能力与 Confirm 方向键滚动，不扩大为完整 ActionMenu 重写。源码核对 goto_targets 已清空 checked，review 的残留风险需回归守卫而非重复添加清空逻辑。
- U3 源码与沙盒指引 done。主agent已读实际改动及event，独立只读源码审查 run `75d22834-bd97-4c76-9dc0-c804725a5e12` 无问题。主agent实跑 `cargo test --locked --bin packcaptain` 26/26通过，`bash -n scripts/try-sandbox.command`通过，证据 `/tmp/packcaptain-keymap-parent-bin.log`。完整真实终端体验仍属U4/U5，不外推整体完成。
- U4 PTY 迁移 done。长列表单test修复已读实际diff与result，根因包含首项误认为Skill000，列表实际按ID排序。90项逐项上下导航保留，边界noop断言正确首末行和进程存活。证据 `/tmp/packcaptain-pty-boundary-fix`。测试断言独立审查 run `4ade8826-2c60-4cf1-b843-7a9a3689cb70` BLOCK，确认同操作partial failure被两命令弱化、边界读旧frame、主PTY缺失0断言移除三项真实问题。唯一writer `e1105447-3375-4f50-8432-acb4a9e5b130` 恢复覆盖与事件屏障，保留scope合同。已读 `/tmp/packcaptain-pty-coverage-fix/REPORT.md`。主agent最终partial实跑通过，随后全量cargo136/0/0通过，证据 `/tmp/packcaptain-final-current`。独立修复复核 run `df7fcb7f-e27a-445d-a58b-4ef4e3d1d74c` OK，确认2个P1和1个P2均resolved，无新问题。旧unused helper非阻塞，未扩大清理范围。
- U5 全量验收 done。独立run `8c6c743a-4251-4061-838a-92b287101ec4` 对测试整改前版本实跑136 passed/0 failed，build和bash-n通过，证据 `/tmp/packcaptain-final-validation/result.md` 已读。expect脚本因ANSI delta观测超时未完成烟测，不能通过。新run `0b93921d-4c64-4e10-8a51-6e8f59eadc6e` 只用/tmp Rust PTY harness+screen parser跑真实脚本，禁止联网/项目改动。整改后的全量主agent已实跑 `cargo test --locked -- --test-threads=1` exit0，136 passed/0 failed/0 ignored，包括两套PTY15个测试；build与bash-n再通过，日志 `/tmp/packcaptain-final-current`。真实沙盒脚本两次PTY运行exit0，证据 `/tmp/packcaptain-script-smoke-v2/result.md`、commands.txt、transcript.bin和screens.txt。主agent已读实际报告与按键序列，实跑cmp before/after取消快照exit0。验证全局返回、临时project、新建Preset成员Space与确认、Agent开启预览Esc取消、q与脚本正常退出。没有后续源码修改。

## 范围与授权

用户要求完成功能修改。接续交接记录的真实 Rust TUI 按键统一、启动指引和 PTY 验收。不重做 HTML，不改 engine 合同，不提交、推送、创建 PR、发布，不写入真实 Agent HOME 或真实项目配置。

## 入口与吞吐检查

源码事件、渲染和测试行为紧密耦合，采用有界设计调查、单源码负责人、独立终端测试迁移、独立验收。每单元材料、执行与报告控制在20%上下文预算内。Grok 与 Kimi 暂受限，实现用 GLM low，判断和验收用 Sol medium。

- Blocking first steps. 调查和设计先于源码修改，先定义行为合同，不能只改 footer。
- Independent workstreams. 两个独立候选只读并行，源码与测试按合同依赖推进。
- Shared mutable state. 单 cwd 单 writer。候选仅写独立 artifact。
- Smallest safe decomposition. 一个源码负责人处理同一事件与渲染接缝，之后测试迁移与独立验收串行，避免旧序列跟随中间设计反复修改。

## 最终验收

本轮U1到U5均完成。全量136测试通过且独立源码审查、测试覆盖复核通过，真实脚本烟测通过。现有domain仍拒绝不可用Skill的启停预览，该限制如实显示，不绕过。旧test helper unused警告非阻塞。本轮无commit/push/PR/release，未写真实Agent HOME或真实项目配置。证据日志位于/tmp，仅本机可复跑，不等同远程CI或不同平台验收。

## 初步决策

四种主操作分别为导航、勾选、继续或确认、返回。Space 只能勾选，不直接写入或猜测 Agent 聚合状态。Skill 启停改为可见选项，用户无需记忆 e/d。保留 Scope+Screen+Draft，不引入完整 reducer 框架。

## 设计裁决

GLM 最小结构作为 base。接纳 DeepSeek 的同源屏幕能力派生提示和事件通用键门控，以及 Confirm 的箭头滚动。删除仅测试 nav_key 副本，改测 App::event。新动作采用类型区分，避免新增标签驱动分支。不重写已有全部动作或持久缓存 merged Snapshot。Skill 开关意图明确，Agent 状态放在目标行逐项呈现，未知不能由缺失推导关闭。本轮不额外添加 SkillPick 聚合状态，避免 mixed 与 manual_closed 的无依据规则。缺失Skill仍可请求关闭已启用目标；未知不能显示为关闭只约束状态呈现，不限制明确关闭意图。

## Deviations

提交与 PR 步骤跳过，本轮无授权。架构候选模型因 Kimi 临时受限改为 GLM 与 DeepSeek。首次sandbox验收隔离CARGO_HOME导致offline缺clap，worker在纠正指令到达前联网获取临时依赖，已报告执行偏离；后续禁止联网，复用现有cache，区分构建环境与Agent运行时隔离。

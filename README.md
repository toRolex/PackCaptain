# PackCaptain 开发版运行说明

PackCaptain 用命名 Preset 管理 Claude Code、Codex 和 Pi 的 Skills，并在用户全局或已认领项目的 Target 上分别启停。Preset 定义全局共享，同一 Target 可以同时开启多个 Preset。关闭一个 Preset 时，其他开启 Preset 仍需要的 Skill 保持开启。

这是 Rust Cargo 开发版，不是已发布版本。当前结论为 [E2 有界再验收通过](.agents/notes/packcaptain-v1-final-revalidation.md)，不是全仓安全保证或发布授权。旧交接中的失败属于历史状态。完整运行证据与未测范围见 [最终证据索引](docs/verification.md)。

## 构建并启动隔离开发环境

需要 Rust stable 与 Cargo。本轮文档没有重新构建，以下是开发入口，E2 的实际构建结果见证据索引。

```sh
cargo build
sandbox=$(mktemp -d)
mkdir -p "$sandbox/agent-home" "$sandbox/project"
export HOME="$sandbox/home"
export XDG_CONFIG_HOME="$sandbox/home/config"
export XDG_STATE_HOME="$sandbox/home/state"
mkdir -p "$HOME"
pc() {
	./target/debug/packcaptain --agent-home "$sandbox/agent-home" --state-dir "$sandbox/state" "$@"
}
pc --help
pc tui
```

也可以用 Cargo 启动 TUI。省略子命令也启动 TUI。

```sh
cargo run -- --agent-home "$sandbox/agent-home" --state-dir "$sandbox/state" tui
```

先退出 TUI，再运行下文 CLI。`--agent-home` 替代三个 Agent 的用户根目录，`--state-dir` 隔离 PackCaptain 的 SQLite、journal、锁和恢复状态。项目入口仍位于明确认领的项目目录，`--agent-home` 不会隔离真实项目。因此只在新建沙盒项目中试运行，不要把真实项目路径填入试验命令。

## 列出 Skills 并认领项目

在隔离用户目录创建一个测试 Skill。

```sh
mkdir -p "$sandbox/agent-home/.claude/skills/demo-a"
cat > "$sandbox/agent-home/.claude/skills/demo-a/SKILL.md" <<'EOF'
---
name: demo-a
description: Sandbox skill
---
# Demo
EOF
pc skills list --search demo --json
pc projects scan --root "$sandbox" --json
pc projects add "$sandbox/project" --json
pc projects list --json
pc targets list --json
```

扫描只列出候选，认领只登记明确选中的目录。复制 `skills list` 返回的 Skill ID，复制 `projects list` 返回的 Project ID。不要把名称当作 ID。

三个 Adapter 的全局 Target ID 是 `claude-code:global`、`codex:global` 和 `pi:global`。项目 Target ID 是 `claude-code:project:<project-id>`、`codex:project:<project-id>` 和 `pi:project:<project-id>`。以 `targets list` 的实际结果为准。

PackCaptain 的首选用户 Skill 根目录分别是 `.claude/skills`、`.codex/skills` 和 `.pi/agent/skills`。项目根目录分别是 `.claude/skills`、`.codex/skills` 和 `.pi/skills`。Codex 和 Pi 还扫描用户或项目的 `.agents/skills`。发现外部入口不等于 PackCaptain 已拥有该入口。不能安全关闭的非托管入口会被拒绝，不能据此宣称已禁用。

## 创建并启停 Preset

将下面变量替换为列表返回的 ID。`presets create` 只创建共享定义，不会启用任何 Target。创建结果是操作报告，使用 `presets list` 获取 Preset ID。

```sh
SKILL_ID='<skills list 返回的 ID>'
PROJECT_ID='<projects list 返回的 ID>'
TARGET="claude-code:project:$PROJECT_ID"
pc presets create --name writing --skill "$SKILL_ID" --json
pc presets list --json
PRESET_ID='<presets list 返回的 ID>'
pc presets enable "$PRESET_ID" --target "$TARGET" --preview --json
pc presets enable "$PRESET_ID" --target "$TARGET" --json
pc presets disable "$PRESET_ID" --target "$TARGET" --json
```

启停和成员同步必须显式指定 Targets，包括全局 Target。重复 `--target` 可以一次选择多个 Targets，各 Target 独立报告结果，不承诺跨 Target 原子提交。

```sh
pc presets enable "$PRESET_ID" \
	--target "codex:project:$PROJECT_ID" \
	--target "pi:project:$PROJECT_ID" --json
```

`--preview` 只读计划，不应用业务变化。CLI 取消的方式是不执行对应的非预览命令。预览不是之后执行的固定授权，执行时仍会重新检查状态。

## 单项关闭并同步成员

单独关闭 Skill 会记下 manual-close。明确重新开启整个 Preset 会重新开启其全部可用成员，不永久保留此前的单项关闭。

```sh
pc skills disable "$SKILL_ID" --target "$TARGET" --json
pc skills enable "$SKILL_ID" --target "$TARGET" --json
```

编辑 Preset 时，重复 `--add-skill` 或 `--remove-skill` 指定成员变更，重复 `--sync-target` 指定同步范围。`NEW_SKILL_ID` 必须是实际列表中的另一个可用 Skill ID。

```sh
NEW_SKILL_ID='<另一个 skills list 返回的 ID>'
pc presets update "$PRESET_ID" --add-skill "$NEW_SKILL_ID" \
	--sync-target "$TARGET" --closed-members keep --preview --json
pc presets update "$PRESET_ID" --add-skill "$NEW_SKILL_ID" \
	--sync-target "$TARGET" --closed-members keep --json
pc presets update "$PRESET_ID" --remove-skill "$NEW_SKILL_ID" \
	--sync-target "$TARGET" --closed-members restore --json
```

Keep 保持本次同步涉及的旧成员此前的单项关闭。Restore 重新开启这些相关旧成员。新成员正常启用，缺失或不支持的成员不算 manual-close。成员同步不是整组重新开启。

若存在相关 manual-close 而 CLI 未提供策略，CLI 返回影响清单和非零退出码，不写入定义或目标变化。JSON 成功结构为 `{ "ok": true, "data": ... }`，失败结构为 `{ "ok": false, "error": { "code": ..., "message": ..., "impact": ... } }`。检查退出码和逐 Target 结果，不要把部分失败当作全部成功。

## 在 TUI 中预览并确认

运行 `pc tui`。按界面键位提示搜索、选择成员和多选 Targets。编辑并同步成员遇到相关 manual-close 时，TUI 预选 Keep，但仍要求你确认。策略仅作用于这次同步，不保存为固定偏好。

在预览页核对 Targets 和影响，再确认。按 Esc 取消。确认时若状态已经变化，TUI 显示新预览，要求重新确认，不静默应用旧预览。每次确认保留原命令与明确 Targets，不自行扩大范围。当前实际 PTY 验证见证据索引，帮助输出本身不证明这些行为。

## 处理 Unknown 和未完成操作

配置解析、读取或扫描失败时，Target 的观察为 Unknown，不是 disabled，也不是 Skill 已缺失。列表中的存储状态仅代表最后可信 intent。先检查诊断并修复配置或权限，再重新预览。Unknown Target 的写操作会失败，健康 Target 可能独立成功。涉及 Unknown 同步 Target 的 Preset 定义更新保守拒绝。

TUI 确认遇到持久 pending journal 会拒绝，不自动恢复。项目认领入口使用自己的恢复路径，不等同于 TUI 命令确认。没有独立 `recover` 命令。

现有合法 CLI 业务操作可以在受保护入口自动恢复，但恢复后还会执行该命令自身的业务意图。先确认该操作及 Targets 确实是你要执行的，再运行 CLI，完成后回到 TUI 重新预览。不要为了恢复随意开关 Skill 或 Preset。出现 `NeedsRecovery` 时保守停机，保留证据并先排查原因，不要手工乱删 journal 或 anchor。

## 遵守安全边界

- 进程锁只协调同一个 `--state-dir` 的合作进程。不支持多个 state-dir 共同管理同一个 Target。
- 操作会检查入口身份，但非协作 writer 在最终检查到 rename 或 unlink 之间仍有微窗口。修改同一入口的 Agent、编辑器或其他工具必须暂停，或使用同一合作锁协议。
- 私有 receipt 用于恢复身份核验，不是抵御同 UID 恶意篡改的安全边界。保留私有恢复目录不等于已完成自动恢复。
- 多 Target 逐项提交。删除过程 crash、多 route 部分 mutation、细分实杀节点、清理失败、掉电耐久性没有全覆盖。
- 共用物理 entry 为其他 Target 保留时，先关闭的 Target 逻辑 intent 可以是 disabled，但不保证该 Agent 不再加载该 entry。
- E2 在真实 macOS 上验证。Linux、Windows、其他文件系统与终端未验证。本轮没有真实 Agent 新会话加载检查，不声称 Agent 已加载或卸载 Skill。
- 用户授权仍禁止提交、推送和发布。本开发版及验收通过都不改变该限制。

术语见 [GLOSSARY](GLOSSARY.md)。Preset 共享、可组合和 Keep/Restore 的规则见 [ADR 0001](docs/adr/0001-presets-as-shared-composable-groups.md)。

#!/bin/bash
# macOS: double-click this file to try PackCaptain in an isolated sandbox.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sandbox=""

finish() {
  status=$?
  if [[ -n "$sandbox" ]]; then
    printf '\n体验沙盒保留在：%s\n' "$sandbox"
    printf '演示项目：%s/project\n' "$sandbox"
    printf '本次没有使用你的真实 Agent HOME 或真实项目。\n'
  fi
  if [[ "$status" -ne 0 ]]; then
    printf '\n体验未完成（退出码 %s），请把上方报错发给我。\n' "$status"
  fi
  if [[ -t 0 ]]; then
    read -r -p '按回车关闭此脚本…' _ || true
  fi
}
trap finish EXIT

if [[ ! -t 0 || ! -t 1 ]]; then
  printf '请在终端运行，或在 Finder 中双击这个 .command 文件。\n' >&2
  exit 1
fi

# Finder-opened terminals may not yet have Cargo's directory in PATH.
if [[ -d "$HOME/.cargo/bin" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
  printf '未找到 Cargo。请先安装 Rust / Cargo，再重新打开此脚本。\n' >&2
  exit 1
fi

cd "$repo_root"
printf '正在构建 PackCaptain 开发版…\n'
cargo build --locked --target-dir "$repo_root/target"
binary="$repo_root/target/debug/packcaptain"
if [[ ! -x "$binary" ]]; then
  printf '未找到构建产物：%s\n' "$binary" >&2
  exit 1
fi

sandbox="$(mktemp -d /tmp/packcaptain-demo.XXXXXX)"
mkdir -p "$sandbox/home" "$sandbox/project"
for name in demo-writing demo-review; do
  mkdir -p "$sandbox/home/.claude/skills/$name"
  printf '%s\n' '---' "name: $name" "description: Sandbox skill for $name" \
    '---' "# $name" > "$sandbox/home/.claude/skills/$name/SKILL.md"
done

pc() {
  env HOME="$sandbox/home" \
    XDG_CONFIG_HOME="$sandbox/home/config" \
    XDG_STATE_HOME="$sandbox/home/state" \
    "$binary" --agent-home "$sandbox/home" --state-dir "$sandbox/state" "$@"
}

pc projects add "$sandbox/project" --json
printf '即将打开 TUI：\n'
printf '1. ↑↓ 或 j/k 移动，T 切换主题，Esc 返回（作用域首页为退出），q 退出。\n'
printf '2. Enter 进入作用域；Preset 列表里选中项按 Enter 查看操作，新建/编辑走列表可见行。\n'
printf '3. Skills 管理 Enter 打开 Skill 操作菜单，选择开启/关闭，再 Space 勾选目标。\n'
printf '4. 选择临时项目的 Claude Code Target，按一次 Enter 预览；核对目标与变更后，再按 Enter 确认写入。\n'
printf '5. 新建 Preset：选中新建行按 Enter，输入名称后 Enter；Space 勾选成员，Enter 预览，核对后再 Enter 写入。\n'
printf '6. 关闭 Skill 或 Preset：在操作菜单选择关闭，Space 勾选目标，Enter 预览，核对后再 Enter 写入；预览时 Esc 取消不写入。\n'
printf '请勿添加真实项目路径；这里仅体验演示 Skill 管理。\n\n'
read -r -p '按回车进入 TUI…' _
pc tui

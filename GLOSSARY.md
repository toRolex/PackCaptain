# PackCaptain

PackCaptain 管理配套 skills 的分组，以及这些分组在不同项目和 agent 下的启停状态。

## 领域术语

**Skill**。
一份可由 agent 发现和加载的技能指令。
_避免_：把 agent 本身称为 skill。

**Preset**。
在所有项目和 agent 间复用的命名 skill 集合，可作为一组批量启停。用户所说的 skill 整合包对应这个概念。
_避免_：互斥工作模式、完整配置快照。

**Agent**。
发现、加载和使用 skills 的工具，例如 Codex 和 Pi。
_避免_：把 preset 作者称为 agent。

**Scope**。
管理操作针对的范围，包括用户全局范围和单个项目范围。
_避免_：把 skill 来源作者称为 scope。

**Target**。
一个 agent 与一个 scope 的组合，是 preset 启停状态所属的管理目标。
_避免_：把 preset 定义与其在某个 target 下的状态混为一谈。

**Project**。
用户管理 skills 时针对的本地项目目录。PackCaptain 发现的候选目录经用户选中后才纳入管理，用户也可以主动添加目录。
_避免_：把 preset 本身称为项目。

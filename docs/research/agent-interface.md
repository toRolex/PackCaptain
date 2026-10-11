# PackCaptain 的 agent 操作入口

本次调查比较非交互 CLI 与 MCP。第一版选择 CLI，并提供 JSON 输出。此结论针对 PackCaptain 的本地管理任务，不代表所有 agent 工具都应采用 CLI。本文只记录调研和设计决定，没有实现或安装接口。

## 已核实的实践

[GitHub CLI 官方格式文档](https://cli.github.com/manual/gh_help_formatting)提供 `--json`，把命令结果转换为供脚本消费的结构化数据。这证明成熟 CLI 可以同时提供人工阅读和机器读取方式，不证明它必然优于 MCP。

[Command Line Interface Guidelines](https://clig.dev/)建议提供 JSON 输出，将结果写入 stdout、提示写入 stderr，成功返回零退出码、失败返回非零退出码。指南也提供 `--dry-run` 惯例，要求仅在交互终端使用提问。非交互模式缺失必要输入时应报错，而不是等待用户输入。该指南不是 agent 专用规范。

[Pi 官方 shell 文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/shell-aliases.md)说明模型可以调用内建 bash 工具。[Pi 变更记录](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/CHANGELOG.md)记录 0.99.0 加入内建 MCP 扩展。因此不能用“Pi 不支持 MCP”作为拒绝 MCP 的理由。

[MCP 官方规范](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/docs/2026-07-28/learn/architecture.mdx)定义工具发现与输入 schema。[结果 schema](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2026-07-28/schema.ts)支持结构化内容和错误标记。MCP 是可采用的标准接口，但仍需要服务端和客户端接入配置。

## PackCaptain 的决定

第一版采用非交互 CLI 加 JSON 输出。TUI 与 CLI 共用 preset 操作规则，不把 CLI 做成自动按键控制器。以下内容是设计要求，不是已实现能力。

- 提供列出 skills、projects、presets 的查询操作。
- 提供创建和修改 preset 的操作。
- 提供对明确 targets 开启和关闭 preset 的操作。
- 提供只描述变更、不执行写入的预览操作。
- 非交互调用缺少必要参数时返回错误，不弹出菜单。
- JSON 模式将结构化结果写入 stdout，将诊断写入 stderr。
- 成功返回零退出码，失败返回非零退出码。
- 写操作必须明确指定 targets，不默认修改所有项目或全局范围。
- 批量操作报告各 target 的结果，不把部分失败包装成全部成功。
- 重复设置同一目标状态应收敛，不提供会因重试而反转状态的含糊 toggle 命令。

该选择的理由是先提供单个本地程序入口，不额外建设 MCP 接入层。命令解析和 JSON 序列化使用所选技术栈的成熟库，技术栈尚未确定，不提前指定依赖。

## 边界与后续验证

本轮没有逐一实测三个 agent 对 PackCaptain CLI 的调用，程序尚未实现。上线前需验证非交互查询、创建、预览和明确目标写入，确认输出可解析、错误不阻塞等待、批量部分失败可见。

第一版不提供 MCP 或 HTTP 服务。未来确有只允许 MCP、不提供 shell 的客户端需求时，再评估薄 MCP 适配器，并复用相同操作规则。

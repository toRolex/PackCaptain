# E3 最终文档

## 状态与检查点

- E3，done，仅本单元文档核验完成，等待父会话最终接收。产物为 README.md、docs/verification.md、实现记录追加检查点、历史交接顶部标记和本 notes。证据为 /private/tmp/packcaptain-e3-SlFafb。无后续实施。
- 检查点依次为读取 E2 与接口记录、固定 CLI 示例验证、文档及冻结文件 SHA 核对。

## 决策

README 采用运行 how-to，verification 采用证据 reference。保留实现记录和交接历史，不用旧失败覆盖 E2 当前有界通过。Prove It Works 决定使用 E2 固定二进制的真实 help 与全新临时目录验证，不重新构建或运行全量测试。

## 范围

不改源码、tests、Cargo 或其他文档。不使用子派发、动态 workflow、Git/jj、提交、推送或发布。README.md 与 docs/verification.md 入口确认不存在。CLAUDE.md 是 AGENTS.md 的链接。冻结清单保存于 /tmp/packcaptain-e3-forbidden-before.txt。

## 核验结果

- 固定 binary 前后 SHA256 均为 88cb8dbbedacd332e27ccc8b088643f43e175434584c9fea9cc52eb2aa2fdb44。真实 help 核实全部示例涉及子命令与 flags。新沙盒实际执行列表、搜索、认领、扫描、创建、启停、preview、Keep/Restore 同步及 Codex/Pi 明确双项目 Targets，examples-result.txt 为 PASS，update-preview.json exit0。preview 对比目标存储状态相等。无真实默认目录 mutation。
- 第一次验证工具误把 create 操作报告当 Preset 对象，preset ID 为 undefined，CLI 正确返回 preset-not-found。原失败保存在 examples-tool-error.json；工具改为 presets list 取 ID 后通过。README 明确这个返回契约。没有改产品或重跑业务全量。
- src、tests 与 Cargo 全文件前后逐字节 SHA 清单 cmp exit0，清单 SHA256 均为 be98a76f404699717f26b3183345adb001b67ce713c0a44d90ec0d2147994eb7。before/final 清单位于证据目录。没有 Git/jj 或暂存区检查，不声称 index 为空。
- README 与 verification 外部链接实际存在，Node 检查 missing=[]。相对入口人工核对。写工具内置 Markdown 检查全部 clean，独立 markdownlint 命令未发现，不安装软件。没有把 JS 分析工具 unavailable warning 当成 clean 证明。
- 没有重新 build、Cargo test、TUI 行为运行或 Agent loader 调研。Cargo build/run 为真实项目开发入口，TUI 默认启动读取 CLI 入口并采用 E2 fixed PTY 行为证据，不把本轮 help 当行为验证。未复制 E2 summary，现有紧凑 summary 仅包含 label 与 exit，无需再增加仓库文件。

## Runtime 与残留

当前 child session 为 731b22fb-81a9-44df-9162-f2b133b06ecb，run-0。首 assistant metadata 时间 2026-10-11T19:46:05.309Z，provider=cliproxy、model=gpt-6.1-sol、api=openai-completions、thinkingLevel=medium，保存为证据目录 runtime-evidence.json。仅证明 Pi 声明，上游真实 model/effort 未知。

E2 原有限制全部保留。跨 state-dir 共 Target、不合作 writer 最终微窗口、同 UID 恶意 receipt 篡改、删除 crash、多 route partial mutation、跨平台与掉电未全覆盖。shared entry 保留不证明 Agent 停止加载，TUI pending 恢复需先确认合法 CLI 业务意图。临时证据可能清理。用户提交、推送、发布禁令仍有效。

## Deviations

无产品偏离。只修正文档核验工具的 create 返回值误读。完成即停。

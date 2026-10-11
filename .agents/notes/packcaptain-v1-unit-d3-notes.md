# D3 长列表 PTY 稳定性

本单元只修测试观察方式。没有产品滚动缺陷证据，不改生产。E 为 `/tmp/packcaptain-d3-P3k2iV`。

## 清单与 checkpoint

1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables). 修前真实 PTY 固定3轮均绿，保存 red-1 到 red-3，不当作修复。
2. Binary-search the cause. 重放三份已有真实失败 transcript，旧观察为 RED，终端屏幕为 GREEN。
3. Plan the fix. how/architect 串行比较历史流文本、完整通用终端模拟、严格当前 Crossterm 输出子集。选择第三种，不增加 Cargo 依赖。
4. Verify on the same surface. 修后 focused 10轮、两个PTY suites 3轮、all-targets 1轮完成，所有 exit0。
5. Stage the commits so the failing repro lands before the fix in git history. skip，禁止提交。
6. Run Opening a PR. skip，禁止发布。

吞吐 checkpoint 为备份/RED、根因/实现、固定数量验收。禁止 fanout 和动态 workflow，how/architect/no-comments 串行执行。先新建 E 并逐字节备份 tui 与测试，src/tests/Cargo SHA 保存。未长期压力重跑。

## 根因证据

Ratatui 是光标定位后的增量单元格更新，不保证完整 Skill 名称出现在原始流或删除 ANSI 后的文本中。搜索 Skill089 时复用 Skill 前缀，输出局部数字。旧 session.text 把历史片段串接，误判当前屏幕。旧 resize 只在尺寸确实改变且异步事件已处理时带来清屏重绘，不是可靠观察协议。

C1、C2b-forward、C2b-rollback 的 all-targets 失败日志分别复制到 historical-c1.log、historical-forward.log、historical-rollback.log。均原 exit101、旧line483 Skill089断言失败。其mtime与默认证据目录中对应long-list.bin时间精确匹配。

- C1 对应 `/var/folders/d9/_0gbv97x6332wsrqhr4fcyj80000gn/T/packcaptain-extended-pty-51620/long-list.bin`。
- forward 对应同父目录 `packcaptain-extended-pty-45205/long-list.bin`。
- rollback 对应同父目录 `packcaptain-extended-pty-75163/long-list.bin`。

三份已复制为 E/historical-*.bin。E/legacy-replay-red.log 与 .exit 原样执行旧解码算法及完整 Skill089 判断，exit1。E/replay.rs、replay.log、replay.exit 使用新模型重放这三份，exit0，每份205个完成draw，最终选中行均为 `> [ ] Skill089 [可用]`。历史重放用20x121最大画布容纳原resize，最终布局是18x120，未冒称有原逐帧时间记录。

E/ansi-comparison.log 证明三个失败流和本次 focused-1 原始流均没有完整 Skill089，但实际屏幕含它。三个历史流与本次旧测试绿流都出现3次clear-screen，因此不能把故障归于resize没到达。当前3次旧测试绿与多轮历史RED共同符合同一观察缺陷。搜索片段依赖随机 skill ID 排序与已有单元格，造成断言间歇。没有证据表明输入数量不足或生产滚动失败。

## 数据形状与实现

`tests/support/pty_screen.rs` 以二维字符单元格、光标位置和完成draw计数表示终端。处理真实观察到的CUP、ED2、SGR、alternate-screen和Hide/Show。宽字符宽度复用已有ratatui Span，不新增依赖。未知CSI、越界和未支持控制字符直接失败，不静默忽略。它是严格当前测试输出子集，不是通用VT100模拟器。

Crossterm在绘制末尾输出Hide，replay只消费最后一个完整Hide以前的字节，避免读线程拆开CSI、UTF-8或draw造成瞬态误判。新增回归证明同样没有连续Skill089的增量流能恢复完整选中行、未完成draw不可观察、清屏后历史项目不残留、未建模CSI拒绝。

长列表保持90 skills、18x120 PTY、95次down与95次up、搜索Skill089、Esc返回及q exit0。排序仍按真实skill ID。每个移动都等待新的完成draw和预期选中行，实际证明末项与回顶。搜索逐字符等待可见过滤条件，确认后断言当前选中Skill089。删除resize强制重绘，不增加固定sleep。现有启动500ms与finish的q 100ms没有延长。保留所有业务断言，末项和Skill089检查迁移到实际viewport，增加回顶断言。

Session记录输入字节、elapsed_ms、完成draw数、原始字节数、18x120 viewport和q状态。正常及Drop失败均保留 `.bin`、`.txt`、`.viewport.log`。其他测试仍沿用旧历史文本API，不扩大本修复范围。

Model the Domain 决定显式屏幕状态。Fix Root Causes 决定对照已有真实失败而不是加sleep。Prove It Works 决定同一真实PTY的固定次数验收及binary身份检查。unslop与technical-writing用于记录。no-comments串行审阅，仅保留说明外部Crossterm draw结束协议的一条注释，无suppression、restore或业务删除。

## 验收与复跑

E/env.sh 隔离HOME/XDG，CARGO_HOME与RUSTUP_HOME仅复用Rust缓存。每个extended fixture再隔离HOME、agent-home、state与cwd。tui_pty原fixture有独立agent-home/state，进程HOME继承E/home。没有读取auth、写真实agent配置或删除skills。

| 命令 | exit | 结果 | E下证据 |
| --- | --- | --- | --- |
| 旧算法重放已有失败transcript | 1 | 原Skill089判断RED | legacy-replay-red.log/.exit |
| 新模型重放3份同一transcript | 0 | 各205完成draw，选中Skill089 | replay.log/.exit与historical-*.viewport.txt |
| 修前focused exact 3轮 | 全0 | 每轮1 passed，保留间歇背景 | red-1到red-3/log、exit、pty |
| 修后focused exact 10轮 | 全0 | 每轮一次执行1 passed、0 ignored | focused-1到focused-10/log、exit、pty |
| 两个PTY suites 3轮 | 全0 | 每轮12 passed、0 ignored，原D2重确认/cancel与原tui_pty保持 | suites-1到suites-3/log、exit、pty及packcaptain-frontend-pty-*副本 |
| cargo test --all-targets -- --nocapture | 0 | 83 passed、0 failed、0 ignored | all-targets/log、exit、pty及原tui_pty transcript副本 |

focused入口为 `. E/env.sh; PACKCAPTAIN_PTY_EVIDENCE=<新目录> cargo test --test tui_extended_pty pty_long_list_scrolls_to_last_skill_and_back_without_crash -- --exact --nocapture`。suites入口为相同环境下 `cargo test --test tui_extended_pty --test tui_pty -- --nocapture`。没有失败过滤、skip、ignore、盲重试或先失败后挑绿。green-1为实现初次校验，另有独立10轮最终focused。全部完整日志保留。LSP两个文件0诊断、2 inconclusive，非confirmed clean；实际Cargo编译与测试是证据。

生产binary前后 SHA256 均 `f81ff905e9a6bb51aacd3d0516a9b30c6672beba78aa3aff726f2f12bbaa1017`，binary-before.sha256和binary-final.sha256 cmp相等。仅测试观察变更，生产身份未变。source-before/source-final/boundary.log与tui_extended_pty.diff已审阅，src全相同，tui字节cmp相同，Cargo cmp相同，其他tests不变。唯一源测试变化为extended与新support文件，另写本notes。定向hook只格式化允许测试文件，未观察越界，没有全仓fmt或clippy fix。未执行git/jj、提交、发布或暂存；没有核验当前index是否为空。

## Runtime 与残留风险

E/runtime-evidence.log从当前run `69f28e4f-b3a2-4307-8020-f8d96650b6e7` child transcript首条assistant提取provider=cliproxy、model=gpt-6.1-sol、api=openai-completions、thinkingLevel=high。仅证明Pi响应metadata，上游真实model/effort未核实。

只实测本机macOS portable-pty、TERM=xterm-256color。长列表新测试只测18x120，原tui_pty另测30x120。其他尺寸、Windows/Linux、其他终端后端、更多skills和combining characters未测。strict ANSI模型若Crossterm改变输出或字符集扩展会显式失败，需要扩展相应行为测试；不声称完整终端兼容性。其他PTY测试仍有固定sleep及历史文本断言，本单元不处理。临时证据可能被系统清理。仅D3修复，不宣称整版完成。

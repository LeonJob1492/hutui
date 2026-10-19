# hutui 项目规范

## 项目定位

在终端里浏览知乎网页版的 TUI 工具，界面参照 Codex CLI，运行于本机 Ghostty（1.3.1）。

**定位优先级：摸鱼伪装优先** —— 窗口看起来要像个正在跑 codex 的终端，其次才是阅读体验。这条优先级决定后面所有命名与交互取舍。

## 命名口径（2026-10-15 定案）

| 层 | 取值 | 说明 |
|---|---|---|
| 仓库名 | `hutui` | 对齐本地短单词风格（`passport`、`token-lens`） |
| 二进制名 | `hutui` + `ks` 别名 | 两者本机 PATH 均未占用 |
| 包名（备用） | `hutui` | npm / crates.io / PyPI 三注册表均可用 |
| 配置命名空间 | `hutui:` | 本地键前缀，与 `ngax:` / `v2cx:` 血统一致 |
| Ghostty 标签标题 | `handler.rs — gateway` | 沿用 nga-codex 的 `<源码文件名> — <项目名>` 模式 |
| 侧栏品牌名 | `Codex` | 同上 |
| panic 快捷键 | 连按两下 `Esc` + `Ctrl+Shift+H` | 沿用已形成肌肉记忆的方案 |

`hutui` = 知乎的「乎」+ TUI。选它的核心理由：**GitHub 零命中、无商标联想、进程名不泄露「知乎」**（`ps` 里藏不住，这是当初排除 `zhihu-reader` 的原因）。

已排除的名字：`zhihu-tui`（GitHub 两个同名仓库）、`zhihu-cli`（npm + crates 已占用，且知乎官方 Skill 的二进制就叫这个）、`saltui`、`zhiliao`、`zhi-cli`、`moyu`（均被占用）。

## 已核验的硬约束（不要重新假设）

1. **知乎游客走不通**：`curl` 返回 403（`code:40362`）；浏览器里导航首页会 302 到 `/signin?next=%2F`。必须登录态。
2. **这些接口实测不校验签名**：在知乎页面的同源上下文里 `fetch` 三个取数接口全部 200，只要带登录 cookie；
   请求头里根本没有 `x-zse-96`，`window.fetch` 也没被知乎 patch。
   旧结论「签名接口不能直连」只对游客/无 cookie 场景成立，**已证伪**（见 `docs/design.md` 2.4）。
   **仍然不做**：重写签名算法、伪造 UA 或客户端身份。
3. **边界（Leon 划定）**：登录态下给自己换皮属客户端行为，可以做；**重写签名算法或伪造客户端身份不做**。
4. **可落地架构**：独立 Chrome 实例只负责提供一个**带登录 cookie 的同源上下文**；
   取数是 `Runtime.evaluate` 里的一次 `fetch`。**不模拟点击、不模拟滚动、不解析 DOM。**
   数据链路细节见 `docs/design.md` 第 2 节，改动前先读它。

## 伪装优先带来的两条新约束（2026-10-15 新增，前序文档未覆盖）

1. **panic 模式必须整体重绘**：终端里没有 DOM 可叠。油猴脚本可以 `display:none` 藏原生再叠一层假 UI，但 Ghostty 的「视口」就是 TUI 画出的字符网格，所以 panic 必须让 TUI **全屏重绘**成「代码编辑器 + 正在跑 `cargo test` 的终端」的字符画面。切换**不能有异步半帧**，否则会闪出知乎正文，伪装当场失败。
2. **自动伪装已砍**（2026-10-15 决定）：原 1004 焦点上报 / `NSWorkspace` 前端 App 检测两条路都不做，只保留手动触发。理由：省掉一个不确定源，而「屏幕自己闪一下」本身就是暴露点。手动 panic = `Esc` 连按两下，或 `Ctrl-H`；**进入应急画面后只有这两个组合能退出**，其他键不响应。

## 窗口标题：已验证可直接控制（利好）

`/Users/leon/Library/Application Support/com.mitchellh.ghostty/config.ghostty` 已设
`shell-integration-features = cursor,no-sudo,no-title,no-ssh-env,no-ssh-terminfo,path`，
Ghostty 自带 shell 集成**不写标题**；`/Users/leon/.zshrc` 的 `_leon_window_title_precmd` 写 `${PWD:t}`，
但 **TUI 前台运行时 precmd 不触发**。

→ 结论：**TUI 用 OSC 2 写伪装标题不会被覆盖**，退出后自动恢复成目录名。链路细节见 memory 的「终端 / Ghostty 窗口标题」章节。

## 可复用的存量资产（别重复造）

| 资产 | 路径 | 用途 |
|---|---|---|
| 知乎官方 Skill v0.3.0 | `leon-claw/workspace/skills/zhihu/` | 搜索 / 热榜 / 直答 / 我的内容 |
| 热榜抓取脚本 | `leon-claw/workspace/zhihu-scraper-final.mjs` | 108 行，配 cookie |
| 登录 cookie | `leon-claw/workspace/zhihu-cookies.json`、`zhihu-cookie.txt` | 已有登录态 |
| 热榜历史快照 | `leon-claw/workspace/memory/zhihu-hot/*.json` | 每天 4 次，含 rank/title/url/缩略图/摘要 |
| 定时器 | `leon-claw/systemd/openclaw-zhihu-hot-*.{service,timer}` | 已在跑 |

**官方 Skill / 开放平台只有搜索、热榜、直答，没有正文接口**，本项目的差异化价值在「读网页版正文 + Codex 皮」。

## 已定案（2026-10-15）

| 决策 | 结论 |
|---|---|
| 技术选型 | **Rust + ratatui**。理由：panic 必须零半帧，ratatui 双缓冲 + 单次 flush 最可控；单二进制、毫秒启动、终端控制粒度最细 |
| 架构 | **浏览器桥 + 终端渲染**：独立 Chrome 实例（`~/.hutui/chrome-profile`）⇄ CDP ⇄ Rust TUI |
| 浏览器窗口 | 有窗口 + `open -n -g` 后台启动；**允许可见，但绝不抢焦点** |
| 正文渲染 | 渲染成「codex 助手的 markdown 输出」形态，**不做**行号编辑器皮肤 |
| 阅读模型 | **一条一屏**：只显示当前这条回答，↑↓ 直接切换该问题下的上/下一条；条内滚动用 PgUp/PgDn，每条各自记住滚动位置 |
| 界面宽度 | 整体限制 **1024px**（按 8px/字符折算 128 列），超出部分居中留白；常量在 `src/config.rs` 的 `MAX_LAYOUT_PX` |
| 主闭环 | 推荐流列表（↑↓ 选择）→ `→` 进阅读流 → `←` 回列表 |
| 自动伪装 | **不做**，只保留手动 panic |
| 取数方式 | **直接 `fetch` 接口**：页面同源上下文里一次 `fetch`，跟随 `paging.next` / 按 `offset` 翻页；不模拟页面操作 |
| 单篇 / 列表 | a = `/api/v4/answers/{aid}` 展示当前条，b = `/api/v4/questions/{qid}/answers` 准备 ↑↓ 切换（`paging.totals` 给总数） |
| 写操作 | 赞 / 收藏 / 评论均已实测（只需 `x-xsrftoken`，不需签名），接口见 `docs/design.md` 2.5；评论只读 |
| 键位 | 列表 ↑↓ → ←；阅读 ↑↓ 换回答、PgUp/PgDn 条内滚动、`z` 赞、`v` 反对、`x` 评论区、`c` 收藏、← 返回；评论区 `↵` 加载更多；`Esc`×2 或 `Ctrl-H` panic |

## 代码结构

```
src/main.rs          入口、事件循环
src/app.rs           状态机（List / Reading / Comments / Panic）+ 按键 + 表态状态
src/state.rs         已读记录（~/.hutui/read.json）
src/term.rs          终端生命周期、OSC 2 伪装标题、panic hook
src/text.rs          CJK 感知换行 / 截断 / 数字压缩 / 相对时间
src/ui/              list · reading · comments · panic · theme
src/zhihu/           领域模型 + 读/写接口的解析器
src/bridge/          chrome 生命周期 · cdp 客户端 · 工作线程
```

`probe/` 是十余轮 CDP 探针的现场脚本与原始报告，**已进 `.gitignore`**（报告里有登录昵称与回答内容）。
其中 `cdp-probe10/11` 是证伪「签名接口不可直连」的关键实验，排查数据问题时先看它们。

## 文档索引

- **README（对外介绍）**：`/Users/leon/Code/hutui/README.md`
- **项目设计文档（实现的唯一事实源）**：`/Users/leon/Code/hutui/docs/design.md`
- 本项目定名与本会话结论：`/Users/leon/Code/leon-claw/workspace/memory/2026-10-15-hutui-zhihu-terminal-codex-skin.md`
- 前序可行性评估（油猴脚本拆解 + 知乎移植分析）：`/Users/leon/Code/leon-claw/workspace/memory/2026-09-23-nga-codex-zhihu-disguise-feasibility.md`

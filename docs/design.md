# hutui 设计文档

> 状态：v0.1 骨架期　最后更新：2026-10-15
> 本文是实现的唯一事实源。与 `AGENTS.md` 冲突时以本文的数据链路结论为准。

## 1. 定位与边界

**摸鱼伪装优先**：窗口要看起来像正在跑 Codex CLI 的终端，其次才是阅读体验。任何取舍与此冲突时以伪装优先。

**边界（Leon 划定，不要越过）**：

- 登录态下给自己换皮属客户端行为，可以做。
- **不伪造签名、不伪造客户端身份、不自己构造 `/api/*` 请求。**
- 不绕付费墙：盐选正文本来就不在数据里，拿不到就是拿不到。
- 界面自带文案零「知乎」字样；正文内文不动。

## 2. 已验证的数据链路（2026-10-15 实测）

五轮 CDP 探针的结论，全部来自真实登录态浏览器，探针自身未发出任何 `/api` 请求。

### 2.1 前置事实

| 事实 | 结论 |
|---|---|
| 游客态 | 导航 `zhihu.com` 直接 302 → `/signin?next=%2F`，api 零命中。**必须登录态。** |
| 登录态载体 | 独立 profile（`~/.hutui/chrome-profile`）。cookie 写入后**冷启动持久有效**。 |
| 调试端口 | Chrome ≥136 起，默认 user-data-dir 会忽略 `--remote-debugging-port`；**必须独立 profile**。 |
| 窗口 | `open -n -g` 后台启动，不抢焦点；窗口被遮挡时滚动仍能触发懒加载（实测）。 |

### 2.2 三个数据源

**① 推荐流列表**

```
GET /api/v3/feed/topstory/recommend
    首屏  ?limit=10                            → 6 条
    翻页  ?action=down&after_id=N&session_token=... → 6 条/页
```

- 触发方式：导航到首页后**滚动页面**，页面自己发请求。
- 条目结构：`data[i].target` 就是 answer 实体。

```jsonc
{ "type": "feed",
  "target": { "type": "answer", "id": "2093...", "content": "<p>…</p>",
              "excerpt": "…", "voteup_count": 110, "comment_count": 12,
              "author": { "name": "…" },
              "question": { "id": "1945...", "title": "…", "answer_count": 217 } } }
```

实测 48 条：**48/48 带 `content`**（max 21333 / p50 1355 / min 84 字符），
`content_need_truncated=true` 为 0 条 → **列表页拿到的就是全文，没有截断。**
48/48 带 `question.id` → 不用解析 URL 就能拿到 qid。

**② 单篇正文（零请求）**

回答页导航完成后，`js-initialData.initialState.entities.answers[aid].content` 就是全文。
实测 `/api/v4/answers/{id}` **从未被调用**。问题实体同层可用。

**③ 同问题下的其他回答**

```
GET /api/v4/questions/{qid}/feeds?include=data[*].…content…&order=default&limit=N
```

- 条目 `type` 是 `question_feed_card`，**全文在 `data[i].target.content`**（是个完整 answer 实体）。
- 回答页首屏 3 条；要更多**必须点「查看全部 N 个回答」** → 再 3 条。
- **问题页 `/question/{qid}` 首屏 5 条，滚动自动翻页**，每次 +5 条（cursor 分页）。

→ **取同问题回答时用问题页，不用回答页。**

### 2.3 两个曾经踩过的坑

- 第一版探针把抓取过滤写成 `/api/v4/`，把 `/api/v3/feed/topstory/recommend` 整个漏掉，
  一度误判"首页是 SSR 内联"。**实现必须抓 `zhihu.com/api/**` 全部，不能只盯 v4。**
- `/api/v4/questions/{qid}/answers` **这个接口不存在**，别照抄。

## 3. 架构

```
┌─────────────────────────────┐
│  hutui (Rust + ratatui)     │  主线程：事件循环 + 渲染
│  ├ ui/     列表/阅读/panic   │
│  ├ app.rs  状态机 + 按键     │
│  └ bridge/ 后台线程          │
└──────────┬──────────────────┘
           │ mpsc: Cmd ↓ / Event ↑
┌──────────┴──────────────────┐
│  采集工作线程                │
│  ├ chrome.rs  实例生命周期    │
│  ├ cdp.rs     WS JSON-RPC    │
│  └ worker.rs  导航/滚动/捕获  │
└──────────┬──────────────────┘
           │ CDP over ws://127.0.0.1:9222
┌──────────┴──────────────────┐
│  独立 Chrome 实例            │
│  profile: ~/.hutui/chrome-  │
│  profile（登录态持久）        │
│  启动: open -n -g（不抢焦点） │
└─────────────────────────────┘
```

**为什么是这个形态**：终端内直连被签名挡住；而 CDP 的 `Network` 域能读到知乎**自己发出**的
响应体 —— 不碰 `x-zse-96`、不伪造身份、被动只读，正好落在边界内侧。

**线程模型**：主线程只管渲染与按键，任何网络等待都在工作线程；`Cmd`/`Event` 走 mpsc。
渲染永远是单帧 `terminal.draw()`，**不存在异步半帧**。

## 4. 数据流与交互

```
启动 ──► 工作线程: ensure Chrome → 建 target → Network.enable
                    → navigate 首页 → 捕获 /api/v3/feed/... → 滚动补页
                    → 增量 Event::Feed 回主线程

列表  ↑↓ 移动光标（j/k 同义）
      → / Enter ──► 打开阅读流
      r 重新拉取        q / Ctrl-C 退出

阅读  进入时：当前条已有全文 → 立刻渲染（零延迟、零请求）
      同时后台 navigate 到 /question/{qid} 滚动预取同问题其他回答，
      按 answer id 去重后追加到流尾

      ↑↓ 连续滚动（滚过某条边界自然进入下一条）
      n / p  跳上/下一条回答
      PgUp/PgDn · Home/End
      ← / Backspace 返回列表

panic Esc 连按两下 或 Ctrl-H ──► 全屏重绘成静止的 cargo test 画面
      任意键（或再按 Esc）恢复；恢复目标为进入 panic 前的视图
```

**为什么阅读用连续流**：知乎的回答长短差极大（84 ~ 21333 字符），"一条一屏"会让长文没法读；
而连续流与 codex 输出"一长条滚动文本"的形态天然一致，伪装和可读性同时满足。

**预取策略**：列表首次目标 42 条（≈7 页）；阅读流首次 5 条，滚到剩 2 条时再取下一页，硬上限 50。

## 5. 渲染规范

```
header  ▌ Codex  ~/hutui  main*  ·  rust 1.98
列表    ▸ 现在大环境很难很难找工作吗？
          重水轮回立 · 110 赞 · 摘要前若干字…
阅读    ── 3/12 · 问题标题 · 作者 · 110 赞 ─────────────────
        正文段落（2 空格缩进）
        （空行）
footer  {n} items · {状态}
```

- **零「知乎」字样**；路径用真实的 `~/hutui`（真实的东西最难被看穿）。
- 颜色：正文 `Rgb(220,220,225)`，次要 `Rgb(120,120,128)`，强调 `Rgb(120,200,180)`。
- **panic 画面必须静止**：会滚动的画面反而吸引注意，静止的终端是自然的（你在读它）。
  且零动效 = 零半帧风险，与硬约束契合。

## 6. 风险与应对

| 风险 | 应对 |
|---|---|
| 高频自动滚动触发风控 | 页间 1.5~4s 随机间隔；被拦时如实提示，不假装成功 |
| Chrome 切到别的 Space 被节流（**未测**） | 用户可切回；设计上不依赖后台渲染 |
| 知乎改版破坏解析 | `parse.rs` 里「接口/字段 → 解析器」保持扁平，改版只改解析层 |
| 同篇回答在多个问题下重复 | 按 answer id 去重 |
| 单问题 200+ 回答 | 阅读流硬上限 50，超出提示 |

## 7. 不做什么

- 不做自动伪装（原 1004 焦点上报 / NSWorkspace 方案）—— 已砍，只保留手动 panic。
- 不做 headless —— 已定有窗口 + `open -g`。
- 不做写操作（赞同/收藏/评论）—— 首版只读。
- 不做登录流程 —— 由 `~/.hutui/chrome-profile` 承载，首次手动登录一次即可。

## 8. 待办

- [ ] panic 画面做成可配（当前写死 cargo test）
- [ ] 文章 / 想法(pin) 类型的阅读分支
- [ ] 登录失效时的探测与提示
- [ ] 预取节流与「已读」去重（知乎服务端会过滤已读，`finished_reading_filtered_count`）

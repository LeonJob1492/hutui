# hutui 设计文档

> 状态：v0.2（数据链路修正版）　最后更新：2026-10-15
> 本文是实现的唯一事实源。与 `AGENTS.md` 冲突时以本文的数据链路结论为准。

## 1. 定位与边界

**摸鱼伪装优先**：窗口要看起来像正在跑 Codex CLI 的终端，其次才是阅读体验。

**边界（Leon 划定）**：

- 登录态下给自己换皮属客户端行为，可以做。
- **不重写 `x-zse-96` 签名算法、不伪造 UA / 客户端身份。**
- 不绕付费墙：盐选正文本来就不在数据里。
- 界面自带文案零「知乎」字样；正文内文不动。

> 边界里原本写的是「不自己构造 `/api/*` 请求」。**这条基于错误前提，已作废**，
> 详见 2.4。现在的口径是：可以在页面同源上下文里直接 `fetch` 公开接口，
> 因为实测这些接口不校验签名；但仍然不重写签名算法、不伪装客户端。

## 2. 数据链路（全部实测）

### 2.1 前置

| 事实 | 结论 |
|---|---|
| 游客 | 导航 `zhihu.com` 302 → `/signin`，`curl` 403（`code:40362`）。必须登录态。 |
| 登录载体 | 独立 profile `~/.hutui/chrome-profile`，cookie 写入后冷启动持久有效。 |
| 调试端口 | Chrome ≥136 起默认 user-data-dir 会忽略 `--remote-debugging-port`，必须独立 profile。 |
| 窗口 | `open -n -g` 后台启动，不抢焦点。 |

### 2.2 三个接口

所有请求都在**知乎页面上下文里**用 `fetch(url, {credentials:'include'})` 发出，
Rust 侧通过 CDP `Runtime.evaluate` 取值。**不模拟点击、不模拟滚动、不解析 DOM。**

**① 推荐流列表**

```
GET /api/v3/feed/topstory/recommend?limit=10&desktop=true
    → 6 条（服务端固定上限，limit 调大无效）
    → paging.next 是完整 URL（带 after_id + session_token），直接跟着翻
```

实测：跟 7 页拿到 42 条，零重叠。条目 `data[i].target` 是 answer 实体，
自带 `content`（全文）、`excerpt`、`author.name`、`question.id/title`、`voteup_count`。

**② 单篇回答（a：用来展示）**

```
GET /api/v4/answers/{aid}?include=content,excerpt,author,voteup_count,comment_count,question
```

根对象就是 answer 实体，一次拿全正文 + 作者 + 所属 `question.id` / `question.title`。
实测 `content` 362 字符，`question.id` 与预期一致。

**③ 该问题下的回答列表（b：用来准备 ↑↓ 切换）**

```
GET /api/v4/questions/{qid}/answers?include=data[*].content,…&offset=N&limit=20&order=default&platform=desktop
```

- **认 `offset`**，`paging.totals` 直接给出总数，`paging.is_end` 标示到底。
- 实测 `limit=20` 一次返回全部 15 条，每条都带全文 `content`。
- `data[i]` **直接就是 answer 实体**（没有 `target` 包装）。

### 2.3 不要用 `questions/{qid}/feeds`

这个接口是 cursor 分页：**忽略 `offset`**，同样的参数永远返回同一批 5 条；
页面上要拿到更多只能靠滚动触发，且经常出现 `data=[] + is_end=true` 的假到底。
早期实现就是被它拖住的，别再回去。取回答列表一律用 ②③ 里的 `answers`。

### 2.4 一条被证伪的旧结论（重要，别再重复）

9-23 的评估里写着「正文走签名接口，带 `x-zse-96`，不能手造请求」。
那句话**只对游客 / 无 cookie 场景成立**，而我把它当成了不可验证的前提，结果是：

- 花了五轮探针去「模拟页面操作」（导航 + 滚动 + 被动捕获响应体）
- 采集依赖页面自己的懒加载节奏，慢了、漏了、`is_end` 还不可靠
- 线上表现为「15 个回答只切到 1 条」

实测澄清（probe10/11）：

```text
window.fetch 是原生函数（/[native code]/），知乎没有 patch 它
我们在页面里 fetch 时，请求头里【没有】x-zse-96
上述三个接口全部 200，只要带登录 cookie
```

**实验方法备忘**：怀疑「某接口不可直接调用」时，先在页面上下文里裸 fetch 一次看状态码，
不要从历史结论往下推。签名是不是必需，一条 `fetch` 就能证伪。

### 2.5 写接口（赞 / 收藏 / 评论）

只多一个头：`x-xsrftoken`（值就是页面里的 `_xsrf` cookie），**仍然不需要签名**。
实测全部成功，每次探针都还原了现场（赞同数 80→81→80，收藏夹 6420→6421→6420）。

| 动作 | 请求 |
|---|---|
| 赞 / 反对 / 取消 | `POST /api/v4/answers/{aid}/voters`，body `{"type":"up"\|"down"\|"neutral"}` |
| 收藏 | `POST /api/v4/collections/{cid}/contents?content_id={aid}&content_type=answer` |
| 取消收藏 | `DELETE /api/v4/collections/contents/answer/{aid}` |
| 默认收藏夹 | `GET /api/v4/me` → `GET /api/v4/people/{url_token}/collections`，取 `is_default` |
| 评论列表 | `GET /api/v4/answers/{aid}/comments?limit=20&offset=0&order=normal` |

**两个坑**：

- 收藏的 add 端点走 JSON body 会返回 `400 missing content_id or content_type`，**参数必须放 query，body 必须为空**。
- 评论作者不在 `author.name`，在 **`author.member.name`**。

表态结果以响应里的 `reaction_value`（`up` / `down` / 空串）和 `reaction_count` 为准，
不要自己猜——服务端是幂等的，重复提交同向不会叠加。

**初态**：`relationship.voting` 三处都有（推荐流条目自带、单篇与列表加个 `include` 就有），
取值 `1` / `-1` / `0`。所以 `z` 第一次按就是正确的方向，不需要盲猜。
**收藏没有对应的初态字段**，本地默认「未收藏」——若这条本来已在收藏夹里，
第一次按 `c` 是一次幂等的重复添加，第二次按才是取消。

## 3. 架构

```
┌──────────────────────────────┐
│  hutui (Rust + ratatui)      │
│  ├ ui/     列表/阅读/panic    │
│  ├ app.rs  状态机 + 按键      │
│  └ bridge/ 后台线程           │
└───────────┬──────────────────┘
            │ mpsc: Cmd ↓ / Event ↑
┌───────────┴──────────────────┐
│  工作线程                     │
│  ├ chrome.rs  实例生命周期     │
│  ├ cdp.rs     WS + fetch_json │
│  └ worker.rs  三个接口的取数   │
└───────────┬──────────────────┘
            │ CDP over ws://127.0.0.1:9222
┌───────────┴──────────────────┐
│  独立 Chrome 实例             │
│  一个常驻知乎页（同源上下文）  │
│  profile: ~/.hutui/chrome-   │
│  profile（登录态持久）         │
└──────────────────────────────┘
```

**为什么还要浏览器**：不是为了模拟操作，而是为了拿到一个**带登录 cookie 的同源上下文**。
所有取数都是一次 `fetch`，没有页面时序依赖。

**线程模型**：主线程只管渲染与按键；`Cmd`/`Event` 走 mpsc；渲染永远单帧 `terminal.draw()`。

## 4. 数据流与交互

```
启动 ──► 工作线程: ensure Chrome → 建知乎页 → 轮询 location.origin 直到是 zhihu.com
                    → 跟 paging.next 取推荐流（首屏 6 条，最多 12 页）

列表  ↑↓ / j k 选择      → / Enter 打开      r 重拉      q / Ctrl-C 退出

打开  → 立刻用手里的正文秒开（零延迟）
       ├─ a: fetch /api/v4/answers/{aid}      → 用权威正文覆盖当前条
       └─ b: fetch /questions/{qid}/answers   → 按接口顺序重排整个列表，
                                                 并把当前这条定位到它自己的位置

阅读  一条一屏。↑↓ / j k / n p 切换该问题下的上/下一条
      PgUp/PgDn · Home/End  在当前这条内部滚动（每条按 id 记住位置）
      z 赞 / 取消          c 收藏 / 取消        x 评论区
      ← / Backspace 返回列表

评论  独立视图（只读）。↑↓ / PgUp/PgDn 滚动      ↵ 加载更多
      x / ← / Backspace 回阅读

panic Esc 连按两下 或 Ctrl-H ──► 全屏重绘成静止的 cargo test 画面
      任意键（或再按 Esc）恢复
```

**为什么一条一屏**：↑↓ 直接对应「换一个回答看」，比"滚过边界"更符合直觉。代价是长回答要
PgUp/PgDn，所以按回答 id 记住各自的滚动位置，切回来不丢。

## 5. 渲染规范

```
整体    视口收窄到最大 1024px（128 列）并水平居中，超宽终端两侧留白
header  ▌ Codex  ~/hutui  main*  ·  rustc 1.98              3/42
列表    ▸ 现在大环境很难很难找工作吗？          ← 未读，光标所在
          重水轮回立 · 110 赞 · 摘要前若干字…
        （空行）
        · 已经看过的问题标题                    ← 已读，整条降到 FAINT
          作者 · 62 赞 · 摘要…
        （空行）
阅读    ── 3/15 · 作者 · 110 赞 ─────────────────
        问题标题
        正文段落（2 空格缩进）
footer  {状态}                                            ↑12 ↓8
```

- **已读置灰**：打开过就记一条 `~/.hutui/read.json`（上限 800 条，超出丢最早），
  列表里整条降到 `FAINT`，行首标记由 `▸` 变成 `·`。重启后仍然认。
- **三级层次**：标题（正文色）→ 作者与数据（DIM）→ 摘要（FAINT），靠颜色分层而不是加符号。
- **行间留白**：每条 2 行内容 + 1 行空行，靠气口而不是分隔线。
- **右侧信息栏**：header 显示位置，footer 显示列表上下还剩多少条 / 当前条内滚动进度。
- **零「知乎」字样**；路径用真实的 `~/hutui`。
- 颜色：正文 `Rgb(220,220,225)`，次要 `Rgb(120,120,128)`，已读与摘要 `Rgb(78,78,88)`，
  强调 `Rgb(120,200,180)`。
- **panic 画面必须静止**：会滚动的画面反而吸引注意；零动效 = 零半帧风险。

## 6. 风险与应对

| 风险 | 应对 |
|---|---|
| 接口改版 | `parse.rs` 里三个解析器保持扁平，改版只改解析层 |
| 接口突然开始校验签名 | `fetch_json` 会把非 200 连同响应头 160 字符报出来，出错即可见 |
| 登录失效 | 页面会 302 到 `/signin`，`wait_origin` 25s 超时报错而非静默空列表 |
| 单问题回答过多 | 列表接口按 `limit=20` 翻页，`STREAM_MAX=50` 硬上限 |
| 同篇回答在多个问题下重复 | 按 answer id 去重 |

## 7. 不做什么

- 不模拟页面点击 / 滚动，不解析 DOM
- 不重写签名算法、不伪造 UA
- 不做 headless（有窗口 + `open -g`）
- 不发评论、不回复（评论区只读）
- 不做自动伪装，只保留手动 panic

## 8. 待办

- [ ] panic 画面做成可配
- [ ] 文章 / 想法(pin) 类型的阅读分支
- [ ] 推荐流「已读」去重（服务端会过滤，`finished_reading_filtered_count`）
- [ ] `content_need_truncated` 的兜底提示

# 桥插件

`dsh-desktop-live-bridge` —— 把内核的 assistant 流重新发布给桌面客户端的**内核侧插件**。

## 为什么需要它

SDK stdio 协议只转发 durable 的 `session/event`，所以一条回答要等整轮结束才整段到达，
中间没有任何增量。内核**有**实时流（全局 Cordis 事件 `agent/assistant-stream`），
但没有东西把它转发给 SDK 客户端。

这个插件就是那个转发器。它是 profile patch 挂载的**插件**，不是对内核源码的修改 ——
钉住版本的内核保持原样、照常可升级。

## 为什么走 stderr

stdout 属于 JSON-RPC 传输。再开一个写入者去写 stdout，两个写入方可能把半帧交错进去。
stderr 本来就是客户端单独读的管道，加上标记前缀后零成本，也不会污染协议数据。

## 线上格式

每条一行：

```
\u0001DSH-LIVE <json>
```

`<json>` 是三者之一：

```json
{ "kind": "start", "sessionId": "...", "attemptId": "...", "turn": 1, "step": 1 }
{ "kind": "chunk", "sessionId": "...", "attemptId": "...", "index": 3, "chunk": { "type": "text-delta", "index": 1, "text": "你" } }
{ "kind": "end",   "sessionId": "...", "attemptId": "...", "outcome": { } }
```

`chunk` 是内核 LLM 层的 `StreamChunk`：`block-start` / `text-delta` / `reasoning-delta` /
`tool-call-delta` / `block-end` / `usage` / `finish`。

## 装在哪

Node 从 `$DSH_HOME/profiles/sdk/` 向上解析插件名，所以必须装在
**`$DSH_HOME/node_modules/dsh-desktop-live-bridge`** —— 不是 `kernel/node_modules`，
也不能把机器路径写进提交的 profile patch。

- 桌面应用启动时自己拷（`AppState::install_bridge`）
- 脚本运行：`pnpm bridge:install`（默认装到 `./.dev-home`，可用 `--home` 或 `$DSH_HOME` 覆盖）

## 已验证

```
cargo test -p dsh-runtime --test kernel_smoke live_bridge   # 15.43s，收到真实 text-delta
node scripts/capture-events.mjs                             # 10 条 live 帧，含流式正文
```

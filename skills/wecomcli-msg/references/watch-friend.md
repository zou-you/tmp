# +watch_friend helper

轮询指定好友新消息，输出 NDJSON；图片和文件会保存到本地。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--to` | string[] | ✅ | 好友名称、备注名或 `chat_id`，可重复传入多个目标 |
| `--interval-sec` | integer | ❌ | 轮询间隔秒数，默认 `5` |
| `--save-dir` | path | ❌ | 图片和文件保存目录，默认使用 wecom-cli 媒体临时目录 |
| `--once` | boolean | ❌ | 只轮询一次后退出 |
| `--max-events` | integer | ❌ | 输出达到指定条数后退出，`0` 表示不限制 |
| `--idle-timeout-sec` | integer | ❌ | 全局无新消息达到指定秒数后退出，`0` 表示不限制 |

## 示例

```bash
wecom-cli msg +watch_friend --to "张三-客户" --interval-sec 5 --save-dir /tmp/wecom/media
wecom-cli msg +watch_friend --to "张三-客户" --once --max-events 10
wecom-cli msg +watch_friend --to "张三-客户" --to "李四-客户" --interval-sec 5
```

## 输出

每条新消息输出一行 JSON：

```json
{"chat_id":"zhangsan","target":"张三-客户","chat_name":"张三","send_time":"2026-03-17 09:35:00","userid":"lisi","msgtype":"image","media_id":"MEDIAID_xxxxxx","name":"screenshot.png","local_path":"/tmp/wecom/media/screenshot.png","size":102400,"content_type":"image/png"}
```

文本消息包含 `text`；图片和文件包含 `local_path`；语音和视频默认不保存，只输出 `media_id` 和元信息。多目标监控时可通过 `target`、`chat_id`、`chat_name` 区分来源。

## 限制

- 依赖 `get_msg_chat_list` 和 `get_message`，只能读取最近 7 天消息。
- 首次运行会把最近 7 天窗口内未记录过的消息视为新消息；后续通过状态文件去重。
- 如果 `--to` 匹配多个会话，命令会失败并返回候选列表，不会自动选择。
- 多目标模式下每个目标独立并行轮询，状态文件按 `chat_id` 独立保存。
- 多目标模式禁用 macOS 桌面消息 fallback，避免多个任务同时控制企业微信窗口。

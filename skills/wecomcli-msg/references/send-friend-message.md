# +send_friend_message helper

通过 macOS 企业微信客户端给指定好友发送文本、图片或文件。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--to` | string | ✅ | 好友名称或备注名 |
| `--text` | string | 三选一 | 文本消息，最大 2048 字节 |
| `--image` | path | 三选一 | 本地图片路径，扩展名需可识别为 `image/*` |
| `--file` | path | 三选一 | 本地文件路径 |

## 示例

```bash
wecom-cli msg +send_friend_message --to "张三-客户" --text "你好"
wecom-cli msg +send_friend_message --to "张三-客户" --image /path/to/a.png
wecom-cli msg +send_friend_message --to "张三-客户" --file /path/to/report.pdf
```

## 返回

```json
{
  "to": "张三-客户",
  "message_type": "file",
  "file_path": "/path/to/report.pdf",
  "status": "sent",
  "detail": "已触发企业微信桌面发送流程"
}
```

`status` 可能为 `sent` 或 `failed`。如果返回 `failed`，必须展示 `detail`。

## 限制

- 仅支持 macOS 企业微信客户端，默认 bundle id 为 `com.tencent.WeWorkMac`。
- 需要企业微信已登录，终端已授权“辅助功能”和“自动化”。
- helper 会尽量检测同名多候选并失败，但桌面 UI 自动化受客户端版本和界面语言影响，重要消息建议人工核对发送对象。

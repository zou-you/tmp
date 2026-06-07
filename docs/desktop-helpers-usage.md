# macOS 桌面 helper 操作说明

本文记录 `+add_external_friend`、`+send_friend_message`、`+watch_friend` 的安装、编译、配置和使用方式。

## 1. 版本说明

如果这些 helper 还没有发布到 npm，直接执行：

```bash
npm install -g @wecom/cli
```

安装到的是 npm 上的正式版，不一定包含本地源码里的新命令。要立即使用本次新增命令，需要从源码编译开发版。

## 2. 从源码编译开发版

前置要求：

- macOS
- Git
- Rust stable 工具链，包含 `cargo`
- 企业微信客户端已安装并登录

安装 Rust：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

拉取源码：

```bash
git clone https://github.com/WecomTeam/wecom-cli.git
cd wecom-cli
```

如果你已经在本地源码目录里，直接进入该目录：

```bash
cd /Users/zy/Workspace/tmp/wecom-cli
```

编译：

```bash
cargo build --release
```

编译后的二进制在：

```bash
./target/release/wecom-cli
```

验证：

```bash
./target/release/wecom-cli --version
```

## 3. 安装为全局命令

开发版推荐用 `cargo install` 覆盖当前用户 PATH 中的 `wecom-cli`：

```bash
cargo install --path . --locked --force
```

确认命令位置：

```bash
which wecom-cli
wecom-cli --version
```

如果 `which wecom-cli` 不是 `~/.cargo/bin/wecom-cli`，把 Rust bin 目录加入 PATH：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

长期生效可写入你的 shell 配置文件，例如 `~/.zshrc`。

不想全局安装时，也可以始终使用源码目录里的二进制：

```bash
/Users/zy/Workspace/tmp/wecom-cli/target/release/wecom-cli --version
```

## 4. 初始化 wecom-cli

首次使用远程 MCP 能力前，需要初始化凭证：

```bash
wecom-cli init
```

如果你没有安装为全局命令，用开发版二进制执行：

```bash
./target/release/wecom-cli init
```

配置默认写入 `~/.config/wecom`。同一台机器上，全局命令和开发版二进制会复用这份配置。

## 5. macOS 权限配置

`+add_external_friend` 和 `+send_friend_message` 通过企业微信桌面客户端自动化完成，需要系统权限。

打开：

```text
系统设置 -> 隐私与安全性 -> 辅助功能
```

把你运行命令的终端加入并启用，例如：

- Terminal
- iTerm
- Cursor
- VS Code

再打开：

```text
系统设置 -> 隐私与安全性 -> 自动化
```

允许该终端控制：

- 企业微信
- System Events

企业微信客户端需要提前登录。默认 bundle id 为 `com.tencent.WeWorkMac`。

## 6. 添加外部联系人

```bash
wecom-cli contact +add_external_friend \
  --phone "13800000000" \
  --remark "张三-客户" \
  --greeting "你好，我是..."
```

返回示例：

```json
{
  "phone": "13800000000",
  "remark": "张三-客户",
  "greeting": "你好，我是...",
  "status": "pending",
  "detail": "已提交或打开添加验证流程；对方确认前不会视为已添加"
}
```

`status` 说明：

| status | 含义 |
| --- | --- |
| `added` | 已添加成功 |
| `pending` | 已提交或打开验证流程，等待对方确认 |
| `already_exists` | 联系人已存在 |
| `failed` | 执行失败，查看 `detail` |

注意：`pending` 不能当作已经添加成功。

## 7. 给好友发消息

发文本：

```bash
wecom-cli msg +send_friend_message \
  --to "张三-客户" \
  --text "你好"
```

发图片：

```bash
wecom-cli msg +send_friend_message \
  --to "张三-客户" \
  --image /path/to/a.png
```

发文件：

```bash
wecom-cli msg +send_friend_message \
  --to "张三-客户" \
  --file /path/to/report.pdf
```

返回示例：

```json
{
  "to": "张三-客户",
  "message_type": "file",
  "file_path": "/path/to/report.pdf",
  "status": "sent",
  "detail": "已触发企业微信桌面发送流程"
}
```

`--text`、`--image`、`--file` 三选一。图片路径必须是本地可读的图片文件。

## 8. 轮询好友新消息

持续轮询：

```bash
wecom-cli msg +watch_friend \
  --to "张三-客户" \
  --interval-sec 5 \
  --save-dir /tmp/wecom/media
```

只查一次：

```bash
wecom-cli msg +watch_friend \
  --to "张三-客户" \
  --once \
  --save-dir /tmp/wecom/media
```

最多输出 10 条后退出：

```bash
wecom-cli msg +watch_friend \
  --to "张三-客户" \
  --max-events 10 \
  --save-dir /tmp/wecom/media
```

并行监控多个联系人：

```bash
wecom-cli msg +watch_friend \
  --to "张三-客户" \
  --to "李四-客户" \
  --to "王五-客户" \
  --interval-sec 5 \
  --save-dir /tmp/wecom/media
```

输出是 NDJSON，每条消息一行：

```json
{"chat_id":"zhangsan","target":"张三-客户","chat_name":"张三","send_time":"2026-03-17 09:35:00","userid":"lisi","msgtype":"image","media_id":"MEDIAID_xxxxxx","name":"screenshot.png","local_path":"/tmp/wecom/media/screenshot.png","size":102400,"content_type":"image/png"}
```

说明：

- 文本消息包含 `text`。
- 图片和文件会保存到 `--save-dir`，输出 `local_path`。
- 语音和视频默认不保存，只输出 `media_id` 和元信息。
- 受 `get_message` 限制，只能读取最近 7 天消息。
- 首次运行会把最近 7 天内未记录过的消息视为新消息。
- 状态文件默认写在 `--save-dir` 下，文件名形如 `.watch_friend_<chat_id>.json`。
- 多个 `--to` 会为每个联系人启动独立轮询任务，`--interval-sec 5` 表示每个联系人约每 5 秒轮询一次。
- 多目标模式禁用 macOS 桌面消息 fallback，只使用远程 `get_message`。

## 9. 常见问题

### 找不到新命令

先确认实际执行的是开发版：

```bash
which wecom-cli
wecom-cli --version
```

如果你仍在使用 npm 正式版，需要执行：

```bash
cd /Users/zy/Workspace/tmp/wecom-cli
cargo install --path . --locked --force
```

或直接使用：

```bash
./target/release/wecom-cli msg +send_friend_message --help
```

### cargo: command not found

说明 Rust 工具链没安装或 PATH 没生效：

```bash
source "$HOME/.cargo/env"
cargo --version
```

仍失败就重新安装 Rust。

### 桌面自动化失败

检查：

- 企业微信客户端是否已登录
- 终端是否有“辅助功能”权限
- 终端是否有“自动化”权限
- 企业微信窗口是否能正常搜索联系人
- `--to` 是否唯一，避免同名联系人

### watch_friend 找不到联系人

`+watch_friend` 通过最近 7 天会话列表匹配好友。如果最近 7 天没有和该好友聊天，可能找不到会话。可以先在企业微信里和对方产生一条消息，再重试。

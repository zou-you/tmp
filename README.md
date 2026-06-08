# wecom-cli

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-%3E%3D1.75-orange.svg)](https://www.rust-lang.org/)

企业微信命令行工具。面向人类和 AI Agent，支持在终端中调用企业微信通讯录、文档、智能表格、消息、待办、会议和日程能力。

> 扫码加入企业微信交流群：
>
> <img src="https://wwcdn.weixin.qq.com/node/wework/images/202603241759.3fb01c32cc.png" alt="扫码入群交流" width="200" />

## 当前功能

| 品类         | category   | 当前能力                                                                                                                                                  |
| ------------ | ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 通讯录       | `contact`  | 查询当前用户可见范围成员列表；按姓名/别名本地匹配；macOS helper 支持按手机号添加外部联系人                                                                |
| 消息         | `msg`      | 查询会话列表、拉取 7 天内消息记录、下载图片/文件/语音/视频媒体、发送文本消息；macOS helper 支持给好友发送文本/图片/文件、轮询指定好友或最近活跃单聊新消息 |
| 文档         | `doc`      | 创建文档、读取文档/在线表格 Markdown 内容、覆写文档内容；创建/导出智能文档；上传文档图片和文件                                                            |
| 智能表格     | `doc`      | 创建智能表格文档；管理子表和字段；记录增删改查；helper 支持用本地图片/文件路径写入图片或附件字段                                                          |
| 待办         | `todo`     | 查询待办列表、获取详情、创建、更新、删除待办，以及变更当前用户处理状态                                                                                    |
| 会议         | `meeting`  | 创建预约会议、查询会议列表、获取会议详情、取消会议、更新会议受邀成员                                                                                      |
| 日程         | `schedule` | 查询日程列表和详情、创建/更新/取消日程、添加/移除参与人、查询多成员闲忙                                                                                   |
| Agent Skills | `skills/*` | 内置面向 AI Agent 的 contact、doc、smartsheet、msg、todo、meeting、schedule 操作说明和业务工作流                                                          |

## 安装

### 前置条件

- Node.js `>= 18`
- 企业微信账号
- 支持平台：macOS x64/arm64、Linux x64/arm64、Windows x64
- 可选：企业微信智能机器人 Bot ID 和 Secret，获取方式参考 [官方说明](https://open.work.weixin.qq.com/help2/pc/cat?doc_id=21677)
- 可选：从源码编译开发版时需要 Rust stable 和 `cargo`
- 可选：使用 macOS 桌面 helper 时，需要已登录 `/Applications/企业微信.app`，并授予运行命令的终端“辅助功能”和“自动化”权限

### npm 安装正式版

```bash
npm install -g @wecom/cli
wecom-cli --version
```

如果安装后找不到平台二进制，确认 npm 没有禁用 optional dependencies，然后重新安装：

```bash
npm install -g @wecom/cli
```

### 安装 Agent Skills

如果要让 AI Agent 直接使用本仓库内置 Skills，执行：

```bash
npx skills add WeComTeam/wecom-cli -y -g
```

### 初始化凭证

首次使用远程企业微信能力前，需要交互式配置凭证：

```bash
wecom-cli init
```

配置会加密写入 `~/.config/wecom`。同一台机器上的 npm 全局命令、源码编译产物和 `cargo run` 会复用这份配置。

### 从源码编译开发版

npm 正式版不一定包含本地源码中的最新 helper。要使用当前源码能力，进入仓库后编译：

```bash
cd /Users/zy/Workspace/tmp/wecom-cli
cargo build --release
./target/release/wecom-cli --version
```

安装为当前用户全局命令：

```bash
cargo install --path . --locked --force
which wecom-cli
wecom-cli --version
```

如果 `which wecom-cli` 不是 `~/.cargo/bin/wecom-cli`，把 Rust bin 目录加入 PATH：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

也可以不安装，直接运行源码目录里的二进制：

```bash
# 运行已编译的 release 二进制；需要先执行 cargo build --release
./target/release/wecom-cli msg +watch_all \
  --interval-sec 5 \
  --save-dir ../ \
  --queue-file ../.watch_all_queue.txt \
  --state-file ../.watch_all_queue_state.json \
  --verbose

# 通过 cargo run 构建并运行；参数需要写在 -- 后面
cargo run -- msg +watch_all --interval-sec 10 --save-dir /tmp/wecom/media
```

两种源码运行方式的区别：

| 方式                             | 执行对象                                                  | 是否自动编译             | 典型用途                                 |
| -------------------------------- | --------------------------------------------------------- | ------------------------ | ---------------------------------------- |
| `./target/release/wecom-cli ...` | 已经由 `cargo build --release` 生成的 release 二进制      | 不会自动编译             | 不想全局安装，但要运行优化后的固定二进制 |
| `cargo run -- ...`               | Cargo 构建出的开发二进制，默认是 `target/debug/wecom-cli` | 会在源码有变化时自动编译 | 本地开发、调试当前源码                   |

## 命令格式

### 查看帮助

```bash
wecom-cli --help
wecom-cli <category> --help
wecom-cli <category> <method> --help
```

分类工具列表和远程工具 schema 需要动态获取，因此查看远程工具帮助也需要凭证与网络。

### 调用远程工具

```bash
wecom-cli <category> <method> '<json_args>'
```

示例：

```bash
wecom-cli contact get_userlist '{}'
wecom-cli doc create_doc '{"doc_type": 3, "doc_name": "项目周报"}'
```

规则：

- `category` 支持 `contact`、`doc`、`meeting`、`msg`、`schedule`、`todo`
- 不传 JSON 参数时，远程工具默认使用 `{}`
- JSON 参数建议用单引号包裹，避免 shell 转义双引号
- 远程工具默认超时 30 秒；`msg get_msg_media` 超时 120 秒
- `msg get_msg_media` 会把媒体文件下载到本地临时目录，并在返回 JSON 中给出 `local_path`

### 调用本地 helper

本地 helper 是以 `+` 开头的子命令：

```bash
wecom-cli <category> +<helper_name> [options]
```

当前 helper：

| helper                                     | 用途                                            |
| ------------------------------------------ | ----------------------------------------------- |
| `contact +add_external_friend`             | macOS 企业微信客户端按手机号添加外部联系人      |
| `msg +send_friend_message`                 | macOS 企业微信客户端给好友发送文本、图片或文件  |
| `msg +watch_friend`                        | 轮询指定好友新消息，图片和文件自动保存到本地    |
| `msg +watch_all`                           | 准实时轮询最近活跃单聊新消息，支持动态监听队列  |
| `doc +smartpage_create`                    | 创建智能文档，支持从本地文件读取子页面内容      |
| `doc +smartsheet_add_records_auto_file`    | 添加智能表格记录，支持本地图片/附件路径自动上传 |
| `doc +smartsheet_update_records_auto_file` | 更新智能表格记录，支持本地图片/附件路径自动上传 |

如果执行时报 `unrecognized subcommand '+xxx'`，说明当前 PATH 中的 `wecom-cli` 不是包含该 helper 的版本。进入源码目录后执行：

```bash
cargo install --path . --locked --force
```

或直接运行：

```bash
cargo run -- msg +watch_all --help
```

## 常用命令

### 通讯录

```bash
# 获取当前用户可见范围内的成员列表
wecom-cli contact get_userlist '{}'

# macOS 客户端按手机号添加外部联系人
wecom-cli contact +add_external_friend \
  --phone "13800000000" \
  --remark "张三-客户" \
  --greeting "你好，我是..."
```

`+add_external_friend` 返回 `pending` 时只表示已提交或打开验证流程，不代表对方已确认添加。

### 消息

```bash
# 查询会话列表
wecom-cli msg get_msg_chat_list '{"begin_time": "2026-03-01 00:00:00", "end_time": "2026-03-07 23:59:59"}'

# 拉取会话消息
wecom-cli msg get_message '{"chat_type": 1, "chatid": "zhangsan", "begin_time": "2026-03-07 09:00:00", "end_time": "2026-03-07 18:00:00"}'

# 下载消息媒体
wecom-cli msg get_msg_media '{"media_id": "MEDIAID_xxxxxx"}'

# 发送文本消息
wecom-cli msg send_message '{"chat_type": 1, "chatid": "zhangsan", "msgtype": "text", "text": {"content": "hello world"}}'
```

macOS 桌面 helper：

```bash
# 给好友发送文本、图片或文件，三选一
wecom-cli msg +send_friend_message --to "张三-客户" --text "你好"
wecom-cli msg +send_friend_message --to "张三-客户" --image /path/to/a.png
wecom-cli msg +send_friend_message --to "张三-客户" --file /path/to/report.pdf

# 轮询指定好友新消息
wecom-cli msg +watch_friend --to "张三-客户" --interval-sec 5 --save-dir /tmp/wecom/media

# 并行监听多个联系人
wecom-cli msg +watch_friend --to "张三-客户" --to "李四-客户" --interval-sec 5

# 轮询最近活跃单聊
wecom-cli msg +watch_all --interval-sec 10 --save-dir /tmp/wecom/media

# 不全局安装时，直接运行已编译的 release 二进制，并使用动态监听队列
./target/release/wecom-cli msg +watch_all \
  --interval-sec 5 \
  --save-dir ../ \
  --queue-file ../.watch_all_queue.txt \
  --state-file ../.watch_all_queue_state.json \
  --verbose
```

`+watch_friend` 和 `+watch_all` 输出 NDJSON，每条消息一行。图片和文件会保存到 `--save-dir`，输出中包含 `local_path`。

常用参数：

| 参数                 | 适用命令                      | 说明                                                                  |
| -------------------- | ----------------------------- | --------------------------------------------------------------------- |
| `--interval-sec`     | `+watch_friend`、`+watch_all` | 轮询间隔秒数，必须大于 0                                              |
| `--save-dir`         | `+watch_friend`、`+watch_all` | 图片和文件保存目录；不传时使用 wecom-cli 媒体临时目录                 |
| `--once`             | `+watch_friend`、`+watch_all` | 只轮询一次后退出                                                      |
| `--max-events`       | `+watch_friend`、`+watch_all` | 输出达到指定条数后退出；`0` 表示不限制                                |
| `--idle-timeout-sec` | `+watch_friend`、`+watch_all` | 无新消息达到指定秒数后退出；`0` 表示不限制                            |
| `--verbose`          | `+watch_friend`、`+watch_all` | 将诊断信息写到 stderr，stdout 仍只输出消息 NDJSON                     |
| `--to`               | `+watch_friend`、`+watch_all` | `+watch_friend` 为监听目标；`+watch_all` 为初始监听队列用户，可重复传 |
| `--queue-file`       | `+watch_all`                  | 动态监听队列文件，支持一行一个用户名或 JSON 字符串数组                |
| `--state-file`       | `+watch_all`                  | 自定义去重状态文件；不传时默认写入 `--save-dir/.watch_all.json`       |
| `--include-server`   | `+watch_all`                  | 队列模式下仍启用服务端消息接口轮询                                    |

`+watch_friend` 只能读取最近 7 天消息；多目标模式会禁用 macOS 桌面消息 fallback。`+watch_all` 启动时只建立基线，不回放历史消息。

### 文档

```bash
# 创建普通文档
wecom-cli doc create_doc '{"doc_type": 3, "doc_name": "项目周报"}'

# 读取文档或在线表格内容，type=2 表示 Markdown
wecom-cli doc get_doc_content '{"url": "https://doc.weixin.qq.com/doc/xxx", "type": 2}'

# 覆写文档正文，content_type=1 表示 Markdown
wecom-cli doc edit_doc_content '{"docid": "DOCID", "content": "# 标题\n\n正文内容", "content_type": 1}'
```

URL 路由：

| URL 模式        | 品类     | 推荐接口                                               |
| --------------- | -------- | ------------------------------------------------------ |
| `/doc/*`        | 文档     | `get_doc_content`、`edit_doc_content`                  |
| `/sheet/*`      | 在线表格 | `get_doc_content`                                      |
| `/smartsheet/*` | 智能表格 | `smartsheet_*` 系列接口                                |
| `/smartpage/*`  | 智能文档 | `smartpage_export_task`、`smartpage_get_export_result` |

智能文档：

```bash
# 从本地 Markdown 文件创建智能文档子页面
wecom-cli doc +smartpage_create '{"title": "项目概览", "pages": [{"page_title": "需求文档", "content_type": 1, "page_filepath": "/path/to/requirements.md"}]}'

# 发起智能文档导出任务
wecom-cli doc smartpage_export_task '{"url": "https://doc.weixin.qq.com/smartpage/xxx", "content_type": 1}'

# 查询导出结果
wecom-cli doc smartpage_get_export_result '{"task_id": "TASK_ID"}'
```

### 智能表格

```bash
# 创建智能表格文档
wecom-cli doc create_doc '{"doc_type": 10, "doc_name": "项目任务表"}'

# 查询子表和字段
wecom-cli doc smartsheet_get_sheet '{"docid": "DOCID"}'
wecom-cli doc smartsheet_get_fields '{"docid": "DOCID", "sheet_id": "SHEETID"}'

# 新增、更新、删除字段
wecom-cli doc smartsheet_add_fields '{"docid": "DOCID", "sheet_id": "SHEETID", "fields": [{"field_title": "任务名称", "field_type": "FIELD_TYPE_TEXT"}]}'
wecom-cli doc smartsheet_update_fields '{"docid": "DOCID", "sheet_id": "SHEETID", "fields": [{"field_id": "FIELDID", "field_title": "新标题", "field_type": "FIELD_TYPE_TEXT"}]}'
wecom-cli doc smartsheet_delete_fields '{"docid": "DOCID", "sheet_id": "SHEETID", "field_ids": ["FIELDID"]}'

# 记录增删改查
wecom-cli doc smartsheet_get_records '{"docid":"DOCID","sheet_id":"SHEETID"}'
wecom-cli doc smartsheet_add_records '{"docid":"DOCID","sheet_id":"SHEETID","records":[{"values":{"任务名称":[{"type":"text","text":"完成需求文档"}]}}]}'
wecom-cli doc smartsheet_update_records '{"docid":"DOCID","sheet_id":"SHEETID","key_type":"CELL_VALUE_KEY_TYPE_FIELD_TITLE","records":[{"record_id":"RECORDID","values":{"任务名称":[{"type":"text","text":"更新后的内容"}]}}]}'
wecom-cli doc smartsheet_delete_records '{"docid":"DOCID","sheet_id":"SHEETID","record_ids":["RECORDID"]}'
```

带本地图片或附件的记录写入：

```bash
wecom-cli doc +smartsheet_add_records_auto_file '{"docid":"DOCID","sheet_id":"SHEETID","records":[{"values":{"图片":[{"image_path":"/path/to/image.jpg"}],"文件":[{"file_path":"/path/to/file.txt"}]}}]}'

wecom-cli doc +smartsheet_update_records_auto_file '{"docid":"DOCID","sheet_id":"SHEETID","key_type":"CELL_VALUE_KEY_TYPE_FIELD_TITLE","records":[{"record_id":"RECORDID","values":{"图片":[{"image_path":"/path/to/image.jpg"}],"文件":[{"file_path":"/path/to/file.txt"}]}}]}'
```

限制：

- 图片最大 30 MB
- 附件最大 10 MB
- 添加或更新记录建议单次不超过 500 行
- 删除记录单次必须在 500 行内，且不可逆

### 待办

```bash
# 查询待办列表
wecom-cli todo get_todo_list '{"limit": 10}'

# 获取待办详情
wecom-cli todo get_todo_detail '{"todo_id_list": ["TODO_ID"]}'

# 创建待办
wecom-cli todo create_todo '{"content": "完成 Q2 规划文档", "remind_time": "2026-06-09 09:00:00"}'

# 更新待办
wecom-cli todo update_todo '{"todo_id": "TODO_ID", "content": "更新后的内容"}'

# 删除待办
wecom-cli todo delete_todo '{"todo_id": "TODO_ID"}'
```

`get_todo_list` 返回的是概要信息，展示给用户前通常需要继续调用 `get_todo_detail` 获取内容和分派人。

### 会议

```bash
# 创建预约会议
wecom-cli meeting create_meeting '{"title": "周例会", "meeting_start_datetime": "2026-06-09 15:00", "meeting_duration": 3600}'

# 查询会议列表
wecom-cli meeting list_user_meetings '{"begin_datetime": "2026-06-01 00:00", "end_datetime": "2026-06-30 23:59", "limit": 100}'

# 获取会议详情
wecom-cli meeting get_meeting_info '{"meetingid": "MEETING_ID"}'

# 取消会议
wecom-cli meeting cancel_meeting '{"meetingid": "MEETING_ID"}'

# 更新受邀成员
wecom-cli meeting set_invite_meeting_members '{"meetingid": "MEETING_ID", "invitees": [{"userid": "zhangsan"}]}'
```

会议列表查询范围限制为当日及前后 30 天。

### 日程

```bash
# 查询日程 ID 列表
wecom-cli schedule get_schedule_list_by_range '{"start_time": "2026-06-09 00:00:00", "end_time": "2026-06-09 23:59:59"}'

# 获取日程详情
wecom-cli schedule get_schedule_detail '{"schedule_id_list": ["SCHEDULE_ID"]}'

# 创建日程
wecom-cli schedule create_schedule '{"schedule": {"start_time": "2026-06-09 14:00:00", "end_time": "2026-06-09 15:00:00", "summary": "需求评审", "attendees": [{"userid": "zhangsan"}], "reminders": {"is_remind": 1, "remind_before_event_secs": 900, "timezone": 8}}}'

# 修改日程
wecom-cli schedule update_schedule '{"schedule": {"schedule_id": "SCHEDULE_ID", "summary": "技术方案评审"}}'

# 取消日程
wecom-cli schedule cancel_schedule '{"schedule_id": "SCHEDULE_ID"}'

# 管理参与人
wecom-cli schedule add_schedule_attendees '{"schedule_id": "SCHEDULE_ID", "attendees": [{"userid": "zhangsan"}]}'
wecom-cli schedule del_schedule_attendees '{"schedule_id": "SCHEDULE_ID", "attendees": [{"userid": "zhangsan"}]}'

# 查询闲忙
wecom-cli schedule check_availability '{"check_user_list": ["zhangsan", "lisi"], "start_time": "2026-06-09 14:00:00", "end_time": "2026-06-09 18:00:00"}'
```

日程列表查询仅支持当日前后 30 天。返回中的 `start_time` / `end_time` 多为 Unix 时间戳秒，需要转成可读时间。

## macOS 桌面 helper 权限

以下 helper 依赖 macOS 企业微信客户端桌面自动化：

- `contact +add_external_friend`
- `msg +send_friend_message`
- `msg +watch_friend` 的单目标桌面消息 fallback
- `msg +watch_all` 的桌面监听队列模式

配置步骤：

1. 确认企业微信客户端已安装并登录，默认 bundle id 为 `com.tencent.WeWorkMac`
2. 打开“系统设置 -> 隐私与安全性 -> 辅助功能”，把运行命令的终端加入并启用，例如 Terminal、iTerm、Cursor、VS Code
3. 打开“系统设置 -> 隐私与安全性 -> 自动化”，允许该终端控制“企业微信”和“System Events”
4. 确保 `--to` 对应的联系人名称或备注名能在企业微信客户端中唯一匹配

## Agent Skills

仓库内置 Skills 位于 `skills/`：

| Skill                 | 品类     | 说明                                            |
| --------------------- | -------- | ----------------------------------------------- |
| `wecomcli-contact`    | contact  | 通讯录查询、成员匹配、添加外部联系人 helper     |
| `wecomcli-msg`        | msg      | 会话、消息、媒体下载、文本发送、好友消息 helper |
| `wecomcli-doc`        | doc      | 文档、在线表格、智能文档创建/读取/编辑          |
| `wecomcli-smartsheet` | doc      | 智能表格子表、字段和记录管理                    |
| `wecomcli-todo`       | todo     | 待办查询、详情、创建、更新、删除和状态变更      |
| `wecomcli-meeting`    | meeting  | 预约会议创建、查询、取消、成员更新              |
| `wecomcli-schedule`   | schedule | 日程查询、创建、更新、取消、参与人和闲忙查询    |

AI Agent 需要更细的业务工作流时，直接读取对应 `skills/<skill-name>/SKILL.md`。

## 运行时路径和环境变量

| 项目         | 默认位置                      | 备注                                |
| ------------ | ----------------------------- | ----------------------------------- |
| 配置目录     | `~/.config/wecom`             | 可由 `WECOM_CLI_CONFIG_DIR` 覆盖    |
| 机器人凭证   | `<config_dir>/bot.enc`        | 执行 `wecom-cli init` 后创建        |
| MCP 配置缓存 | `<config_dir>/mcp_config.enc` | 配置凭证后更新                      |
| 媒体临时目录 | `<system_tmp>/wecom/media`    | 可由 `WECOM_CLI_TMP_DIR` 覆盖根目录 |

| 环境变量                        | 作用                                  |
| ------------------------------- | ------------------------------------- |
| `WECOM_CLI_CONFIG_DIR`          | 覆盖默认配置目录                      |
| `WECOM_CLI_TMP_DIR`             | 覆盖媒体临时目录的根目录              |
| `WECOM_CLI_LOG_LEVEL`           | 打开 stderr 日志并设置过滤级别        |
| `WECOM_CLI_LOG_FILE`            | 打开 JSON 日志输出，按天写入 `ww.log` |
| `WECOM_CLI_MCP_CONFIG_ENDPOINT` | 覆盖默认 MCP 配置接口地址             |

## 本地开发

项目主体是 Rust CLI，npm 包负责分发平台二进制。

| 路径           | 说明                                                          |
| -------------- | ------------------------------------------------------------- |
| `src/`         | Rust CLI 主实现，包括命令解析、认证、JSON-RPC、日志和媒体处理 |
| `src/helpers/` | 本地 helper 子系统                                            |
| `bin/wecom.js` | npm 入口脚本，负责定位并执行当前平台二进制                    |
| `packages/*`   | 各平台 npm 二进制包                                           |
| `skills/*`     | Agent Skills 和业务参考资料                                   |
| `docs/`        | 历史文档入口，当前用户手册以本 README 为准                    |

常用开发命令：

```bash
cargo build
cargo test
cargo fmt
cargo clippy --all-targets --all-features
```

新增或维护 helper 时，先读 `src/helpers/AGENTS.md`。

## 常见问题

### `wecom-cli: command not found`

确认全局 npm bin 或 Rust bin 在 PATH 中：

```bash
npm prefix -g
ls "$(npm prefix -g)/bin/wecom-cli"
which wecom-cli
export PATH="$HOME/.cargo/bin:$PATH"
```

### 找不到 `+watch_all` 等新 helper

当前 PATH 中的 `wecom-cli` 不是开发版。进入源码目录后重新安装：

```bash
cd /Users/zy/Workspace/tmp/wecom-cli
cargo install --path . --locked --force
```

### `cargo: command not found`

安装 Rust stable：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

### 桌面自动化失败

检查：

- 企业微信客户端是否已登录
- 终端是否有“辅助功能”权限
- 终端是否有“自动化”权限
- 企业微信窗口是否能正常搜索联系人
- `--to` 是否能唯一匹配目标联系人

### `+watch_friend` 找不到联系人

`+watch_friend` 会先从最近 7 天会话列表匹配好友，再尝试通讯录。若最近 7 天没有和该好友聊天，可能无法解析到有效会话。可以先在企业微信里与对方产生一条消息，再重试。

## 许可证

本项目基于 [MIT 许可证](./LICENSE) 开源。

## 使用说明

### 配置凭证 `init`

交互式配置企业微信机器人凭证，加密存储到本地。仅需执行一次。
- 若选择手动配置 Bot ID 和 Secret，获取方式[参考](https://open.work.weixin.qq.com/help2/pc/cat?doc_id=21677)
- 若选择扫码接入，需使用企业微信扫码创建绑定

```bash
wecom-cli init
```

### 查看帮助 `--help`
支持获取各级命令的使用方式

```bash
# 列出所有支持的命令和品类
wecom-cli --help

# 列出指定品类下的所有工具
wecom-cli <category> --help

# 列出指定工具的所需要的输入
wecom-cli <category> <method> --help
```

说明：
- 分类工具列表和工具 schema 都需要动态获取，因此“查看帮助”需要凭证与网络。

### 调用工具

通用格式：

```bash
wecom-cli <category> <method> [json_args] 
```
其中 `category` 为业务品类标识，支持以下值：

| category   | 品类          |
| ---------- | ------------- |
| `contact`  | 通讯录        |
| `doc`      | 文档/智能表格 |
| `meeting`  | 会议          |
| `msg`      | 消息          |
| `schedule` | 日程          |
| `todo`     | 待办          |

工具调用行为：
- `wecom-cli <category>` 获取该品类下的支持调用工具
- `wecom-cli <category> <method> --help`  获取该指定工具的参数定义
- `wecom-cli <category> <method>`  执行调用工具并指定参数为'{}' 
- `wecom-cli <category> <method> 'json_args'`  执行该工具调用

示例：
```bash
## 调用工具 — 获取通讯录可见范围内的成员列表
wecom-cli contact get_userlist '{}'

## 调用工具 — 创建文档
wecom-cli doc create_doc '{"doc_type": 3, "doc_name": "项目周报"}'
```

### 本地 helper

以 `+` 开头的子命令为本地 helper，用于处理远程 MCP 工具暂不覆盖的客户端侧能力。

开发版编译、全局安装、macOS 权限配置和完整排错说明见 [`desktop-helpers-usage.md`](desktop-helpers-usage.md)。

```bash
## macOS 企业微信客户端：按手机号添加外部联系人
wecom-cli contact +add_external_friend \
  --phone "13800000000" \
  --remark "张三-客户" \
  --greeting "你好，我是..."

## macOS 企业微信客户端：给好友发送文本、图片或文件
wecom-cli msg +send_friend_message --to "张三-客户" --text "你好"
wecom-cli msg +send_friend_message --to "张三-客户" --image /path/to/a.png
wecom-cli msg +send_friend_message --to "张三-客户" --file /path/to/report.pdf

## 轮询指定好友新消息，图片和文件会保存到本地目录
wecom-cli msg +watch_friend --to "张三-客户" --interval-sec 5 --save-dir /tmp/wecom/media
wecom-cli msg +watch_friend --to "张三-客户" --to "李四-客户" --interval-sec 5
```

说明：
- 外部联系人添加、发送图片和发送文件依赖 macOS 桌面自动化，仅支持已登录的 `/Applications/企业微信.app`。
- 使用桌面自动化前，请在系统设置中授予当前终端“辅助功能”和“自动化”权限。
- `+watch_friend` 复用消息 MCP 接口，只能读取最近 7 天内消息；首次运行会把最近窗口内未记录过的消息作为新消息输出。
- `+watch_friend` 可重复传 `--to` 并行监控多个联系人，多目标模式禁用 macOS 桌面消息 fallback。

补充说明：
- 工具调用默认超时为 30 秒；`get_msg_media` 超时为 120 秒。
- `get_msg_media`会把媒体文件下载到本地临时目录，返回结果字段`local_path`为文件保存的路径 。


## 运行时路径

| 项目 | 默认位置 | 备注 |
| --- | --- | --- |
| 配置目录 | `~/.config/wecom` | 可由 `WECOM_CLI_CONFIG_DIR` 覆盖 |
| 机器人凭证 | `<config_dir>/bot.enc` | 配置凭证时创建 |
| MCP 配置缓存 | `<config_dir>/mcp_config.enc` | 配置凭证后更新 |
| 媒体临时目录 | `<system_tmp>/wecom/media` | 可由 `WECOM_CLI_TMP_DIR` 覆盖根目录 |

## 环境变量

| 变量 | 作用 |
| --- | --- |
| `WECOM_CLI_CONFIG_DIR` | 覆盖默认配置目录 |
| `WECOM_CLI_TMP_DIR` | 覆盖媒体临时目录的根目录 |
| `WECOM_CLI_LOG_LEVEL` | 打开 stderr 日志并设置过滤级别 |
| `WECOM_CLI_LOG_FILE` | 打开 JSON 日志输出，按天写入 `ww.log` |
| `WECOM_CLI_MCP_CONFIG_ENDPOINT` | 覆盖默认 MCP 配置接口地址 |

# AI Agent 指引

> 本文件帮助 AI Agent 快速定位项目关键文件，避免重复阅读无关代码。

## 文档入口

- 用户手册、安装方式、初始化、命令格式、功能总览、常用示例、运行时路径和排错：[`README.md`](README.md)
- 旧的 `docs/*` 页面只保留为跳转入口；不要再往 `docs/` 里新增安装或命令使用说明
- 具体业务工作流仍以 `skills/<skill-name>/SKILL.md` 为准

## 项目概述

`wecom-cli` 是一个企业微信 CLI 工具（Rust），通过 JSON-RPC 调用远程 MCP 服务。CLI 结构为：

```bash
wecom-cli <category> <method>          # 远程工具调用
wecom-cli <category> +<helper_name>    # 本地 helper（+ 前缀）
```

## 任务路由

根据任务类型，优先阅读对应入口：

| 任务                                       | 指引文件                                         | 你需要修改的文件             |
| ------------------------------------------ | ------------------------------------------------ | ---------------------------- |
| 更新用户文档、安装说明、命令示例、功能总览 | [`README.md`](README.md)                         | `README.md`                  |
| 新建或维护 helper                          | [`src/helpers/AGENTS.md`](src/helpers/AGENTS.md) | 对应 helper 的 `.rs` 文件    |
| 理解人类的需求格式                         | [`src/helpers/HUMANS.md`](src/helpers/HUMANS.md) | 通常不需要改文件             |
| 更新 Agent Skill 的业务流程                | `skills/<skill-name>/SKILL.md`                   | 对应 Skill 文件和 references |

## 项目结构速查

```text
src/
├── main.rs                # CLI 入口，构建 clap Command
├── json_rpc.rs            # 远程调用：call_tool(category, method, args)
├── fs_util/               # 文件系统工具（atomic_write, sanitize_filename）
├── service/
│   └── handler.rs         # 调度逻辑：helper 优先 -> fallback 远程调用
└── helpers/               # Helper 子系统（详见 helpers/AGENTS.md）
    ├── mod.rs             # 模块声明
    ├── registry.rs        # Helper trait + HelperRegistry
    ├── AGENTS.md          # AI 实现指南
    └── <category>/        # 按 category 分目录存放 helper
```

## 注意事项

- 用户安装、执行命令、功能清单统一维护在 [`README.md`](README.md)
- 不要在 `docs/cli-reference.md`、`docs/desktop-helpers-usage.md`、`docs/skills.md` 里恢复重复内容
- 新增 helper 时通常只需要改 `src/helpers/**` 并在 [`README.md`](README.md) 的 helper 清单中补充说明
- 不要修改 `main.rs` 和 `service/` 下的文件来注册 helper；helper 系统通过 registry 自动注册

# 无界面版管理 CLI 设计

状态：方案，待实现。目标是在没有桌面窗口的机器上完成 OpenKeylet 写入授权、GitHub 加密备份和恢复，使 Grok Bot 云电脑可以独立初始化并使用自己的保险库。

## 1. 范围

沿用无界面版现有的 `openkeylet mcp stdio` 和保险库存储。增加两组本地管理命令，桌面界面与 CLI 共用权限配置、备份格式和后端校验。现有 `backup export/import` 继续处理本地文件。首版连接已有的私有仓库，备份和恢复由管理员手动触发。

Grok Bot 通过 Command 类型的自定义连接器，在其云电脑上启动对应系统与架构的 OpenKeylet 无界面二进制。连接器只运行 `mcp stdio`。命令及管理员操作必须指向同一保险库目录，可通过 `OPENKEYLET_DATA_DIR` 显式指定。配置示例及命令名以下均为拟定接口。

## 2. 写入权限

```text
openkeylet permissions status
openkeylet permissions folder add agent-test
openkeylet permissions enable
openkeylet permissions folder remove agent-test
openkeylet permissions disable
```

| 命令 | 行为 |
| --- | --- |
| `status` | 输出总开关、白名单和当前是否实际允许 MCP 写入。 |
| `folder add/remove <name>` | 使用现有名称校验，去除首尾空白，按完整名称匹配。添加时拒绝空名或重复名，移除只改变授权。名单为空时拒绝一切 MCP 写入。 |
| `enable/disable` | 首次默认关闭，启用要求至少一个授权文件夹。关闭保留名单，已连接的 MCP 下一次写入立即读取新策略。 |

复用 `SettingsPatch`、文件夹规范化和 [MCP 写入授权规则](mcp-vault-write-permissions.md)。添加、移除及启用前的名单检查在同一次保险库锁内完成，避免并发管理命令覆盖彼此改动。保持现有项目版本冲突检查和秘密可见性规则。

管理命令属于本地管理员入口。文件夹白名单约束 MCP 的项目写入。如果 Bot 能以同一系统用户运行管理 CLI、访问保险库文件，或通过通用终端执行任意命令，白名单不能隔离这些路径。需要与 Bot 隔离管理权限时，应在部署层隔离系统用户、文件和通用命令能力；首版不新增权限服务。

## 3. GitHub 加密备份与恢复

```text
openkeylet github-backup status
openkeylet github-backup connect owner/private-repo --mode existing --auth gh
openkeylet github-backup restore
openkeylet github-backup upload
openkeylet github-backup disconnect
```

| 命令 | 行为 |
| --- | --- |
| `connect` | `--mode existing` 要求已有快照且恢复密钥正确，成功后标记为待恢复。`--mode new` 用于尚无快照的已有私有仓库。连接只验证并保存本机配置。 |
| `restore` | 下载、校验、解密并合并快照；输出新增和跳过的重复项数量。内容不同的项目按现有冲突规则分别保留。 |
| `upload` | 手动上传当前保险库的加密快照。接入已有快照的设备须先恢复。远端文件变更时按 SHA 拒绝覆盖，提示重新恢复。 |
| `status` | 显示本机记录的连接状态、仓库、分支、待恢复标记和最近操作时间。 |
| `disconnect` | 清理本机连接配置，GitHub 上的历史快照保留。 |

复用 `GithubBackup` 的 `connect`、`restore`、`upload`、`status` 和 `disconnect`。保留私有仓库检查、现有加密与快照格式、加密的设备配置和 GitHub SHA 并发检查。设备上的连接、上传、恢复及断开应串行更新备份状态，桌面与 CLI 如可同时运行，应共用同一设备锁。

首版认证复用当前机器显式登录的 GitHub CLI（`gh`）。恢复密钥由管理员在受控终端隐藏输入。新备份使用现有生成逻辑，管理员确认已妥善保存密钥后才连接。GitHub Token 和恢复密钥不进入命令参数、Bot 聊天、MCP 响应或日志。GitHub 快照用于备份与合并恢复；写入授权和 GitHub 设备连接需要每台机器单独配置。

## 4. 验收与分工

| 环境 | 要验证的内容 | 所需配合 |
| --- | --- | --- |
| 本机，临时保险库 | 编译无界面版。验证默认关闭、空名单禁止启用、名称完整匹配、允许文件夹新增和修改、越权移动拒绝。对同一个已连接的 MCP 进程撤权后，用最新版本重试，确认立即拒绝且项目未被修改。 | 无。使用本机隔离进程和模拟数据，不接触现有保险库。 |
| 本机，模拟 GitHub 服务 | 复用现有 GitHub 测试设施。用两个临时保险库验证加密上传、恢复合并、重复导入、公有仓库拒绝、错误恢复密钥、远端冲突保护。检查输出及日志不含秘密。 | 无。模拟服务能验证 CLI 和后端行为。 |
| 真实 GitHub 私有测试仓库 | 经授权后用模拟项目验证 `connect → upload → restore`。核实仓库中只有加密快照，另一保险库恢复后能查询到项目，且写入授权仍需单独配置。 | 需用户指定或提供专用私有测试仓库，并由管理员配置对应的 `gh` 登录和恢复密钥。 |
| Grok Bot 云电脑与实际聊天 | 查看云电脑系统和 CPU 架构，运行配套的无界面版。注册 Command 连接器后，实际聊天调用 `vault_write_capabilities`，再完成默认拒绝、授权后新增和更新（版本 `1 → 2`）、回查及撤权后拒绝。最后从私有测试仓库向云电脑上的空保险库恢复，并在 Bot 聊天中查到恢复的模拟项目。 | 需用户提供云电脑及连接器的操作入口，处理 Grok Bot 的接入批准。GitHub 登录及恢复密钥由管理员在受控终端完成。 |

目前已有本机 macOS ARM64 无界面构建和 `stdio` 工具发现的验证记录。上表是新增 CLI 的待验收项目。只有实际 Grok Bot 聊天成功调用工具并回查结果，才算 Bot 接入通过；本机工具发现只证明 OpenKeylet 进程就绪。

Grok Bot 接入方式参考 [Grok Bot 自定义连接器说明](https://docs.x.ai/grok-bot/computer-and-apps)。

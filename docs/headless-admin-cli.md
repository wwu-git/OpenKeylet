# 无界面版管理 CLI 设计

状态：已实现，通过本机验收；用户回传的 Grok Bot 云端模拟数据验收已通过，包含真实 GitHub 加密备份、恢复和 `0ffaed9` 冲突修复回归。详见 [云端验收记录](headless-admin-cli-acceptance.md)。目标是在没有桌面窗口的机器上完成 OpenKeylet 写入授权、GitHub 加密备份和恢复，使 Grok Bot 云电脑可以独立初始化并使用自己的保险库。

## 1. 范围

沿用无界面版现有的 `openkeylet mcp stdio` 和保险库存储。增加两组本地管理命令，桌面界面与 CLI 共用权限配置、备份格式和后端校验。现有 `backup export/import` 继续处理本地文件。首版连接已有的私有仓库，备份和恢复由管理员手动触发。

Grok Bot 通过 Command 类型的自定义连接器，在其云电脑上启动对应系统与架构的 OpenKeylet 无界面二进制。连接器只运行 `mcp stdio`。验收前确认管理员有受控终端，并核对连接器和管理 CLI 使用同一个绝对数据目录。两边都通过 `OPENKEYLET_DATA_DIR` 显式指定。构建命令为 `npm run build:headless`，二进制路径为 `src-tauri/target/release/openkeylet`。

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

复用现有保险库更新、文件夹规范化和 [MCP 写入授权规则](mcp-vault-write-permissions.md)。现有 `SettingsPatch` 会替换整张名单。CLI 的添加、移除和启用前检查必须在同一次保险库锁内读取最新名单并保存，避免两个进程各自修改旧副本。保持项目版本冲突检查和秘密可见性规则。

管理命令的信任边界是操作系统用户及数据目录权限。桌面端的所有者解锁只约束桌面入口。文件夹白名单只限制 MCP 项目写入。如果 Bot 能以同一系统用户访问保险库文件、执行管理 CLI 或调用通用终端，它也具备这些路径的权限。需要隔离时，由部署环境隔离系统用户、文件和通用命令能力。

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
| `upload` | 手动上传当前保险库的加密快照。接入已有快照的设备须先恢复。按现有 SHA 检查拒绝覆盖已变化的远端文件。 |
| `status` | 只读取本机记录的连接状态、仓库、分支、待恢复标记和最近操作时间。 |
| `disconnect` | 清理本机连接配置，GitHub 上的历史快照保留。 |

桌面端与 CLI 共用 `GithubBackup` 的私有仓库检查、加密快照格式、加密的设备配置和远端 SHA 检查。共享后端按数据目录使用一把跨进程设备锁，覆盖 `connect/upload/restore/disconnect` 的整个状态读写流程。锁被占用时提示稍后重试。现有桌面进程内锁和单次原子写文件不足以防止桌面与 CLI 同时修改设备状态。GitHub 请求期间不持有保险库全局写锁，恢复时仍使用已有保险库事务合并项目。

**恢复密钥交互。** 首版使用本机显式登录的 `gh` 作为 GitHub 凭据来源。`connect --mode new` 复用现有生成逻辑，仅在管理员的交互终端展示一次新密钥。管理员确认已保存后才连接。`--mode existing` 在同一类终端隐藏输入已有密钥。无交互终端、取消输入或未确认保存时返回明确错误或取消状态，不保存连接配置。Token 和恢复密钥不得进入命令参数、标准输出、Bot 聊天、MCP 响应或日志。新密钥只写到受控交互终端。

**远端异常与恢复范围。** 远端快照仍存在但已更新时，提示先恢复再上传。曾有的快照缺失时，上传保留现有拒绝逻辑，恢复返回快照缺失，提示管理员检查仓库、分支或 GitHub 历史。首版没有强制覆盖入口。`new` 模式只用于管理员确认的初始化，不能作为冲突自动重试。恢复合并项目数据，写入授权和 GitHub 设备连接需要每台机器单独配置。OCI 签名私钥不随快照恢复。

## 4. 验收与分工

| 环境 | 要验证的内容 | 所需配合 |
| --- | --- | --- |
| 本机，真实 CLI 与临时保险库 | 用真实进程检查命令参数、退出码、默认关闭、空名单拒绝启用、名称精确匹配、授权目录内新增和修改、越权移动被拒绝。并发添加两个不同文件夹后两项都应保留。移除最后一项后，用同一个 MCP 连接和最新版本重试，确认立即被拒绝且项目未变。 | 可独立完成。使用临时保险库和模拟数据。 |
| 本机，CLI 分发与模拟 GitHub 后端 | 在分发层验证参数传递、两种密钥交互、取消和非交互错误。复用已有模拟服务测试加密上传、合并及重复恢复、公有仓库及错误密钥拒绝、快照变更与缺失、两进程的设备锁冲突。检查输出和日志不泄露秘密。测试注入留在测试环境。 | 可独立完成。无需访问真实 GitHub，也无需给生产 CLI 增加 API 地址配置。 |
| 真实 GitHub 私有测试仓库 | 用模拟项目连接已有私有仓库，从第一个保险库上传加密快照，再用第二个空保险库恢复并回查。确认上传文件是加密快照，写入授权仍需单独配置。此项可独立于 Grok Bot 完成。 | 需用户指定专用私有测试仓库并授权写入，由管理员在受控终端配置 `gh` 登录和恢复密钥。 |
| Grok Bot 云电脑与实际聊天 | 确认系统、架构、管理员终端和共同数据目录。注册 Command 连接器，从实际聊天调用 `vault_write_capabilities`，验证默认拒绝、授权后新增和更新（版本 `1 → 2`）、回查及撤权后拒绝。真实 GitHub 端到端验收再从私有测试仓库恢复到云电脑上的空保险库，由 Bot 聊天回查模拟项目。 | 需用户提供云电脑受控终端和连接器入口，并完成接入批准。真实 GitHub 恢复的登录和密钥由管理员操作。缺少受控终端时，单独报告已完成的 Bot 调用和 GitHub 验证，云端恢复留待验收。 |

### 本机验收结果

初版（`b16abb1`）在 macOS ARM64 上，无界面版 Rust 测试 154 项、默认桌面版 Rust 测试 185 项通过，桌面 GitHub 备份界面回归通过。已使用实际无界面二进制和同一 MCP 会话验证 CLI 授权、文件夹精确匹配、项目新增和修改、越权移动及即时撤权。独立进程持有设备锁时，CLI 读取状态仍可用，写操作返回忙。伪终端验证新密钥只展示一次，已有密钥输入无回显，取消或无效密钥未写入连接配置。GitHub 加密上传、换保险库恢复、远端冲突与缺失用模拟服务完成。

### 云端验收流程（后续部署复用）

1. 提供 Grok Bot 云电脑的系统和 CPU 架构（Linux 或 macOS 可执行 `uname -s; uname -m`），以及管理员受控终端和 Command 连接器入口。准备对应构建与临时数据目录。
2. 管理员和连接器使用同一 `OPENKEYLET_DATA_DIR`。注册 OpenKeylet 的 `mcp stdio` 命令，完成批准。在实际 Bot 聊天调用写入能力查询，按上表验证默认拒绝、授权后新增和修改、撤权后拒绝。
3. 指定并授权专用私有测试仓库，由管理员登录 `gh`，在受控终端使用测试恢复密钥。以本机临时保险库上传，在云电脑的空保险库运行 `connect --mode existing` 和 `restore`。最后在 Bot 聊天回查恢复的模拟项目，并确认源设备的写入授权未随备份迁移。

Grok Bot 接入以实际聊天成功调用并回查为准。实际 GitHub 私有仓库验收也可独立于 Grok Bot 开展。

Grok Bot 接入方式参考 [Grok Bot 自定义连接器说明](https://docs.x.ai/grok-bot/computer-and-apps)。

# MCP 保险库写入首版验收

日期：2026-11-17。范围：[功能设计](mcp-vault-write-permissions.md)。实现及验收均在当前工作区完成，未推送、未替换已安装客户端、未修改用户真实保险库或授权设置。

## 交付内容

- `vault_write_capabilities`：查询写入开关、文件夹、字段和认证方式及 OCI 限制。
- `items_create` / `items_update`：新增、按 ID 局部修改、移动；修改必须携带 `expectedRevision`，模块更新使用稳定 ID。
- 设置 → Agent 接入 → 保险库写入权限：默认关闭，支持已有/新文件夹、移除立即撤权、关闭保留名单。
- 文件夹权限只约束 MCP；客户端手动编辑保持独立。原有查询与字段可见性规则保留，不提供 MCP 删除或可见性修改。
- 复用现有锁、原子文件写入、加密、模块校验、SSH/HTTP 执行器；私钥解析提取为共享函数。

保险库格式升级为 13，兼容读取 7–12；首次保存写入新格式，旧程序会拒绝新格式。旧模块取得稳定 ID，新增模块持久保存 UUID；项目版本随实际修改递增，列表排序不改变项目版本。不要用旧程序覆盖升级后的保险库。

OCI 仅允许已有项目的普通信息和附加模块写入。创建/转换 OCI、区域和身份信息、签名私钥、签名认证方式及派生 URL 不开放；普通 SSH 私钥不会替换 OCI 签名私钥。

## 实际执行结果

| 检查 | 结果与覆盖 |
| --- | --- |
| 完整 Rust/GUI 测试 | `cargo test --manifest-path src-tauri/Cargo.toml`：181 通过，0 失败；包含现有浏览器、备份、OCI 和新增写入回归。 |
| 构建 | 无 GUI 与默认桌面 `cargo build` 均通过。实际 stdio 验收也用最终桌面构建运行。 |
| MCP stdio + HTTP | `python3 scripts/test-mcp-write.py`：通过。真实 JSON-RPC 初始化、工具发现、默认拒绝、授权后新增/修改、隐藏值不出现在查询和写入结果、Bearer/API Key/Basic 请求头、Token 替换后实际使用新值、旧版本冲突、越权移动整次回滚、畸形参数不回显秘密、已连接进程即时撤权、落盘秘密加密。 |
| 本机 SSH | `saved_ssh_credentials_authenticate_using_selected_mode_on_loopback`：通过。密码与私钥同时保存，切换认证方式后分别连接隔离 SSH 服务，服务端确认私钥/密码认证，客户端收到模拟输出及退出码 0；服务端不执行系统命令。 |
| 私钥关联规则 | 加密私钥缺少/错误口令拒绝；旧口令不能代替同请求新口令；正确组合保存、仅改口令验证、无口令替换清除旧口令、失败整次回滚均通过。 |
| 权限、并发和 ID | 默认/名单/双向移动、客户端不受限、重复文件夹拒绝、旧数据 ID 稳定、备份保留 ID、模块跨项目拒绝、并发仅一个成功、手动修改令旧版本失效、秘密可见性保持等回归通过。 |
| OCI 隔离 | 普通附加字段更新成功，签名配置及密钥内容不变；类型/认证/派生 URL 修改拒绝且版本不变。保留现有 OCI 回归。未调用真实 OCI 账户。 |
| 设置 UI | `node scripts/test-agent-write-ui.mjs`：通过。浏览器实际操作开关、添加/移除、关闭保留名单、空名单提示、空/重复名称拒绝，并截图检查。 |
| 原有客户端 UI | `node scripts/test-personal-ui.mjs`：通过，包含编辑、草稿、网站匹配、导入、密码历史、回收站和窄窗口布局。 |
| 差异检查 | `git diff --check` 通过。 |

## 审查修复与执行说明

独立审查发现并修复两项问题，均新增回归测试：

1. 自定义字段连续改名按原始秘密快照处理，并按最终模块集合清理旧名称，避免串值、丢值或把隐藏值转移到可见字段；改名首尾空白统一处理。
2. 客户端删除原 SSH 认证凭据时，保存逻辑回退到现有可用认证方式，避免保留失效选择；MCP 显式认证仍须先通过凭据校验。

初次完整测试因沙箱不允许回环端口监听而失败，允许本机监听后完成验收。另有两项原有 Agent 探测测试假定本机未安装 OpenCode，已调整为比较环境基线；未修改产品探测逻辑。

动态测试全部使用临时保险库、本机端点和模拟凭据。这里证明的是本地首版行为及现有回归通过，不代表真实 SSH 主机或 OCI 账户已完成业务登录验收。

## 复现

```sh
cargo test --manifest-path src-tauri/Cargo.toml
cargo build --manifest-path src-tauri/Cargo.toml
python3 scripts/test-mcp-write.py
npm run test:agent-write-ui
npm run test:personal-ui
```

需要可用的 Rust 工具链、Playwright 浏览器，以及允许本机回环监听的测试环境。当前机器 Rust 使用 `/tmp/openkeylet-cargo`、`/tmp/openkeylet-rustup`，Playwright 浏览器使用 `/tmp/openkeylet-playwright`；其他机器按自己的安装位置设置环境即可。stdio 脚本可通过第一个参数指定构建产物路径。

使用时运行新构建，在设置中显式开启并授权文件夹；已运行的旧 MCP 进程需要重新连接才能加载新增工具。授权后的开关或名单变更无需重连即生效。

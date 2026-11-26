<p align="center">
  <img src=".github/assets/openkeylet-hero.svg" alt="OpenKeylet" width="100%" />
</p>

<h1 align="center">OpenKeylet</h1>

<p align="center"><strong>为 AI Agent 提供本地密码库与凭据管理。</strong></p>

<p align="center">
  <a href="../../releases/latest"><strong>下载</strong></a> ·
  <a href="README.md">English</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="SECURITY.zh-CN.md">安全说明</a>
</p>

OpenKeylet 将密码、Passkey、SSH 私钥、API 凭据和一次性验证码种子保存在本机加密库中。你可以在桌面应用和 Chromium 扩展中使用这些凭据，也能通过本地 `stdio` MCP 让 Agent 登录、连接服务器或调用 API。隐藏字段由 OpenKeylet 在本机使用，不会通过正常 MCP 响应返回给 Agent。

OpenKeylet 免费开源，支持 Windows、macOS 和 Linux，无需注册 OpenKeylet 账号。

## 界面预览

![OpenKeylet 项目列表，展示示例登录、SSH、API、信用卡、身份资料和安全笔记](.github/assets/readme-vault.png)

<table>
  <tr>
    <td width="50%" valign="top">
      <img src=".github/assets/readme-agent-setup.png" alt="Agent 连接设置，展示示例连接状态" /><br />
      <strong>Agent 连接。</strong> 在设置中注册或修复本地 MCP 配置。
    </td>
    <td width="50%" valign="top">
      <img src=".github/assets/readme-ssh-trust.png" alt="SSH 编辑器在项目保存前显示尚未验证的候选指纹" /><br />
      <strong>SSH 主机身份。</strong> 项目保存前即可读取并核对候选指纹。
    </td>
  </tr>
</table>

截图使用当前界面和合成测试数据。

## 快速开始

1. [下载对应平台的版本](../../releases/latest)并打开。
2. 点击 **添加**，保存一个登录、SSH 或 API 项目，并为它起一个唯一的名字。
3. 在 **设置 → Agent 连接** 中点击对应客户端的 **Connect**，然后开启新的 Agent 会话。应用路径变更后可点 **Repair**。
4. 点击项目行上的 **Use** 复制给 Agent 的使用提示，或直接在对话中写出项目名。

支持配置 Antigravity、Claude Code、Codex、Cursor、DeepSeek Harness、OpenClaw 和 OpenCode。其他支持本地 `stdio` MCP 的客户端可以手动配置。MCP 进程按需启动。

| 要做的事 | 可以这样告诉 Agent |
| --- | --- |
| 检查服务器 | `使用 OpenKeylet MCP 中的「Staging Gateway」，查看磁盘使用情况。` |
| 调用已认证的 API | `使用 OpenKeylet MCP 中的「Release API」，列出 owner/repo 的最近提交。` |
| 填写登录 | `打开演示控制台，需要登录时使用 OpenKeylet MCP 中的「Atlas Console」。` |

## 能做什么

### 管理密码、Passkey 和双重验证

用文件夹、收藏和搜索整理登录、信用卡、身份资料、安全笔记以及自定义字段。可以生成密码、查看加密保存的密码历史，以及从回收站恢复项目。

Chromium 扩展通过本地浏览器桥接使用同一个密码库。它可以填充网站登录、信用卡和身份资料，也可以在保存前让你审核捕获的登录信息。扩展弹窗按照网站匹配规则填充。支持的 Passkey 可在确认后创建或用于登录，私钥保存在密码库中。

可从本地图片或剪贴板离线导入 2FA 设置二维码。扩展也能扫描当前页面上的二维码，预览并确认后保存到匹配的登录项目，再自动填充当前验证码。

### 连接服务器和执行命令

SSH 支持密码、私钥和用户证书。可以直连，也可以指定一个已保存的跳板项目，每一跳使用自己的凭据。Agent 可以通过 SSH 执行命令、传入标准输入、读取退出码和输出，以及通过 SFTP 传输文件和目录。命令可能已经启动但连接中断时，OpenKeylet 可以报告远端结果未知。

新建或修改 SSH 项目时，打开 **本机主机身份信任**，保存前即可读取候选指纹。通过可信来源核对并确认后，保存项目会一并写入本机信任。已有项目的连接路径未变时，可立即更新本机信任。[SSH 跳板](docs/ssh-jump.md) · [证书工作流](docs/certificates.md)

### 调用 API 和在本地使用凭据

已保存的凭据可用于 HTTP 请求、本地命令和托管终端。OpenKeylet 在本机解析 `{{openkeylet:模块名}}` 引用。支持的 MCP 工作流会显示实际选用的项目、下一条可调用的请求，以及缺少的参数和分页方式。

原生工作流覆盖 GitHub 提交、OCI 区间与运行实例、AWS EC2 运行实例、Cloudflare 站点与 DNS、DigitalOcean Droplets 和 Hetzner 服务器。其他 API 可走通用 HTTP。证书签发和密码库写入需要单独授权。[API 支持](docs/api-provider-integration.md) · [MCP 密码库写入权限](docs/mcp-vault-write-permissions.md)

### 迁移和备份密码库

| 数据来源 | 支持方式 |
| --- | --- |
| 其他密码库与浏览器 | Bitwarden JSON、Chrome 或 Edge CSV、KeePass 2.x XML、1Password 1PUX 或 CSV |
| 开发工具配置 | `.env`、OpenSSH 配置、cURL 命令、AWS CLI 凭据配置 |
| 另一台 OpenKeylet 设备 | 受密码保护的 `.mvault` 导出，或自己私有 GitHub 仓库中的加密快照 |

写入前可以预览导入内容。导入 `.env` 后，可以为本批实际保存的变量生成绑定到对应项目的模板，模板用占位符代替原始值。

GitHub 备份采用手动加密快照和独立恢复密钥。恢复前可以预览，确认后合并。若云端快照已变更，需要重新预览。

## 凭据控制

| 控制项 | 作用 |
| --- | --- |
| Agent 可见性 | 已启用项目的隐藏模块可供 OpenKeylet 在本机使用。开启模块可见性后，MCP 可以返回该模块的值。 |
| 项目启用开关 | 停用项目后，Agent 无法使用它。 |
| 本机 PIN | 锁定桌面和扩展的密码库访问。桌面锁定时，已启用的 MCP 操作仍可继续。 |
| SSH 主机身份 | 已有信任记录会参与校验，跳板路径要求信任各端点。允许直连未信任主机的开关默认关闭。 |

密码库使用 XChaCha20-Poly1305 加密秘密字段。OpenKeylet 以当前操作系统用户的权限运行，已启用的项目可以执行所请求的操作，包括在远端执行命令。使用敏感系统前，请阅读[安全边界](SECURITY.zh-CN.md)。

## 下载

| 平台 | 软件包 |
| --- | --- |
| Windows x64 | 便携 ZIP |
| macOS arm64 | 包含 macOS 应用的 ZIP |
| Linux x64 桌面版 | 包含 AppImage 的 tar 包，桌面输入支持 X11 |
| Linux x64 Headless | 包含 CLI 和 MCP 程序的 tar 包，无 WebView 依赖 |

<details>
<summary>手动配置 MCP、连接浏览器和升级旧版</summary>

执行以下任一命令获取本机 MCP 配置：

```text
openkeylet config stdio-json
openkeylet config stdio-toml
```

在客户端配置中使用已安装程序的绝对路径，参数设为 `mcp stdio`。无桌面的 Linux 环境可参考 [Headless CLI 指南](docs/headless-admin-cli.md)。

使用浏览器扩展时，在设置中启动 **浏览器桥接**，根据提示安装并配对随应用提供的扩展。使用期间保持桌面程序和桥接运行。

OpenKeylet 以前叫 KRU。新版继续使用原有本地保险库，并支持旧版 `{{kru:模块名}}` 引用。从 KRU 升级后，可在 **Agent 连接** 点 **Repair**，然后开启新的 Agent 会话。

</details>

## 从源码构建

安装 Rust、Node.js 22+ 和 [Tauri 平台依赖](https://v2.tauri.app/start/prerequisites/)。运行 `npm ci` 安装 JavaScript 依赖，`npm test` 运行 Rust 测试，`npm run dev` 启动桌面应用。各平台的打包步骤见[发布工作流](.github/workflows/release.yml)。

[MIT 许可证](LICENSE)

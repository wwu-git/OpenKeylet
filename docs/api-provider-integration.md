# 按 API 服务识别与认证

## 使用边界

服务 URL 用来识别具体 API 服务和认证协议，不能替代凭据、区域、IAM 权限或 OAuth 授权。OpenKeylet 负责目标校验、本地认证准备、签名和一次请求的执行。Agent 读取官方文档，选择接口、方法、业务参数、API 版本、过滤条件及分页。新操作通常不需要修改 OpenKeylet；新认证协议或端点签名规则需要适配。

本次首批为 AWS、现有 OCI，以及 GitHub、Cloudflare、DigitalOcean、Hetzner 的指定控制面 API。其他厂商的认证差异见[官方文档调研](api-provider-research.md)。代码支持和本机模拟验证不代表真实账户、网络或 IAM 验收。

## 当前接入范围

| 服务 | 认证和地址 | 当前能力 |
| --- | --- | --- |
| AWS | 标准商业区端点；Access Key ID、Secret Access Key、可选 Session Token；SigV4 | 本地签名，支持 EC2、区域 STS、SSM、Secrets Manager、CloudWatch Logs、CloudWatch、ELB、KMS、DynamoDB，以及全球 IAM |
| OCI | 已支持 OC1 区域服务端点；tenancy/user OCID、指纹、RSA 私钥 | 复用现有通用 RSA 请求签名；校验区域、主机、必要头和实际正文摘要 |
| GitHub REST | `https://api.github.com/`，已有 Token | Bearer；缺省补 `Accept: application/vnd.github+json` 和 `X-GitHub-Api-Version: 2022-11-28`，保留 Agent 显式版本；分页读 `Link` |
| Cloudflare 控制面 | `https://api.cloudflare.com/client/v4/`，API Token | Bearer，或原有显式自定义认证头；资源 ID、权限和分页仍由具体操作决定。Origin CA Key 不自动变为 Bearer |
| DigitalOcean 控制面 | `https://api.digitalocean.com/v2/`，已有 Token | Bearer；Agent 处理 `links.pages` 等操作定义的分页；不含 Spaces |
| Hetzner Cloud | `https://api.hetzner.cloud/v1/`，项目 Token | Bearer；Agent 读取 `meta.pagination`；不含 Robot、Storage Boxes 和旧 DNS API |

AWS 端点表为九类区域服务与全球 IAM。区域服务使用 `<service>.<region>.amazonaws.com`，CloudWatch 的服务名为 `monitoring`，ELB 为 `elasticloadbalancing`。`iam.amazonaws.com` 的签名区域固定为 `us-east-1`，保存的区域字段也须一致。请求可切换至同一保存区域内表中其他服务；跨区域必须另行配置项目。具体允许区域为代码中维护的官方商业区列表。

AWS 首版不支持 S3/Spaces、SigV4a、预签名 URL、流式分块签名、China/GovCloud、FIPS/dualstack/private/custom endpoints、`.api.aws` 或全球 STS。S3 的路径规范化规则不同，不能套用普通签名。不会读取环境变量凭据、IMDS、系统 AWS CLI，不自动 AssumeRole、获取或刷新 STS Token。`ASIA` 前缀的临时凭据必须有 Session Token。

Azure ARM、GCP Compute/Resource Manager 仅保留已有 access token 的显式 Bearer 用法，并要求保存与目标属于同一识别出的服务商。它们不会自动获取、换取或刷新 OAuth Token，也不接受把服务账号 JSON/PEM 作为 Token。URL 单独查询仍明确标为原生 OAuth 流程未支持。阿里云 ECS、腾讯云、华为云、OVH 等已知但未实现的签名服务会返回明确错误，不尝试 Bearer 降级。

## 客户端配置 AWS

1. 项目页添加项目，选择「AWS API」模板。
2. 模板添加普通 URL 和自定义字段模块，默认区域和 EC2 地址为 `us-east-1`。修改区域时也要修改 URL。所有字段继续支持可见性、显示、复制、删除和排序。
3. 填写 Access Key ID、Secret Access Key；使用临时凭据时填写 Session Token。凭据默认对 Agent 隐藏，区域默认可见。
4. HTTP 认证选择 `AWS SigV4`，展开「AWS 签名模块」检查字段绑定。绑定使用模块 UUID，自定义字段改名或排序不会破坏绑定。没有自动按相似名称寻找替代模块。
5. 点击保存后生效。关闭保留加密草稿，不提交到原项目；配置不完整可以保存，但请求会报告缺少的字段。

模板只帮助组合字段，不创建 AWS 凭据或提升权限。服务地址目录只填写地址，不悄悄替换认证、秘密或字段绑定。AWS 的 CHECK 只检查本机签名配置，不发送请求，也不证明凭据或 IAM 权限有效。旧项目仍为 `auto` 时，CHECK 提示选择 `AWS SigV4` 并在「AWS 签名模块」绑定已有字段。字段名称相同不会自动建立绑定。验证实际云操作时，用 `http_send` 指定文档中的操作。

## MCP 使用

需要确认下一步时，调用只读工具 `action_capabilities`。它返回 `nextCall.tool` 和 `nextCall.arguments`，或返回候选项目、缺少的字段和参数来源。查询不会发送 API 请求或改变会话选中的项目。返回的调用使用项目 UUID，可直接传给对应工具。

例如，从 OCI 区间名查运行中的实例：

```json
{
  "action": "nativeApi",
  "item": "OCI 项目",
  "workflow": {
    "operation": "ociRunningInstances",
    "compartmentName": "app"
  }
}
```

执行返回的 `nextCall`。如果响应提供 `pagination.nextCall`，继续使用它，直到查完所有页。合并各页的 `lookup.matches`。找到唯一匹配后，使用相同的 `itemId` 和匹配的 `compartmentId` 再查一次能力。遇到同名区间时，用候选区间和父级确认目标。

原生流程还提供 `githubCommits`、`awsRunningInstances`、`cloudflareDnsRecords`、`digitalOceanDroplets` 和 `hetznerServers`。通用请求用 `httpRequest`。直接调用 `http_send` 时，默认保留 `body`；使用 `responseFormat: "structured"` 时，完整 JSON 放入 `data`，文本和截断内容仍放在 `body`。远端 HTTP 错误包含状态码和可用的请求 ID。

同一入口也支持 `credentialFill`、`terminalRun`、`terminalOpen`、`terminalSession`、`vaultWrite` 和 `desktop`。桌面流程返回导入或恢复的实际界面步骤。凭据写入流程检查文件夹授权，并给出更新所需的项目 ID 和版本。

新增只读工具 `api_service_capabilities`，可查询已有 `item`，也可只提供 `url`。`items_search` 的 `apiService` 返回同样的服务信息。

返回内容包括服务商、具体服务、认证模式、`ready` / `needsConfiguration` / `unsupported`、缺少的字段、官方文档、使用说明和限制。URL 单独查询只能说明协议支持，不能证明凭据已配置。区域只在对应字段可见时返回；不返回 Access Key、Secret Key、Session Token 或原始隐藏 URL。服务类别及固定文档属于公开能力说明。

Agent 配置 AWS 时沿用写入白名单、版本号和可见性限制。可先创建普通模块，再从返回的模块 ID 通过 `items_update.awsBindings` 绑定角色。字段如下：

| awsBindings 字段 | 绑定模块 |
| --- | --- |
| `serviceUrl` | URL 模块 |
| `region` | 保存区域的秘密字段，可由用户设为 Agent 可见 |
| `accessKeyId` | Access Key ID 秘密字段 |
| `secretAccessKey` | Secret Access Key 秘密字段 |
| `sessionToken` | 可选 Session Token 秘密字段 |

遗漏的绑定在 MCP 修改时保留，显式 null 清除该绑定。不同角色必须引用不同的项目内模块。删除或清空绑定字段不会自动选择另一字段。受保护 CA 项目仍不能通过通用 HTTP/占位符使用。

配置好项目后，Agent 可例如执行 EC2 的 Query API，只需提交业务参数：

```json
{
  "item": "AWS Test",
  "method": "POST",
  "url": "/",
  "form": {
    "Action": "DescribeInstances",
    "Version": "2016-11-15"
  }
}
```

这是手动调用示例。常用的运行实例查询可用 `awsRunningInstances` 生成请求和续页。其他操作可自行提供文档要求的参数，例如 JSON 服务的 `Content-Type` 和 `X-Amz-Target`。AWS 的 POST 可能是查询，也可能修改资源，具体行为取决于操作。

## 目标和传输规则

- 已识别的原生服务只允许 HTTPS:443，无 URL 用户信息或 fragment。原生 Token 项目不允许将凭据发给其他服务商或伪造后缀主机。指定控制面路径也必须匹配。
- AWS 请求区域须与保存的区域一致，签名服务由实际目标端点表解析，不接受 Agent 自报签名服务；OCI 保留现有区域限制。
- 请求最终 URL、头和实际正文构建完成后签名，缓冲正文上限 64 MiB。只支持一种正文表示；签名后不再重新序列化。
- 签名与传输头由程序生成。原生 Token 在存储的自定义认证头注入后也复验，防止通过 `Host` 等头绕过。GitHub 的显式版本/媒体类型及普通业务头保留。
- 原生路径禁用环境/系统代理、重定向和自动重试。3xx 不继续连下一站，不自动跟随分页或下载地址。GitHub 上传等其他主机不因厂商相同而自动放行。
- 生成的认证头和签名、Token 等响应回显会脱敏，包含返回头、正文和保存的响应文件。敏感凭据和派生 HMAC Key 使用零化容器，但不承诺所有 HTTP 库或中间分配的内存副本都被清零。
- 未识别的自定义服务保留既有通用 HTTP 行为，包括其代理和重定向规则。这不是针对同用户任意程序的网络沙箱。

这些规则收紧了已识别服务的历史通用目标行为。跨服务商备用地址不再自动可用；本来就没有自动地址故障转移。原生服务拒绝无法确认的目标，不会悄悄退回通用 Bearer。

## 存储和实现选择

保险库版本升至 18。16/17 升级只补默认字段，保留现有证书 CA 标记、授权、SSH 信任及日志；旧程序会拒绝版本 18。AWS 模块及绑定随已有加密备份保留，并纳入冲突比较和重复恢复去重。不新增凭据仓库或另一套云权限系统。

比较两个设计后，选择小型类型化服务表和 AWS 稳定绑定，合入统一原生响应处理。保留现有 OCI 模型，避免为接入 AWS 重写 OCI 存储。现有 Rust 签名 SDK 最新版最低编译器要求高于项目声明，因此首版用已有 HMAC/SHA256 依赖实现限定的普通 SigV4，并用官方参考签名、botocore 独立数据和实际请求进行交叉验证。不把 S3 的不同规则纳入未经证明的通用实现。

Model the Domain 原则用于服务/执行策略/配置状态类型；Prove It Works 原则要求核对服务器实际收到的请求。两份设计来自继承配置的通用 Agent，未验证异厂商模型差异；安全审核者只读检查代码和日志。

## 本机验收记录（2026-11-24）

验收仅使用临时保险库、模拟凭据和本机 TLS 端点，不访问真实云账户。实现前基线为 `ad35fbb`，281 项通过；实现后 299 项库测试、1 项程序测试、3 项管理 CLI、2 项证书集成及 1 项服务 MCP 集成，共 306 项通过，0 失败。1 项依赖独立 OpenSSH 环境的测试默认忽略。

后续 AWS CHECK 修复新增两项回归，覆盖完整配置、旧 `auto` 项目、空 URL、区域不符、临时 Token 缺失、未支持端点、停用项目及 SSH 检查优先级。修复后共 308 项通过，0 失败，仍有 1 项默认忽略。AWS 标准模块、保存草稿及窄窗口界面回归通过。本地配置检查不替代实际云 API 验收。

| 检查 | 实际结果 |
| --- | --- |
| AWS 签名 | 官方 IAM 参考签名及 5 组独立 botocore 数据通过；本机服务器实际收到的表单、JSON 请求，其 URL、签名头和正文由 botocore 再签名，结果一致 |
| 目标与传输 | 区域不符、伪造主机、HTTP、未支持的 S3 等拒绝；307 不连接 B；设置环境代理后仍不连接代理；存储的 custom/apiKey 认证不能注入 Host 等传输头 |
| 四家 Token 服务 | 本机 TLS 核对认证头、GitHub 缺省头及显式头保留；分页头保留，回显和响应文件脱敏通过 |
| 存储与 MCP | 真实 stdio 发现新工具；AWS 稳定模块绑定、隐藏字段、版本冲突、即时撤权、备份与重复恢复比较通过；现有 MCP 写入脚本全项通过 |
| 界面 | 隔离渲染测试验证 AWS 标准模块、改名后绑定、保存与加密草稿、690/460px 布局；现有 OCI、服务目录及证书界面回归通过。人工查看窄窗口截图，绑定控件和说明完整显示 |
| 构建 | 默认桌面配置和 `--no-default-features` 无界面配置的本机 debug 构建成功；未另行验证 Rust 1.89 工具链或跨平台构建 |
| 审核与隐私 | 只读安全复审完成；修改文件和提交范围脱敏扫描仅命中模拟 AWS 测试签名，人工确认为固定测试数据；未新增实际凭据或账户资源信息 |

复验入口：

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo build --manifest-path src-tauri/Cargo.toml --locked
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --locked
python3 scripts/test-mcp-write.py src-tauri/target/debug/openkeylet
node scripts/test-inline-editor.mjs --aws-provider
node scripts/test-inline-editor.mjs --oci
node scripts/test-inline-editor.mjs --api-services
node scripts/test-certificates-ui.mjs
```

独立签名核验需在单独 Python 环境安装 `botocore`（本次版本为 `1.44.10`）。设置 `OPENKEYLET_AWS_ORACLE_DIR` 后运行 `aws_final_wire_form_and_json_are_signed_and_echoes_redacted` 测试，随后运行 `python scripts/aws-signature-oracle.py --verify-wire "$OPENKEYLET_AWS_ORACLE_DIR"`。普通 Rust 测试直接使用已提交的独立数据，无需 Python 依赖。UI 测试需可用 Chrome/Playwright，本次指定 `OPENKEYLET_TEST_CHROME=chrome`。

布局验收以实际滚动容器、页面和交互控件边界为准；装饰性文字倾斜可能使段落自身 `scrollWidth` 多出 2px，不将该值单独判作内容遮挡。完整测试不证明已安装应用或当前聊天加载了新工具。本次未安装、未推送；上线前仍需用户自己的最小权限账户验证实际服务端响应、IAM 范围及临时凭据有效期。

## 官方依据

- [AWS 请求签名](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html)、[AWS 端点](https://docs.aws.amazon.com/general/latest/gr/rande.html)、[EC2 Query API](https://docs.aws.amazon.com/ec2/latest/devguide/Query-Requests.html)、[S3 签名例外](https://docs.aws.amazon.com/AmazonS3/latest/developerguide/sig-v4-header-based-auth.html)。
- [OCI 请求签名](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/signingrequests.htm)。
- 其余服务的认证、生命周期、版本和分页来源见[调研表](api-provider-research.md)。

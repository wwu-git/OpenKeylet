# 常见服务商认证调研

核对日期为 2026-11-24。仅查阅官方文档，未调用任何账户 API，未读取凭据。下表是接入建议，不是声称当前代码已经支持。AWS/OCI 的独立调研由父任务完成，本文件覆盖其余十家。

## 服务和协议矩阵

| 服务范围 | 官方 API 地址示例 | 认证材料与传输 | 请求规则 | 首批建议 |
| --- | --- | --- | --- | --- |
| GitHub REST | `https://api.github.com/` | 已有 PAT、安装令牌或 OAuth access token 放入 `Authorization: Bearer`；GitHub App 私钥不是 access token | JSON media type、明确 API 版本；操作按路径、方法和参数选择 | 复用 Bearer，加服务元信息、默认媒体类型/版本及精确 API 主机边界；不做 App JWT/安装令牌自动换取 |
| Cloudflare control plane | `https://api.cloudflare.com/client/v4/` | 推荐 scoped API token 的 Bearer 认证 | 需要正确的 account/zone ID、令牌权限和各操作参数 | 复用 Bearer；不将全球 API Key 或 Origin CA Key 当作 Bearer token |
| DigitalOcean control plane | `https://api.digitalocean.com/v2/` | scoped PAT 或 OAuth access token 的 Bearer 认证 | JSON API，按路径调用 Droplet 等资源 | 复用 Bearer，仅覆盖该控制面；不包含 Spaces、Inference 或客户专属 Agent 地址 |
| Hetzner Cloud | `https://api.hetzner.cloud/v1/` | 项目 API token 的 Bearer 认证 | 项目边界由 token 决定；Read token 只能 GET | 复用 Bearer，仅 Cloud API；不把 Robot、Storage Box 或旧 DNS API 一并识别为同一服务 |
| OVHcloud control plane | EU `https://api.eu.ovhcloud.com/1.0/` 等分区端点 | Application Key、Application Secret、Consumer Key；请求签名和 `X-Ovh-*` 头 | 方法、完整 URL、精确 body、时间戳均进入签名 | 研究并明确标为待实现，不能推断成普通 Bearer；OpenStack 公有云 API 又是另一套身份流程 |
| Azure ARM | `https://management.azure.com/` | Entra access token；client credentials 需 tenant、client ID、secret/证书/联合身份、资源 scope | API 路径需订阅/资源组等标识，查询参数 `api-version` 依操作决定 | 标记需要 OAuth 生命周期；已有 access token 可按手动 Bearer 使用，不声称完成自动认证 |
| GCP Compute | `https://compute.googleapis.com/compute/v1/` | OAuth access token；服务账号私钥需先生成签名 JWT 再换 token，或采用 ADC/工作负载身份等 | project、zone、资源路径、scope/IAM；并非任意 Google API 都接受相同 token/audience | 标记需要 Google OAuth/凭据源；不把服务账号 JSON 或 API Key 当作 Compute Bearer token |
| 阿里云 ECS OpenAPI | `https://ecs.<region>.aliyuncs.com/` | AccessKey ID/Secret，临时凭据还需 SecurityToken；ACS3-HMAC-SHA256 | action/version、UTC 日期、nonce、内容摘要、规范化路径/查询/头 | 待专用 signer，不能把 ECS URL 自动转成 Bearer |
| 腾讯云 CVM API 3.0 | `https://cvm.tencentcloudapi.com/` | SecretId/SecretKey，临时凭据还需 Token；TC3-HMAC-SHA256 | `X-TC-Action`、`X-TC-Version`、`X-TC-Region`、时间戳以及确切 body | 待专用 signer，不能仅把 AWS signer 换名字 |
| 华为云 ECS 控制面 | `https://ecs.<region>.myhuaweicloud.com/` | AK/SK 的 SDK-HMAC-SHA256，或 IAM token 的 `X-Auth-Token` | 区域/项目 ID；部分 global/multi-project 请求还需 domain/project 头 | 待专用 signer或受控 IAM token交换，不推断成 `Authorization: Bearer` |

## 首批四种 Token 服务的实际差异

### GitHub

- `Authorization` 的令牌必须已具备目标资源权限。App JWT、installation token、PAT 的生成和作用域不同，识别主机不能解决这些差异。
- 推荐 `Accept: application/vnd.github+json`；必须支持调用者显式指定 `X-GitHub-Api-Version`。当前新版为 `2026-03-10`，旧版 `2022-11-28` 仍受支持且仍是无版本头时的默认。不要因为调研了新文档便静默升级已有项目，新增版本含破坏性变化。
- 多页结果遵循响应 `Link`；参数如 `per_page`、`page` 按具体操作。不要自动跟随任意返回链接并把 token 发往其主机。
- token 过期/撤销和资源权限由 GitHub 决定。首批不自动创建令牌、刷新 OAuth 或提升权限。

来源：[认证](https://docs.github.com/en/rest/authentication/authenticating-to-the-rest-api)、[媒体类型和请求格式](https://docs.github.com/en/rest/using-the-rest-api/getting-started-with-the-rest-api)、[版本](https://docs.github.com/en/rest/about-the-rest-api/api-versions)、[分页](https://docs.github.com/en/rest/using-the-rest-api/using-pagination-in-the-rest-api)。

### Cloudflare

- Bearer 支持 scoped API token；权限、资源范围和有效时间可能受限制。旧全球 Key 的权限和头部方案不同，不应通过名字或 Key 前缀统一转换。
- 通常分页使用 `page`/`per_page`，响应包含 `result_info`；有些接口另有游标，Agent 必须以该操作文档为准。
- Origin CA 证书 API 可接受 Origin CA Key 的 `X-Auth-User-Service-Key` 或合适的 API token。其专用 Key 不能成为整个 Cloudflare 控制面的普通 token。首批仅声称 API-token 模式。
- `account_id`/`zone_id` 是资源上下文，不是另一把秘密，也不能从 API 域名推导。

来源：[调用与分页](https://developers.cloudflare.com/fundamentals/api/how-to/make-api-calls/)、[token 与旧 Key 区别](https://developers.cloudflare.com/fundamentals/api/get-started/)、[旧 Key 限制](https://developers.cloudflare.com/fundamentals/api/get-started/keys/)、[Origin CA API 认证](https://developers.cloudflare.com/api/resources/origin_ca_certificates/methods/get/)。

### DigitalOcean

- 控制面 PAT 使用 Bearer；scoped PAT 能限定资源操作，不应以“能认证”推定“可操作全部资源”。首批复用已有 access token，不实现 OAuth 授权跳转或续期。
- 常规列表以 `page`、`per_page` 及响应 `links.pages` 处理分页，具体限制按操作文档。
- Spaces 是独立的 S3-compatible XML API，使用 Spaces key pair、区域/空间端点以及 SigV4 或兼容 SigV2。`*.digitaloceanspaces.com` 不得落入控制面 Bearer 识别。
- Inference API 同样与控制面独立，不能按 DigitalOcean 品牌通配域名接入。

来源：[PAT 与 scope](https://docs.digitalocean.com/reference/api/create-personal-access-token/)、[控制面 API](https://docs.digitalocean.com/reference/api/reference/)、[列表与分页](https://docs.digitalocean.com/reference/api/reference/public-apis/)、[Spaces 签名](https://docs.digitalocean.com/reference/api/spaces/)。

### Hetzner

- Cloud API token 属于项目，至少区分 Read 与 Read & Write。首批只复用已有 token；不自动创建或提升 token 权限。
- Cloud API 默认列表分页为 25，`per_page` 通常最大 50，具体操作可能另有规定；优先读 `meta.pagination.next_page`，不要依赖将被移除的分页 `Link`。
- `api.hetzner.cloud` 管理 Cloud；`api.hetzner.com` 管理 Storage Boxes；Robot Web Service 管理 Dedicated Servers。识别必须按服务而非仅按公司名称。

来源：[Cloud 认证、分页及限制](https://docs.hetzner.cloud/reference/cloud)、[服务入口边界](https://docs.hetzner.cloud/)、[token 的 Read/Write](https://docs.hetzner.com/cloud/api/getting-started/generating-api-token/)。

## 其余厂商后续接入要求

### OVHcloud

控制 API 使用 AK/AS/CK 和 `$1$` 前缀的 SHA1 摘要签名。摘要材料依次包含 AS、CK、方法、完整 URL、精确 body 和时间戳，字段间有规定分隔符；需要 `X-Ovh-Application`、`X-Ovh-Consumer`、`X-Ovh-Timestamp` 和 `X-Ovh-Signature`。Consumer Key 绑定获批的路径/方法及授权有效期。后续应对固定分区端点签名并处理时钟偏差，不自动申请 CK 或扩大权限。API `/v1`、`/v2` 与具体端点差异由 Agent 查官方操作文档，分页无统一硬编码规则。

来源：[官方首次调用与签名公式](https://docs.ovhcloud.com/en/guides/manage-and-operate/api/first-steps)、[凭据和权利委派](https://docs.ovhcloud.com/en/guides/manage-and-operate/api/api-right-delegation)。

### Azure

ARM bearer token 通常先由 Entra OAuth 2.0 token endpoint 获取。client credentials 请求发送至固定受信任租户身份端点 `https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token`，包含 `client_id`、对应凭据、`grant_type=client_credentials` 和单一资源 `.default` scope。不能把 resource URL 当作 token endpoint。应遵守响应有效期并重新获取，不能把密码/secret塞入目标 API。证书和 federated credential 路径不能仅重用 secret 表单。官方另提醒 ARM 资源 URI 尾斜杠可能要求 `https://management.azure.com//.default`，不能机械去斜杠。Storage Shared Key/SAS、Graph 等不是同一资源 audience。查询版本和 `nextLink` 由各操作定义，下一页仍需目标校验。

来源：[client credentials](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow)、[`.default` 与尾斜杠](https://learn.microsoft.com/en-us/entra/identity-platform/scopes-oidc)、[ARM REST](https://learn.microsoft.com/en-us/azure/azure-resource-manager/management/manage-resources-rest)。

### GCP

服务账号本地私钥方案通常构造签名 JWT 以向 Google OAuth token endpoint 换 access token，再在有效期内复用。部分 API 允许直接 signed JWT，但不是所有 API 的通用规则。后续实现要绑定 credential type、固定 token host、scope/audience、IAM principal，以及过期和 token cache 的行为。应优先支持官方凭据库/明确服务账号方案，不能自动读取云主机 metadata 或调用环境 CLI。Compute list 使用 project/zone，返回 `nextPageToken`，请求使用 `pageToken`。不把服务账号私钥/JSON 作为 Bearer 值。

来源：[服务账号 OAuth 与直接 JWT 的区别](https://developers.google.com/identity/protocols/oauth2/service-account)、[REST 认证](https://docs.cloud.google.com/docs/authentication/rest)、[Compute 列表](https://docs.cloud.google.com/compute/docs/reference/rest/v1/instances/list)。

### 阿里云

ECS OpenAPI V3 使用 ACS3-HMAC-SHA256，除 AccessKey 对外，还需 action、version、日期、nonce、内容 SHA256 和规范请求；临时 STS 凭据另需 `x-acs-security-token`。签名必须覆盖最终发出的字节。响应分页形式依操作，Agent 应读取对应版本的 ECS 文档。官方明确部分自主管理网关服务用不同认证，尤其 OSS、SLS。不能将整个 `aliyuncs.com` 后缀都宣称支持 ACS3，Model Studio Bearer Key 也不同。

来源：[官方 ACS3 语法、STS 和服务例外](https://www.alibabacloud.com/help/en/sdk/product-overview/v3-request-structure-and-signature)。

### 腾讯云

CVM API 3.0 推荐 TC3-HMAC-SHA256。SecretId/SecretKey 通过日期和 service 派生签名，service 必须与产品域名一致；日期来自 UTC 时间戳。请求另有 `X-TC-Action`、`X-TC-Version`、`X-TC-Region`，临时凭据还需 `X-TC-Token`。JSON body 的确切序列化参与摘要。分页可能使用 `Offset`/`Limit` 等业务参数，不能因方法是 POST 就判为写操作，也不能把方法当操作名。COS、旧版 API 等需另查协议，不能匹配整个腾讯云域名族后套 TC3。

来源：[官方 CVM 签名 V3、完整请求及临时 Token 说明](https://cloud.tencent.com/document/product/213/30654)。

### 华为云

控制面 AK/SK 请求使用 SDK-HMAC-SHA256 和 `X-Sdk-Date`；另一模式以 IAM 返回的用户 token 写入 `X-Auth-Token`。作用域还涉及 regional project/global domain，某些场景要求 `X-Project-ID` 或 `X-Domain-ID`。签名规范和 AWS 不同，应使用华为官方规则或 signer。后续 token 路径需固定 IAM 端点并按有效期重新认证，不能自动把 AK/SK 或 token换成 Bearer。ECS path 版本、项目和分页按操作；OBS/S3、APIG AppKey/AppSecret 不等于控制面身份方案。

来源：[请求头、AK/SK 与 token 模式](https://support.huaweicloud.com/intl/en-us/api-cts/cts_api_0007.html)、[官方 AK/SK 签名机制](https://support.huaweicloud.com/eu/devg-apisign/api-sign-algorithm-001.html)。

## 对实现的约束建议

- URL 识别返回“具体 API 服务 + 已支持认证模式 + 所需字段 + 官方操作文档”，不要返回“该厂商全部功能已接入”。
- 标记必须区分“已支持此认证”、“需要配置”和“仅识别但尚未原生支持”。已知需要签名或 OAuth 的服务不得悄悄用普通 Bearer 尝试。
- 精确验证 scheme、host、port、path/service/region 等边界；不按品牌子串或凭据前缀授予目标权限。`api.github.com.attacker.invalid`、userinfo、非 HTTPS、跨服务地址均需负例。
- Provider 决定认证和传输约束，Agent 决定 API 操作、版本、过滤、分页与结果解释。保留业务 action/version 及请求头，不添加固定“列服务器”的动作引擎。
- 授权 token 权限不足、过期、时钟偏差和 API 版本错误应分别说明，不把 HTTP 401/403 等同于私钥无效，更不能为修复认证而自动获取更高权限。
- 新增或变化的 API 操作只需 Agent 阅读官方文档；仅出现新的认证协议、端点/签名规则或 token 生命周期时才需要 OpenKeylet adapter 更新。

验证边界为官方文档研究。本文件不证明真实服务账户认证、网络连通、IAM 权限或代码测试已通过。

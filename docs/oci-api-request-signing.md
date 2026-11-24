# OCI API 签名代执行

OpenKeylet 通过现有 `http_send` 执行 OCI API 请求。Agent 根据用户意图和 OCI 官方文档选择接口、方法、参数及操作步骤。OpenKeylet 保管凭据，在本机校验目标、构造签名、发送请求并返回结果。

## 能力和职责

OCI API 项目提供通用的 API v1 RSA-SHA256 请求签名，不再限定 `ListInstances`、GET、路径或查询参数。新增 OCI API 只要沿用已支持的签名协议和请求格式，Agent 就可以直接构造请求，不需要新增专用 MCP 工具。

| Agent | OpenKeylet |
| --- | --- |
| 阅读接口文档，选择方法、路径、参数和正文 | 验证目标和传输参数，签名最终请求 |
| 查找区间和资源，处理同名结果、分页和状态筛选 | 发送一次请求，返回状态、响应头和正文 |
| 在用户授权范围内决定执行步骤 | 隐藏已保存的凭据和签名认证头 |

OCI IAM 决定身份实际可以访问的资源和操作。权限拒绝是服务结果，OpenKeylet 不切换身份、扩大权限或绕过它。通用执行能力不等于任意 OCI 身份都具有写入权限。CHECK 是独立的轻量探测，不限制 `http_send` 的业务操作。

## 项目配置与发现

桌面端的 **OCI API** 模板保留区域、租户 OCID、用户 OCID、公钥指纹和 RSA 签名私钥。选定区域后，默认服务 URL 为 `https://iaas.<region>.oraclecloud.com/20160918/`。保存项目后配置和私钥才生效。现有 OCI Compute 项目继续使用，无需迁移或重新导入密钥。

Agent 使用 `items_search` 获取非秘密的 `ociContext.region` 和 `ociContext.tenancyOcid`。租户 OCID 同时是根 compartment 的 OCID，可用于构造区间发现请求。此上下文不包含用户 OCID、公钥指纹或签名私钥。普通模块仍遵守各自的 Agent 可见性开关。

项目可附加普通账号、密码、Token 或自定义字段。Agent 可以用现有 `{{openkeylet:模块名}}` 占位符引用普通秘密模块。OpenKeylet 在本地替换后签名，并对返回内容脱敏。独立的 OCI 签名私钥不能通过模块、占位符、通用填写或终端环境变量读取。

## 请求和目标边界

- 使用现有 `http_send` 的 URL、方法、查询、请求头和正文参数。一次请求只选择 JSON/文本 `body`、原始字节 `bodyBase64`、或 `form`/`files` 中的一种正文形式。
- 空 URL 或相对路径使用默认 Compute 服务地址。访问 IAM、Object Storage 或其他 OCI 服务时，Agent 提供该服务的绝对 URL。
- CONNECT 和 TRACE 通道方法不受支持，其他有效 HTTP 方法交给 OCI 服务处理。目标必须为配置区域下的 OCI 商业云 HTTPS 端点，有效端口为 443。支持 `.<region>.oraclecloud.com` 和 `.<region>.oci.oraclecloud.com` 区域后缀，前面的服务域名须合法；不维护服务名或业务路径白名单。其他区域、其他域名、IP、URL 用户信息和片段均拒绝。
- 保留正常 TLS 证书验证。OCI 请求不使用系统代理或代理环境变量，不跟随同源或跨源重定向，失败不回退到其他地址。
- 签名及传输所需的 Host、Authorization、Date/X-Date、正文摘要和长度由本机生成，调用方不能覆盖。Agent 可以提供 API 所需的其他头，例如 Content-Type、条件请求头和请求 ID。
- 最终序列化后的方法、路径、查询顺序和正文参与签名。带正文的请求，以及 POST/PUT/PATCH 的空正文，包含正文 SHA-256、类型和长度。签名后不再调整已签名的请求字节。
- 正文在本机完成序列化和签名，缓冲上限为 64 MiB。超过上限时不发送，不提供流式分块签名。S3 Signature V4、OAuth、实例主体等其他认证协议不属于此 RSA API 密钥模式。

## 响应和后续调用

`http_send` 返回状态、响应头和正文。Agent 根据对应 API 的文档解释响应、错误和下一步，不由 OpenKeylet 固定业务流程。OCI 响应默认最多读取 2 MiB，可通过正值 `maxResponseBytes` 调整。使用 `saveResponseTo` 时也先完整读取、脱敏，再原子保存到本机文件；超过指定响应上限时失败，不留下部分文件。响应和文件中的已知秘密及签名认证头均受脱敏保护。

每次调用只发送一个请求。列表 API 返回 `opc-next-page` 时，Agent 把令牌原样作为下一次请求的 `page`，保留原有筛选条件，不自行预编码。空列表也可能带下一页令牌，不能仅凭正文为空停止。异步操作按该 API 的响应头和文档继续查询，不能把一次请求接受等同于操作完成。

查找名为 `example-compartment` 的区间时，Agent 应先查询有权访问的区间并取得其 OCID。唯一匹配可以直接继续；同名或路径仍有歧义时才请用户选择。能够自行查询的 OCID 不要求用户反复粘贴。根区间的空实例列表也不能解释为整个租户没有服务器。

## CHECK

项目保存且已导入私钥后，项目页启用 CHECK。CHECK 固定向配置区域发送一次根 compartment 的 `ListInstances`，`limit=1`、30 秒超时和受限响应体积。不读取下一页，不显示实例正文。

只有 HTTP 2xx 算成功。失败返回状态及可用的请求 ID。仅获准访问子区间的身份可能被根区间拒绝，这不一定表示私钥无效。CHECK 不修改通用 `http_send` 的参数或权限。

## 私钥、保存和备份

私钥支持文件、剪贴板和手动输入，共用校验及暂存。只接受未加密 RSA PEM，支持 PKCS#1、PKCS#8、单个开头 UTF-8 BOM 和 Oracle `OCI_API_KEY` 尾标记，至少 2048 位并满足底层 RSA 参数校验。取消、无效替换或保存失败保留原私钥。

有效私钥在保存项目时加密入库，只由本地签名器使用。客户端没有私钥明文查看、复制或单独导出入口。MCP、错误和活动记录不返回私钥或签名认证头。

便携导出、密码保护导出和 GitHub 快照均排除 OCI 签名私钥，保留非秘密配置。恢复后没有本地私钥的项目需重新导入，才能签名或 CHECK。用户需独立保管原始 PEM。

## MCP 使用示例

以下是 Agent 可选择的调用示例，不是 OpenKeylet 内置的固定流程。实际参数和权限以相应 OCI API 文档为准。

先按项目名发现上下文。

```json
{
  "query": "oci-readonly-example"
}
```

读取返回的 `ociContext`，使用其区域和租户 OCID 构造 IAM 区间查询。

```json
{
  "item": "oci-readonly-example",
  "method": "GET",
  "url": "https://identity.<region>.oci.oraclecloud.com/20160918/compartments",
  "query": {
    "compartmentId": "<ociContext.tenancyOcid>",
    "compartmentIdInSubtree": true,
    "accessLevel": "ACCESSIBLE",
    "lifecycleState": "ACTIVE",
    "limit": 100
  }
}
```

Agent 根据名称和父区间确认 `example-compartment` 的唯一 OCID，再调用实例 API。

```json
{
  "item": "oci-readonly-example",
  "method": "GET",
  "url": "instances",
  "query": {
    "compartmentId": "<app compartment OCID>",
    "lifecycleState": "RUNNING",
    "limit": 100
  }
}
```

如 API 支持对应的状态筛选，可以直接提交参数；否则 Agent 在返回结果中筛选。接口版本、路径和查询参数不由 OpenKeylet 维护操作名单。

## 验收

本机模拟仅使用合成密钥。测试连接层把经过正式目标校验的请求导向本机 TLS 端点，不在正式目标范围中加入回环地址。

- 用独立参考实现校验无正文、空正文及 JSON、文本、原始字节、表单和文件的实际方法、路径、查询、摘要、长度和 RSA 签名。
- 验证新路径和查询参数可以直接调用；重复参数及分页令牌保持请求语义。外域、伪后缀、错误区域、非 HTTPS、端口、用户信息、片段和签名头覆盖在发送前失败。
- 验证响应头、错误和文件结果不会暴露已知秘密、签名或认证头；代理和重定向无回退请求。
- MCP 发现只暴露非秘密 OCI 上下文，普通模块遵守可见性；附加秘密可本地插值，签名私钥不能绑定或读取。
- 检查中英文模板、保存取消、CHECK、无私钥恢复和既有 HTTP/SSH 回归。
- 真实验收使用专用只读项目，查询区间后读取目标区间实例。IAM 拒绝和区间歧义须如实报告，不用模拟通过代替真实结果，不执行云端资源写入。

2026-11-24，新构建的独立 MCP 进程使用只读测试项目（文中匿名为 `oci-readonly-example`）完成只读实测。Agent 从 `ociContext` 取得请求上下文，查询到可访问区间，唯一匹配目标区间，再查询到 `RUNNING` 实例（真实名称与资源数量已省略）。区间与实例请求均返回 HTTP 200，响应未截断且没有下一页。此次没有修改云资源、IAM 权限或保险库项目。

## 参考

- [Oracle OCI 请求签名规范](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/signingrequests.htm)
- [OCI REST API、区域接入点与分页](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/usingapi.htm)
- [Oracle OCI 商业云区域列表](https://docs.oracle.com/en-us/iaas/Content/General/Concepts/regions.htm)

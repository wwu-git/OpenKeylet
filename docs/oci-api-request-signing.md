# OCI API 请求签名设计

状态：首版已实现。真实 OCI 只读联调需要部署方提供已配置的专用只读项目和用于验收的 compartment。

OpenKeylet 将为 OCI 用户 API 密钥提供显式的「OCI API 签名」认证方式。Agent 继续使用现有的 `http_send`，由 OpenKeylet 在本机校验、签名并发送请求。

## 首版范围

首版只支持 OCI 商业云 Compute 的 **`ListInstances`（`GET /20160918/instances`）**。Compute 和 Networking 可能共用入口，因此同一主机上的其他路径也不允许。

OCI 地址模板只提供服务地址。用户需明确选择「OCI API 签名」，再从受控的 OCI 商业云 OC1 区域列表中选择一个区域。其他 Compute 操作、Networking、Object Storage、S3 兼容接口和实例主体认证留待后续。

## 添加项目模板

在桌面端「添加项目」的模板区增加 **OCI Compute**，说明显示「使用 OCI API 密钥，仅支持读取实例列表」。

选择模板后，项目进入显式的 OCI API 签名配置。表单提供区域、租户 OCID、用户 OCID、公钥指纹和「导入 RSA 签名私钥」。选定区域后，自动显示对应的服务 URL，地址由区域决定。项目配置不包含 `compartmentId`，调用方在每次请求中提供它。

现有 OCI 服务 URL 模板仍用于填写地址。「OCI Compute」项目模板与请求签名功能一起上线，且仍需点击「保存项目」才会提交配置和私钥。

## 目标和请求校验

1. 根据项目配置的区域构造 `https://iaas.<region>.oraclecloud.com/20160918/`。不接受自由输入的区域或主机覆盖。检查解析后的最终 URL，其协议、主机和有效端口必须与该 HTTPS 来源一致。绝对 URL 和相对路径使用同一规则。拒绝 URL 中的用户信息和片段。
2. 方法必须是 GET，路径必须精确等于 `/20160918/instances`。拒绝请求正文、Base64 正文、表单和文件上传。合并 URL 查询串与 MCP `query` 参数后，要求恰好一个非空 `compartmentId`。首版仅另外接受可选的 `page` 和 `limit`，各最多出现一次。空 `page`、不符合 OCI 约束的 `limit`、未知参数和重复参数均拒绝。OCI IAM 决定可访问的 compartment，首版不维护本地 compartment 权限名单。
3. 不接受调用方设置的 `Host`、`Authorization`、`Date` 或 `X-Date`，头名称按不区分大小写校验。执行器根据最终请求生成这些签名所需数据。
4. 所有检查在签名前完成。OCI 签名请求使用专用 HTTPS 客户端，保留正常证书校验，禁用系统代理和代理环境变量的自动发现，不跟随任何重定向。直连失败即返回错误，不回退到代理，也不重发到其他地址。现有通用 HTTP 模式的代理策略独立。
5. 使用实际发送的方法、编码后的路径和查询串、主机及请求时间生成 OCI RSA-SHA256 签名。签名与发送使用同一个最终请求，签名后不重新编码或调整查询参数顺序。

## 手动分页

每次 `http_send` 只请求一页。响应保留原始 `opc-next-page` 头。即使响应体是空列表，只要该头存在，就表示还有下一页。调用方保留原有 `compartmentId` 和 `limit`，将该头的值作为下一次请求的 `page` 参数。下一页重新走同一校验及签名流程，不对分页令牌做预编码。缺少 `opc-next-page` 时结束。

## 私钥和备份

项目保存区域、租户 OCID、用户 OCID 和已登记的 OCI API 公钥指纹。首版仅导入未加密 RSA PEM（PKCS#1 或 PKCS#8，至少 2048 位，允许 Oracle 的 `OCI_API_KEY` 文件尾标记），导入时验证密钥格式和类型。加密 PEM、非 RSA 或损坏文件均拒绝，校验失败或取消编辑保留原密钥。有效私钥在保存项目时加密入库。

签名私钥是独立的凭据类型，只供本地 OCI 签名器使用。桌面端可以导入和替换，但不提供私钥明文查看、复制或单独导出。MCP 项目查询、通用填写、占位符和终端环境变量均不能读取或引用它。签名私钥、签名认证头不进入 MCP 响应、活动记录或错误信息。可保留 HTTP 状态、OCI 请求 ID 和错误类别用于诊断。

首版便携导出、密码保护导出及 GitHub 快照均排除 OCI 签名私钥。备份可保留项目的非秘密配置，恢复后没有本地私钥的项目在重新导入密钥前不能签名。用户需独立保管原始 PEM。

## 验收用例

本机测试使用测试专用连接层将请求导向受控监听端点，保留生产目标校验规则，不在正式许可范围中加入回环地址。预期在签名前拒绝的用例同时断言签名器未被调用、请求未发送。

| 场景 | 预期结果 |
| --- | --- |
| 固定 RSA 测试密钥、时钟和 Oracle GET 样例 | 签名与官方样例或独立参考实现一致，覆盖编码字符和多个查询参数。 |
| 捕获正式校验通过的最终请求 | 实际方法、路径、查询顺序、Host 和时间与签名输入一致，签名可用测试公钥验证。 |
| 在「添加项目」中选择 OCI Compute 模板并选择区域 | 显示签名配置字段和该区域的服务 URL。保存前不创建项目或持久化私钥。 |
| 有效区域与同源 `GET /20160918/instances` | 相对路径和绝对 URL 都可通过校验，且仅向配置的来源发送。 |
| 未知区域、不同区域或主机、HTTP、非默认端口、跨源引用、URL 用户信息或片段 | 签名前拒绝。 |
| 用不同大小写传入 Host、Authorization、Date 或 X-Date | 签名前拒绝。 |
| 非 GET 方法、其他 Compute 或 Networking 路径、GET 正文、表单、文件、缺失或重复的 `compartmentId`，以及未知、重复或无效查询参数 | 签名前拒绝。重复用例覆盖同一名称分别出现在 URL 和 MCP `query` 中。 |
| 设置可用系统代理或代理环境变量 | OCI 请求仍直连原目标，代理未收到连接。直连失败时不会尝试代理。 |
| 端点返回同源或跨源重定向 | 返回重定向被阻断的错误，不发起第二次请求。 |
| 列表响应为空，但包含 `opc-next-page` | 返回下一页令牌。调用方发起带 `page` 的新 GET 后，重新校验并签名。没有下一页令牌时结束。 |
| 导入有效未加密 RSA PEM、加密 PEM、非 RSA、损坏 PEM，或取消替换 | 仅有效未加密 RSA PEM 可保存。校验失败或取消保留原密钥。 |
| 查询、填写、复制、通用占位符、终端环境变量，或对项目执行备份与恢复 | 这些路径不能取得签名私钥。备份不含私钥，恢复到无本地密钥的保险库后须重新导入。 |
| 签名失败或 OCI 返回认证错误 | 响应和活动记录不含私钥、签名认证头，OCI 响应带请求 ID 时保留该 ID。 |
| 用专用 OCI IAM 只读身份调用真实 `ListInstances` | 能读取获准 compartment 的一页实例，包括合法空页。访问无权 compartment 时失败，保留可诊断的状态和 OCI 请求 ID。 |

## 调用示例

在桌面端保存 OCI Compute 项目后，Agent 可以通过现有 `http_send` 读取一页实例。以下示例中的项目名和 compartment ID 需换成实际值。

```json
{
  "item": "OCI Compute 东京",
  "method": "GET",
  "url": "instances",
  "query": {
    "compartmentId": "<compartment OCID>",
    "limit": 20
  }
}
```

- [Oracle OCI 请求签名规范](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/signingrequests.htm)
- [OCI REST API、区域接入点与分页](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/usingapi.htm)
- [Oracle OCI 商业云区域列表](https://docs.oracle.com/en-us/iaas/Content/General/Concepts/regions.htm)

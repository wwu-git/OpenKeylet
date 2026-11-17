# OCI API 请求签名设计

状态：待实现。

OpenKeylet 将为 OCI 用户 API 密钥提供显式的「OCI API 签名」认证方式。Agent 继续使用现有的 `http_send`，由 OpenKeylet 在本机完成请求校验、签名和发送。

## 首版范围

首版只支持 **OCI 商业云 Compute 的只读 GET 请求**，以 `ListInstances` 作为端到端验收入口。项目绑定一个明确选择的区域，目标形如 `https://iaas.<region>.oraclecloud.com/20160918/`。Compute 和 Networking 共用部分入口，因此仅限制主机和 GET 方法不足以限定首版范围。执行器还应检查请求路径属于首版明确支持的 Compute 只读操作。最小实现只允许 `GET /20160918/instances`。

其他 HTTP 方法、Networking、Object Storage、S3 兼容接口、流式上传和实例主体认证留待后续。现有 OCI 地址模板继续用于配置地址，选择模板不会自动启用签名。

## 凭据和签名

项目保存所选区域、租户 OCID、用户 OCID 和 OCI 公钥指纹。RSA API 签名私钥从 PEM 导入并加密保存，作为独立的凭据类型。首版可先支持未加密的 RSA PEM，导入加密 PEM 时明确提示当前不支持。

签名私钥只供本地签名器使用。它不进入 `items_search` 结果，不作为通用秘密占位符，也不复用 SSH 私钥字段。MCP 返回、操作日志和错误信息中不包含私钥或签名认证头。

最终请求通过检查后，使用实际发送的方法、已编码的路径和查询串、主机以及 `date` 或 `x-date` 生成 OCI RSA-SHA256 签名。签名和发送使用同一份最终 URL，签名后不重新编码、不调整查询参数顺序。签名认证头由执行器生成。

## 目标校验

1. 由已保存的服务和区域解析允许的 HTTPS 来源。对最终 URL 比较协议、主机和有效端口，拒绝其他主机、区域、端口及 HTTP。来源匹配仍需通过 Compute 操作路径和 GET 方法检查。
2. 校验基于实际解析后的 URL。绝对 URL 和相对路径都经过同样的检查。跨源引用、带用户信息的 URL、调用方提供的 `Host`、`Authorization`、`date` 或 `x-date` 均拒绝，避免目标或签名输入由调用参数覆盖。
3. 所有检查在签名前完成。OCI 签名请求不自动跟随重定向，即使重定向目标同源。收到重定向时报告请求被阻断。
4. 真实验收用 OCI IAM 只读身份。签名证明调用方身份，具体资源访问权限由 OCI IAM 决定。

## 验收用例

| 场景 | 预期结果 |
| --- | --- |
| 固定密钥、时钟，使用 Oracle GET 样例 | 签名与独立参考实现或官方样例一致。覆盖编码字符与多个查询参数。 |
| 本机捕获最终发出的请求 | 实际方法、路径、查询串顺序、Host 和日期与签名输入一致，签名可用测试公钥验证。 |
| 使用有效项目调用同源 `GET /20160918/instances` | 通过目标校验并发送签名请求。 |
| 指定其他主机、区域、端口、HTTP、跨源引用，或覆盖 Host 等签名相关头 | 签名前拒绝，目标端点未收到请求。 |
| 调用 POST、PUT、DELETE，或请求同源的 Networking 路径 | 签名前拒绝。 |
| 服务返回指向任意地址的 3xx | 不跟随重定向，第二个端点未收到请求。 |
| 签名失败或 OCI 返回认证错误 | 响应和活动记录不含私钥、签名认证头。项目查询和通用占位符无法读取或引用签名私钥。 |
| 使用专用 OCI 只读身份调用真实 `ListInstances` | 返回获准 compartment 的实例列表，包括合法的空列表。访问无权资源时拒绝，并保留可诊断的状态及 OCI 请求 ID。 |

- [Oracle OCI 请求签名规范](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/signingrequests.htm)
- [Oracle OCI REST API 和区域接入点](https://docs.oracle.com/en-us/iaas/Content/API/Concepts/usingapi.htm)

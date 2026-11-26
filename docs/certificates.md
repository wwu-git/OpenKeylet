# 密钥与证书

SSH 与 TLS 证书首版已完成本机实现与验收。本文说明使用方式、权限边界和实际验证范围。

## 范围

- 检查私钥、公钥及证书是否匹配。
- 通过标准模块保存公钥、SSH 证书、X.509 证书、CSR 和 CA 证书。
- 验证证书的签名、受信任签发者、有效期、身份和用途。
- 使用已有 CA 签发 SSH 用户、SSH 主机、TLS 服务器及 TLS 客户端叶子证书。
- 使用 SSH 用户证书登录，通过本地用户明确配置的 CA 验证主机证书。直连、跳板、命令和 SFTP 共用连接入口。
- 客户端与无界面 CLI 管理签发权限。MCP 检查材料、验证和申请签发，签发接口不返回私钥。
- 持久化签发序列号和请求结果，支持重试及并发；验证备份和恢复。

首版不创建根或中间 CA，不接入 ACME、自动续期或自动吊销服务，不自动部署证书或修改系统信任库。停用签发不会使已有证书失效。验证结果须说明没有查询吊销状态。

## 数据与权限

证书模板只帮助填写标准模块，沿用已有的可见性、显示、复制、删除、排序、保存和取消流程。CA 私钥保存在现有加密秘密模块中。

CA 用途由本地用户绑定具体模块 ID。MCP 修改项目名称、类型、模块或文件夹不能清除这项用途。绑定缺失时停止使用，不自动选择另一把密钥。

将项目作为受保护 CA 后，其私钥不能通过 OpenKeylet MCP 的终端、填充、环境变量或 HTTP/SSH 占位符等通用入口借用。MCP 查询只返回隐藏配置状态。客户端本地用户仍可管理材料。此限制沿备份保留。

这属于 OpenKeylet 的 MCP 授权边界，不是操作系统隔离。已有本机用户权限的程序和其他位置的重复密钥副本，不因新增工具而自动受到隔离。

签发权限与项目写入权限分开，默认关闭。授权绑定签发者的项目和模块、公钥指纹、TLS CA 证书指纹、证书用途及最大有效期。替换材料后需要重新授权。每次签发和重放都重新检查当前授权。导入备份不自动授予签发权限或主机 CA 信任。

## 签发流程

1. Agent 提交已有 CA 项目、请求 ID、待签公钥或 CSR、身份、用途及有效期。
2. OpenKeylet 在保险库锁内读取当前材料和授权，检查请求是否重复。
3. 证书处理核心校验材料及约束，在本地内存中签发并复核。
4. 加密保存签发结果并原子写入，然后返回公开证书和结果信息。

同一签发者和请求 ID、相同参数返回相同结果。参数变化时报冲突。随机序列号避免恢复或复制保险库后从相同计数器重新开始。记录不保存私钥和口令，不自动清除可重放请求。

## 协议校验

SSH 证书检查可信 CA、用户或主机类型、非空身份列表、有效期和关键选项。主机身份与请求中的逻辑主机名匹配。CA 信任按项目、主机和端口设置，不能由普通模块值替代。

SSH 用户证书的身份名称可能通过服务器授权映射到远端账号，不能一概要求证书 principal 等于 SSH 用户名。登录仍需对应私钥完成会话认证。

TLS 签发检查 CSR 签名、CA 公私钥匹配、CA 身份与签发用途、有效期和算法。生成明确的叶子 SAN 和用途，不复制 CSR 要求的 CA 权限。叶子证书不能超过签发者的有效期。使用证书链验证器检查链、身份和用途，不能只验证单个数学签名。

密钥、证书和 CSR 输入有大小及数量限制。错误和日志不包含私钥、口令或原始秘密输入。

## 客户端使用

1. 在项目编辑器添加 `publicKey`、`sshCertificate`、`certificate`、`csr` 或 `caCertificate` 标准模块，也可选择 SSH 用户证书、TLS 证书、SSH/TLS 签发材料模板。模板只组合模块，不自动注册 CA 或授权。沿用复制、Agent 可见性、删除、排序、保存和草稿。
2. SSH 用户证书项目保存账号、匹配的 `privateKey` 和一张 `sshCertificate`，认证方式选择「SSH 用户证书」。A、B 分别保存自己的认证材料；选择 B 后自动经 A 连接。
3. 在已保存项目的「本机主机身份信任」选择主机公钥或主机证书 CA。CA 模式核对的是签发者 `caFingerprint`，不是原始主机公钥指纹。读取候选值不会自动信任，必须通过可信来源核对并勾选确认。
4. 在设置「证书与签发权限」选择已保存项目及具体模块，注册已有 CA。TLS 另需绑定 CA 证书。之后可保存用途和最长有效期授权，再开启 Agent 签发。界面单次授权替换该项目原有用途；CLI 可一次授权多个用途。
5. 在「签发证书」选择已注册的 CA 项目、证书用途和有效期。SSH 填写待签公钥、身份和 SSH key ID。TLS 填写 CSR 和身份。重试保留请求 UUID，修改参数或续签时生成新的 UUID。
6. 在「验证证书」填写证书用途、待验证证书、预期身份和可信 CA。TLS 中间证书链可填写多张连续 PEM。「检查密钥匹配」可用所选项目的私钥核对公钥或证书。

本地用户签发不受 Agent 开关限制。授权只约束证书用途和最长有效期，首版不提供 principal、域名或 IP 白名单；授权前应理解 Agent 可为该用途申请任意具体身份。SSH 签发 API 默认无转发扩展，可显式请求受支持的 OpenSSH 标准用户扩展。

## MCP 使用

| 工具 | 输入与作用 |
| --- | --- |
| `certificate_capabilities` | 查询已注册 CA、Agent 开关、授权用途、有效期上限和当前是否生效 |
| `certificate_inspect` | `certificate`，解析公开证书；不表示证书可信 |
| `certificate_check_key` | `item`、可选 `keyModuleId`、`publicOrCertificate`，只返回匹配结果和公钥指纹 |
| `certificate_verify` | `certificate`、`authorities`、可选 `intermediates`、`identity`、`kind`，检查链/签名、可信 CA、时间、身份和用途 |
| `certificate_issue` | `item`、UUID `requestId`、`request`、`ttlSeconds`，按当前授权签发并持久化；不接收 CA 私钥 |

`kind` 为 `sshUser`、`sshHost`、`tlsServer` 或 `tlsClient`。SSH 请求包含 `publicKey`、非空 `principals`、`keyId`，用户证书另有可选 `extensions`。TLS 请求包含已签名 `csr` 和非空 `names`。TLS 验证数组每项恰好一张 PEM 证书。SSH 可信 CA 使用 OpenSSH 公钥文本。

例如 Agent 为 `SSH Test CA` 签发用户证书，提交以下结构，其中公钥值替换为实际待签公钥：

```json
{
  "item": "SSH Test CA",
  "requestId": "bf8175c3-e813-47d0-a481-17edf3a75242",
  "request": {
    "kind": "sshUser",
    "publicKey": "ssh-ed25519 <subject-public-key>",
    "principals": ["service-user"],
    "keyId": "service-user-session",
    "extensions": []
  },
  "ttlSeconds": 3600
}
```

先查询权限再申请签发。相同请求重试会返回原证书，即使原证书已到期；续签必须使用新的请求 ID。Agent 不能注册/解除 CA、修改授权或自动信任主机 CA。`check_key.matches` 仅说明密钥对应，不代替 `certificate_verify`。

## 无界面 CLI

使用与 MCP 相同的 `OPENKEYLET_DATA_DIR`。以下 `ITEM_ID`、`KEY_MODULE_ID`、`CA_CERT_MODULE_ID` 为已有项目和模块的 UUID。CLI 是本机用户管理入口，签发使用 Owner 权限；不受 Agent 签发开关限制。

```sh
openkeylet certificates status
openkeylet certificates authority set ITEM_ID ssh KEY_MODULE_ID
openkeylet certificates authority set ITEM_ID tls KEY_MODULE_ID CA_CERT_MODULE_ID
openkeylet certificates permissions allow ITEM_ID sshUser,sshHost 3600
openkeylet certificates permissions enable
openkeylet certificates permissions remove ITEM_ID
openkeylet certificates permissions disable
openkeylet certificates authority clear ITEM_ID

openkeylet certificates check-key ITEM_ID subject.pub
openkeylet certificates check-key ITEM_ID subject.pub KEY_MODULE_ID
openkeylet certificates inspect issued-cert.pub
openkeylet certificates verify verify-request.json
openkeylet certificates issue ITEM_ID REQUEST_UUID request.json 3600
openkeylet ssh-trust set-ca ITEM_ID HOST PORT SHA256:CA_FINGERPRINT
```

`request.json` 是上例的 `request` 对象；`verify-request.json` 对应 MCP 验证输入。文件大小最多 1 MiB。命令不接受私钥或口令参数；私钥沿用已有保险库导入逻辑。`authority clear` 同时移除该 CA 的签发授权，并解除其 MCP 通用用途限制。

## 格式和兼容性

- SSH 使用现有 RSA、ECDSA 和 Ed25519 密钥；RSA 至少 2048 位。支持加密 OpenSSH 和受支持的加密 PKCS#8。
- TLS 支持 RSA ≥2048 位 PKCS#1/PKCS#8、P-256/P-384 PKCS#8、Ed25519 PKCS#8、P-256 SEC1。加密 PKCS#8 支持 PBES2，限制 PBKDF2/scrypt 工作量；不支持传统加密 PEM 或 P-384 SEC1。
- TLS 身份首版为 DNS/IP SAN，不含 URI/email SAN。单张证书和 CSR 不混入其他 PEM 材料。最大有效期 366 天，Agent 还受授权上限约束。
- TLS 公钥指纹为 SHA256(SPKI DER)，SSH 为 OpenSSH 公钥指纹；两种编码不能直接比较。授权另保存 `IssuerIdentity.certificateFingerprint`，它为 SHA256(完整 CA 证书 DER)，用于识别同一公钥下的证书轮换。单独解析 TLS 证书时未知签发者指纹为空，不能据此推定可信。
- 保险库升级到版本 17，兼容读取 7–16，16 升级只补默认值。旧版本拒绝版本 17，不能降级直接打开。项目及 CA 绑定随加密备份保留；签发授权、主机信任及本机签发日志不随便携备份导入。

## 实现顺序与验收

| 单元 | 通过条件 |
| --- | --- |
| SSH 依赖升级 | 既有原始公钥直连、跳板和 SFTP 通过；普通公钥固定值不隐式信任证书 |
| 证书处理核心 | 正反例通过；OpenSSH/OpenSSL 独立接受签发产物 |
| 模块与保险库 | 迁移、草稿、回收站、备份、冲突副本和绑定重映射通过 |
| 受保护 CA 与授权 | 默认拒绝、即时撤权、材料替换、通用秘密入口拒绝、并发幂等通过 |
| SSH 证书连接 | 本机证书服务端和用户证书直连、跳板、命令及 SFTP 通过；错误 CA、身份、类型和时间在认证前拒绝 |
| MCP 与 CLI | 工具发现、参数说明、权限管理和实际模拟签发通过；不回显秘密 |
| 客户端 | 标准模块、模板、保存取消、权限设置及窄窗口通过 |
| 最终回归 | GUI 与无界面构建、既有 OCI/HTTP/SSH、写入权限及备份测试通过 |

测试只使用临时保险库、本机端点及合成材料。不使用真实 CA、服务器或云资源。任何平台未完成的实机验证单独列出，不能由编译或模拟结果替代。

## 本机验收记录

2026-11-24，在 macOS 上使用临时保险库、合成密钥及本机端点完成以下验收。没有读取真实保险库、使用真实 CA 或部署应用。

| 验收 | 实际结果 |
| --- | --- |
| 完整 Rust 回归 | 275 库测试、1 二进制测试、3 管理 CLI 集成和 2 证书 CLI/MCP 集成通过，共 281 项；0 失败 |
| 本机 OpenSSH 独立互通 | 默认忽略的用例另行显式执行，1 项通过；实际证书登录，principal 可与远端账号不同 |
| SSH 直连与跳板 | 实际命令、SFTP 上传下载通过；错误 CA/名称/类型/时间/签名/密钥及关键选项拒绝，无认证回退 |
| 证书核心 | 16 项通过；OpenSSL/OpenSSH 独立检查；正确 TLS 名称/CA 实际握手成功，错误名称/CA 拒绝 |
| 多级 TLS 链 | root → int1 → int2 → leaf 验证通过；缺失/替换任一中间证书、错误 SAN 或用途均拒绝 |
| CA 权限与保险库 | 默认拒绝、用途/TTL、并发相同请求、重开后重放、材料轮换、即时撤权、版本迁移及备份不继承授权通过 |
| 实际 stdio MCP | 新进程 initialize/tools/list/tools/call 通过；签发、检查、验证、查询及同进程撤权重试通过 |
| 浏览器界面夹具 | 证书、SSH 跳板、标准个人模块、Agent 写入设置通过；460px 无水平溢出，按钮单行；多张中间证书分别提交，候选 CA 不自动信任 |
| 构建 | 默认 GUI 和 `--no-default-features` 无界面 dev 构建通过 |
| 隐私扫描 | Gitleaks 扫描新增提交；2 处命中经人工确认是 `starts_with` 私钥头部格式判断字符串，非私钥。新增内容无完整私钥 PEM；未添加扫描豁免 |

复验命令：

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo test --manifest-path src-tauri/Cargo.toml --locked local_openssh_accepts_generated_user_and_host_certificates -- --ignored --nocapture
cargo build --manifest-path src-tauri/Cargo.toml --locked
cargo build --manifest-path src-tauri/Cargo.toml --locked --no-default-features
OPENKEYLET_TEST_CHROME=chrome npm run test:certificates-ui
OPENKEYLET_TEST_CHROME=chrome npm run test:ssh-jump-ui
OPENKEYLET_TEST_CHROME=chrome npm run test:personal-ui
OPENKEYLET_TEST_CHROME=chrome npm run test:agent-write-ui
python3 scripts/test-mcp-write.py src-tauri/target/debug/openkeylet
```

独立互通测试需要 OpenSSH、OpenSSL 3 和本机端口访问。浏览器验收使用真实 Chrome 加模拟 Tauri 后端，不代表已安装桌面应用的完整端到端验证。没有安装或重连用户的 MCP，也未推送。实际使用的是 Rust 1.100；声明的最低 Rust 1.89 未单独构建。Linux、Windows、Grok Bot 云电脑的本次证书功能尚未实机验收。独立审核者核对了代码和执行日志，没有另行重跑，模型来源仅为继承配置，未验证多模型差异。

按 Model the Domain 原则将四种证书用途编码为类型，避免散落布尔配置。按 Make Operations Idempotent 原则以请求 UUID 和持久化结果处理重试。按 Build the Lever 原则保留 Rust 和浏览器夹具作为可复跑的验收工具。未安装 deslop，提交前采用 diff、无关变更、调试输出和重复逻辑检查；独立 no-comments 审核保留必要的协议及安全原因说明。

## 参考

- [OpenSSH 证书与签发](https://man.openbsd.org/ssh-keygen)
- [OpenSSL CSR 校验](https://docs.openssl.org/3.6/man1/openssl-req/)
- [X.509 扩展和 CA 约束](https://docs.openssl.org/3.6/man5/x509v3_config/)
- [rcgen CSR 和签发者校验边界](https://docs.rs/rcgen/0.14.10/rcgen/struct.CertificateSigningRequestParams.html)

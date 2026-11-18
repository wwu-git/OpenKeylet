# 无界面版 Grok Bot 云端验收记录

记录日期：2026-11-18。关联 [无界面版管理 CLI 设计](headless-admin-cli.md)。

## 证据来源与范围

以下云端结果来自用户在本聊天中转回的 Grok Bot 实际执行报告，并非本机代跑或本次独立复测。环境为 Grok Bot 云电脑 Debian GNU/Linux 13（trixie）x86_64，使用 Linux x86_64 无界面版、临时保险库和模拟数据。已覆盖真实聊天 Command MCP 与真实 GitHub 私有测试仓库；未使用生产保险库或真实业务凭据。这不代表其他系统、架构或全部异常路径已经云端验收。

## 版本与包校验

- 修复版本：`0ffaed9`；本机 Git 解析完整提交为 `0ffaed94b55eb10ad460b2908fa7883e788b9864`。
- 包名：`OpenKeylet_0.16.0_linux-headless-x64-0ffaed9.tar.gz`。
- SHA256：`8de58f4842a6cc2d4f6ea3d25eef3c4d3da71a902fa21c0b15b4a24005cd7da4`。云端报告校验通过。
- 云端程序：`/tmp/openkeylet-0ffaed9-extract-5clSu6YF/OpenKeylet_0.16.0_linux-headless-x64-0ffaed9/openkeylet`。
- 云端 `BUILD-INFO.txt` 提供短提交标识 `0ffaed9`，未提供完整 40 位提交值；完整提交值来自本机仓库解析。

## 前序验收与修复原因

初版实际聊天发现 14 个工具，并通过默认拒绝、仅授权 `agent-test`、模拟项目新增（revision 1）、修改（revision 2）、回查不返回隐藏 Token、同一连接即时撤权及最终关闭写入测试。

在私有测试仓库（仓库标识已省略）的 A、B 保险库测试中，加密快照使用 XChaCha20-Poly1305，检查未检出模拟凭据、marker 或恢复密钥明文。B 未首次恢复就上传返回 `GITHUB_RESTORE_REQUIRED`；远端更新后上传返回 `GITHUB_REMOTE_CHANGED`。但 A 仅修改公开用途后，B 恢复仍保留空用途、revision 1，未产生应有冲突副本，暴露了内容比较遗漏。

`0ffaed9` 修复恢复时的项目内容比较，补齐公开用途等元数据的差异判定。不同内容继续沿用现有合并规则：保留本地原项目，为远端内容新增冲突副本。原 A、B 的测试恢复密钥已清理，因此未对原 B 完成修复后复验；改用新临时保险库 C、D 重建相同场景，结果如下。

## 修复版 GitHub 备份恢复回归

| 步骤 | 云端实际结果 |
| --- | --- |
| 创建 C、D，C 通过 MCP 创建模拟 API 项目 | 通过；公开用途为空字符串，revision 1。 |
| C 首次上传，D 连接并首次恢复 | 通过；新增 1、合并 0。 |
| C 仅修改 description 并上传 | 通过；revision 2，其余字段未变。 |
| D 未同步就上传 | 按预期被 `GITHUB_REMOTE_CHANGED` 拒绝。 |
| D 恢复更新 | 新增 1、合并 0，总数 2，产生冲突副本。 |
| D 再次恢复同一快照 | 新增 0、合并 1，总数仍为 2；ID、公开用途和版本均未变化。 |

恢复后的两项数据：

| 字段 | 原项目 | 冲突副本 |
| --- | --- | --- |
| 名称 | GitHub 备份模拟验收0ffaed9 | GitHub 备份模拟验收0ffaed9(2) |
| ID | `5f09cd82-b16e-4e84-8165-6936d0e05fc2` | `fb02503b-d89a-49a8-bd6d-95a1acf510c6` |
| description | 空字符串 `""`，非缺失或 null | 验收更新后的公开用途（冲突测试） |
| revision | 1 | 2 |

D 的实际聊天 MCP 查询确认能查到以上两个 ID 和对应版本；两项秘密字段均隐藏，未返回 Token 值。重复恢复没有再新增副本，也没有改变这两项的 ID、用途或版本。

## 收尾结果

C、D 均显式指定各自数据目录执行关闭写入、断开 GitHub 和状态回查：

| 检查 | C | D |
| --- | --- | --- |
| 写入总开关 enabled | `false` | `false` |
| 实际写入权限 effective | `false` | `false` |
| GitHub 连接 connected | `false` | `false` |
| 文件夹白名单 | 保留 `agent-test` | 空 |

D 的实际聊天 MCP 再次确认 `enabled=false`。C 保留白名单符合关闭开关保留配置的行为；当前写入仍被关闭。

一次性测试恢复密钥文件 `/tmp/openkeylet-0ffaed9-work-1DVXjD1m/test_recovery_key` 已删除，并确认文件不存在；报告称没有其他已知密钥副本。此结论限于已知副本和文件不存在检查，不宣称存储介质安全擦除。C、D 保险库、安装包和测试仓库保留，未删除云端快照。断开 GitHub 是移除 OpenKeylet 本机连接配置，不表示撤销已有的 `gh` 登录授权。

本次云端模拟数据验收通过：修复版能保留公开用途冲突副本，重复恢复不增加重复项目，实际聊天能回查恢复结果，写入权限与连接均已关闭，已知一次性测试恢复密钥已清理。

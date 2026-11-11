# 上架流程（2026-11-11 已完成）

源码仓库、Release、registry PR 均已发布，`python scripts/check_release.py` 全过。当前只等官方 CI 与 maintainer review。下面是当时按顺序执行的步骤与结果，留作以后发新版时参照。

## 当前产物

| 项 | 值 |
| --- | --- |
| 插件 id | `jiusi.mybatis-log` |
| 版本 | `0.1.0` |
| 包 | `dist/jiusi.mybatis-log-0.1.0.oiplugin` |
| BLAKE3 digest | `0d0598f506790be89b77261e749b8347741f0c75d9657b4ce6031e6188bce164` |
| 包 sha256 | `b2e21db68036fb802d7e37f5f0c2da0f1bff58db614094fdb803030955f5d13f` |
| 签名 | publisher `jiusi`，私钥沿用 `openidea-stock-watch/.keys/jiusi.key`（不要复制进本仓库） |

`publish/publishers/jiusi.toml` 已与官方 registry 的现有条目逐字一致，PR 不应包含这个文件的改动。

## 执行顺序（已按此完成，顺序不能颠倒）

1. `git init` + 初始 commit `0543455`。
2. 创建公开仓库 `jiusi666/openidea-mybatis-log`，设 `origin`，分支统一 `main` 并 push。
3. 打 tag `v0.1.0` 并推送。
4. 创建 **非草稿** Release `v0.1.0`，正文用 `publish/RELEASE-NOTES.md`，上传附件 `jiusi.mybatis-log-0.1.0.oiplugin`（文件名不能改）。
5. `python scripts/check_release.py` 核远端附件 sha256、验签、比对本地字节。
6. `python scripts/open_pr.py --dry-run` 看计划，再执行 `python scripts/open_pr.py` 提 PR 到 `l0ng-ai/openidea-plugins`，正文用 `publish/PR.md`。
7. 等 CI（下载附件 → 核 sha256 → 验签 → `openidea-plugin check`）与 maintainer review；合并后约 1 小时出现在 `Settings › Plugins › Available`。

## 执行结果

| 项 | 结果 |
| --- | --- |
| 源码 | https://github.com/jiusi666/openidea-mybatis-log （main，commit `0543455`；文档状态更新后另有一次 commit） |
| Release | https://github.com/jiusi666/openidea-mybatis-log/releases/tag/v0.1.0 ，`draft=false`，附件 `jiusi.mybatis-log-0.1.0.oiplugin`（117562 字节） |
| 远端校验 | `check_release.py` 全过：sha256 一致、摘要重算与 Ed25519 验签通过、与本地 `dist/` 字节一致 |
| registry PR | https://github.com/l0ng-ai/openidea-plugins/pull/117 ，`state=open`，diff 只有新增 `plugins/jiusi.mybatis-log.toml`（+6 行），没有改动 `publishers/jiusi.toml` |

仍待：官方 CI 与 maintainer review、IDE 内 `Install from Disk` 实机验收。

## 验收清单

- IDE 里 `Settings › Plugins › Install from Disk` 装 `dist/jiusi.mybatis-log-0.1.0.oiplugin`，确认版本号 0.1.0。
- 权限弹窗只列 `editor.read` 与 `storage`，没有 Medium / High。
- 面板粘贴标准 MyBatis 日志能出 SQL；选中日志按 `Alt+Shift+M` 面板自动弹出；复制可用。
- `python scripts/local_install.py status` 确认没有 Linked / Dev 副本与安装包抢同一个 id。

## 注意

- 改任何进包的文件（`plugin.toml`、`dist/plugin.wasm`、`web/index.html`、`settings.schema.json`、`icons/*`）都必须重新 `pack`，digest 与 sha256 会变，registry 条目与已上传的 Release 附件要同步替换。
- `scripts/oiplugin.py` 的能力白名单已按官方 `docs/en/plugins/capabilities.mdx` 补齐（旧版漏 `editor.read` 等合法能力）；这个脚本不进包，不影响摘要。

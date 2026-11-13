# MyBatis Log

把 MyBatis / MyBatis-Plus 控制台日志里的 `Preparing:` + `Parameters:` 还原成可以直接执行的 SQL，结果一键复制或整体复制。纯本机文本处理，不联网、不执行程序、不改动项目文件。

> **定位**：只做「日志 → SQL」这一件事。Mapper 接口与 XML 之间的跳转请用官方示例插件 `openidea.mybatis`（MyBatis Navigator，0.4.1 已含 gutter 图标与 Ctrl+B）；本插件不重复导航，官方示例和社区市场里也都没有 SQL 日志还原类插件。

> **状态**：已上架 OpenIDEA 社区插件市场，插件 id `jiusi.mybatis-log`，本地已升级到 **0.2.1**（市场条目仍为 0.1.0，验收通过后再发版），许可 Apache-2.0，插件 API `^1.4`。

## 安装

- **市场安装**：`Settings › Plugins › Available` 搜索「MyBatis Log」（目前是 0.1.0）。
- **本地 0.2.1 验收**（推荐本次用这个）：`Settings › Plugins › Install from Disk`，选 `dist/jiusi.mybatis-log-0.2.1.oiplugin`；或用 `openidea-plugin link` 直接加载本目录。
- 0.2.1 **能力没有变化**，仍是 `editor.read` + `storage` 两个 Low，升级不会触发「Needs permission」。
- 装完在右侧工具窗口打开「MyBatis Log」，无需重启。

## 还原效果

粘贴或从编辑器里选中这样的日志：

```text
==>  Preparing: select id, name, status from user where id = ? and status = ?
==> Parameters: 1001(Long), ACTIVE(String)
<==      Total: 1
```

得到：

```sql
select id, name, status from user where id = 1001 and status = 'ACTIVE'
```

## 功能

- **三种输入**：右侧面板粘贴（自动还原）、选中日志后按 `Alt+Shift+M`（macOS `⌘⇧M`）、命令面板 `MyBatis Log: 还原选中的日志`。有选中就读选中文本，只有光标时读整篇文档（常见于直接打开 `.log` 文件）。
- **批量**：一次可处理多组 `Preparing` / `Parameters`，兼容 `==>`、logback 时间戳、logger 名等前缀；最多 300 条、输入上限 512 KiB。
- **按类型渲染**：数值与布尔原样，`String` / `Date` / `Timestamp` / 枚举 / `Object` 自动加引号并转义内部引号，`null` 按可配置写法输出。
- **安全回填**：只替换引号、注释之外的 `?`；占位符与参数数量不符时保留未匹配的 `?` 并在卡片上给出提示。
- **展示选项**：子句换行美化、关键字大写、末尾分号、单/双引号、`NULL`/`null` 写法。美化只影响展示，复制出去的始终是单行可执行 SQL。
- **复制与历史**：单条复制 / 复制全部（跟随当前筛选）、停止输入自动记历史且连续修改合并为一条、标签可读、可载入与删除（最多 30 条），状态栏显示最近一次还原的条数。
- **0.2.0 · 元数据**：每条 SQL 带类型徽章、logger 名（按 logger 着色分组）、耗时与 `Total:` / `Updates:` 影响行数；兼容 Hibernate `binding parameter`、P6Spy、log4j2 JSON 布局。
- **0.2.0 · 参数对照表**：序号 / 原始值 / Java 类型 / 回填结果，可在「参数表」开关里切回紧凑 chip。
- **0.2.0 · 筛选与风险**：按 SQL 类型、关键字筛选，「只看风险」；静态提示无 `WHERE` 的 UPDATE/DELETE、TRUNCATE/DROP/ALTER、无过滤的 `SELECT *`。
- **0.2.0 · 语法高亮**：关键字、字符串、数字，以及 `--` / `/* */` 注释。

设置面板新增 `显示参数对照表` 与 `显示静态 SQL 风险提示`，默认都开启；面板里也能直接切换，选项存在本机。

复制通过**面板内选中 + 浏览器复制命令**完成，因此没有申请 `clipboard` 权限；若宿主限制了这条路径，面板会提示选中结果后手动 `Ctrl+C`。

## 用法

1. 打开右侧 `MyBatis Log` 面板，把控制台日志粘进输入框；或
2. 在编辑器里选中日志，按 `Alt+Shift+M`，面板会自动弹出并显示结果；
3. 点卡片上的「复制」或工具栏「复制全部」，把 SQL 贴到数据库客户端执行。

粘贴后停止输入约 1 秒会**自动记一条历史**，连续修改只保留最后一条，不会刷屏；点「还原 SQL」按钮或按 `Ctrl/⌘+Enter` 会立即另存一条。历史标签显示 `SQL 类型 · 条数 · SQL 开头`，在下拉里选一条点「载入」即可回到当时的输入，「删除」移除该条。

## 还原规则

| 日志里的参数 | 回填结果 | 说明 |
| --- | --- | --- |
| `1001(Long)` | `1001` | `Byte/Short/Integer/Long/Float/Double/BigDecimal/BigInteger` 等按数值原样，支持带包名 |
| `3.14(Double)` | `3.14` | 同上 |
| `true(Boolean)` | `true` | 布尔原样（SQL Server 需要 `1/0` 时请手动改） |
| `张三(String)` | `'张三'` | 字符串、日期、枚举、UUID、`byte[]` 等加引号 |
| `O'Brien(String)` | `'O''Brien'` | 内部单引号翻倍转义；双引号模式同理 |
| `2026-01-02 03:04:05.0(Timestamp)` | `'2026-01-02 03:04:05.0'` | 日期时间按字符串处理 |
| `null` / `null(String)` | `NULL` | 可在设置里改成 `null` |
| `(String)` | `''` | 空字符串，不是 `NULL` |
| `hello,world(String)` | `'hello,world'` | 值里含逗号不会被误拆成两个参数 |

没有 `Preparing` 前缀、但以 `select` / `insert` / `update` / `delete` 开头的输入，会当作单条 SQL 原样格式化。

## 权限

| 能力 | 风险 | 用途 |
| --- | --- | --- |
| `editor.read` | Low | 只读取当前聚焦编辑器的文档与选区，供 `还原选中的日志` 命令使用 |
| `storage` | Low | 本机保存输入框内容、面板选项与历史记录，卸载时随插件数据一起删除 |

**不申请**：`project.read`、`editor.write`、`clipboard`、`net:*`、`process:*`、`fs:*`、`vcs.*`、`secrets`。插件读不到项目文件、写不了文件、不联网、不跑进程、不读写系统剪贴板，也没有账号和后端。

## 构建

```sh
rustup target add wasm32-wasip2        # 只需一次
cargo +stable-x86_64-pc-windows-gnu build --release --target wasm32-wasip2
mkdir -p dist && cp target/wasm32-wasip2/release/openidea_plugin_mybatis_log.wasm dist/plugin.wasm
cargo +stable-x86_64-pc-windows-gnu test
python scripts/oiplugin.py check .
python scripts/oiplugin.py pack . --key .keys/jiusi.key --publisher jiusi \
  --out dist/jiusi.mybatis-log-0.2.1.oiplugin
python scripts/oiplugin.py verify dist/jiusi.mybatis-log-0.2.1.oiplugin
```

Windows + Git Bash 下若报 `link: extra operand`，是 Git 自带的 `link` 抢了 MSVC 链接器，用上面的 `+stable-x86_64-pc-windows-gnu` 工具链即可。

## 目录

| 路径 | 说明 |
| --- | --- |
| `plugin.toml` | 清单：id、版本、能力、命令、快捷键、视图、设置 |
| `src/restore.rs` | 纯文本还原逻辑：日志解析、方言、参数类型、元数据/风险、占位符回填、SQL 美化（27 个单测） |
| `src/lib.rs` | WASM 侧：状态、命令、视图消息、设置与状态栏（6 个单测） |
| `web/index.html` | 沙箱面板，单文件；直接用浏览器打开可看本地预览 |
| `settings.schema.json` | 设置面板字段 |
| `vendor/openidea-plugin` | SDK 源码快照（API 1.4），保证离线可构建 |
| `scripts/oiplugin.py` | 打包、签名、校验；能力白名单已按官方 `capabilities.mdx` 补齐 |
| `publish/` | registry 条目与 PR 草稿 |

## 已知限制

- 没有 Java AST / 类型解析，也不读项目文件；这是纯文本还原，不是 SQL 解析器。
- 参数值里同时含逗号、又没有 `(Type)` 后缀时，切分可能不精确（MyBatis 标准日志都带类型后缀）。
- 关键字大写可能把同名的裸标识符一起大写，默认关闭；关闭时不影响任何 SQL 文本。
- 一次最多 300 条 SQL、输入 512 KiB，超出会截断并提示。
- 布尔值按 `true/false` 输出；SQL Server 需要 `1/0` 时请自行替换。

## 发布状态

已上架 [OpenIDEA 社区插件市场](https://github.com/l0ng-ai/openidea-plugins)：

- 源码：https://github.com/jiusi666/openidea-mybatis-log
- 市场条目：`plugins/jiusi.mybatis-log.toml`
- Release：https://github.com/jiusi666/openidea-mybatis-log/releases/tag/v0.1.0
- 首次上架 PR：https://github.com/l0ng-ai/openidea-plugins/pull/117
- 本地 0.2.1 包：`dist/jiusi.mybatis-log-0.2.1.oiplugin`，用于 `Install from Disk` 验收；验收通过后再发 v0.2.1 Release 并更新 registry 条目。

`publish/` 里是 registry 条目与 PR 草稿；条目上的 `suggest` 会让插件出现在含 `pom.xml` / `logback-spring.xml` 等文件的项目下「Suggested for This Project」。

## 路线图

按「先不加权限，再加低权限」推进：

- **✅ 0.2.1 · 历史记录修好（已完成）**：停止输入自动保存、连续修改合并、可读标签、旧存储兼容。
- **✅ 0.2.0 · 零新能力增强（已完成，本地验收中）**：SQL 类型徽章与 logger 分组着色、耗时与 `Total:` / `Updates:` 影响行数、参数对照表、按类型与关键字筛选、静态风险提示、注释高亮，新增 Hibernate / P6Spy / log4j2 JSON 方言。仍只用 `editor.read` + `storage` 两个 Low，**不需要重新授权**。
- **0.3.0 · 日志文件跟随**：用 `files-changed` + `subscribe-paths`（API 1.3）监听项目内 `*.log`，配合 `project.read` 增量读取新增行并自动还原，逼近 IDEA 原版「自动出 SQL」的体验。新增的 `events` + `project.read` 仍是 Low，但会触发一次「Needs permission」重新授权。
- **0.4.0 · 可选体验**：把 `clipboard` 作为 **optional** 能力，让「复制」真正一键完成；不勾选时继续用面板内选中复制。

**明确不做**：拦截 IDE 原生 Run/Debug 控制台输出（插件 API 没有 console-output 事件）、在原生控制台里染色、直接执行 SQL（IDE 已有 Database 工具窗，且会引入数据库连接权限）、读取项目外日志（`fs.read:` 是 High）、上传任何日志。

## 交流

Java 技术交流 Q 群：700072697

## 许可

Apache-2.0。

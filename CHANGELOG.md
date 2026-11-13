# Changelog

## 0.2.1 — 历史记录修好

- **问题**：粘贴触发的自动还原走 `remember:false`，历史下拉框长期为空；且历史标签取日志第一行前 56 字符，带时间戳的日志看起来全是同一串前缀。
- 停止输入约 1.2 秒后自动保存一条历史；连续修改由 host 合并为同一条（`HistoryItem.auto`），不会刷屏。点「还原 SQL」或 `Ctrl/⌘+Enter` 仍立即另存一条。
- 历史标签改为 `SQL 类型 · 条数 · SQL 开头`（例如 `SELECT · 6 条 · select id, name...`），一眼可分辨。
- 历史区补上提示文案；旧存储里没有 `auto` 字段的历史条目仍可正常读取（serde default）。
- 新增 3 组测试：自动历史合并与标签、旧 JSON 兼容、载入/删除往返。单元测试 34 → 37 个。
- 能力不变，仍只申请 `editor.read` + `storage` 两个 Low。

## 0.2.0 — 零新能力增强

- 每条 SQL 增加类型徽章、logger 名、耗时（`Time: 12 ms` / `(123 ms)`）与 `Total:` / `Updates:` 行数，卡片按 logger 着色分组。
- 参数对照表：序号、原始值、Java 类型、回填后的值；可在设置或面板里切回紧凑参数标签。
- 面板支持按 SQL 类型、关键字筛选和「只看风险」；静态风险提示覆盖无 `WHERE` 的 UPDATE/DELETE、TRUNCATE/DROP/ALTER、无过滤的 `SELECT *`。
- SQL 语法高亮支持行注释与块注释；关键字表保持不变。
- 新增日志方言：Hibernate `binding parameter`、P6Spy `|statement|` / `|parameters|`、log4j2 JSON 布局的 `message` 字段。
- 设置新增 `showParamTable` 与 `riskHints`（默认开启），状态仍保存在本机。
- **能力不变**：仍只申请 `editor.read` + `storage` 两个 Low，安装与升级都不需要重新授权。
- 单元测试 27 → 33 个（新增元数据、方言、风险、显示开关 6 组），clippy `-D warnings` 零告警。

## 0.1.0 — 首个版本

- 还原 MyBatis / MyBatis-Plus 的 `Preparing:` + `Parameters:` 日志：按 `?` 顺序回填，支持带时间戳 / logger 名的前缀与一次多条 SQL。
- 按 Java 类型渲染参数：数值与布尔原样，字符串 / 日期 / 枚举加引号并转义，`null`、空串、带包名类型、值内含逗号都做了处理。
- 只替换引号与注释之外的 `?`；占位符与参数数量不符时保留 `?` 并给出提示。
- 展示选项：子句换行美化、关键字大写、末尾分号、单双引号、`NULL`/`null` 写法；复制出去的始终是单行可执行 SQL。
- 三种输入：面板粘贴自动还原、`Alt+Shift+M`（macOS `⌘⇧M`）还原编辑器选中文本、命令面板命令；只有光标时读整篇文档。
- 单条 / 全部复制、历史记录载入与删除、状态栏条数、右侧沙箱面板（明暗主题适配）。
- 能力只有 `editor.read` + `storage` 两个 Low；不联网、不执行程序、不读写项目文件与系统剪贴板。
- 27 个单元测试（`src/restore.rs` 22 个、`src/lib.rs` 5 个），clippy `-D warnings` 零告警。

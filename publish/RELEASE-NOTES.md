# MyBatis Log 0.2.1

把 MyBatis / MyBatis-Plus / Hibernate 日志还原成可直接执行的 SQL。这一版修好历史记录，并包含 0.2.0 的全部增强。

- 停止输入约 1 秒自动记一条历史，连续修改只合并为一条；标签改为 `SQL 类型 · 条数 · SQL 开头`。
- 0.2.0：SQL 类型徽章、logger 分组着色、耗时与 `Total` / `Updates` 行数、参数对照表、按类型/关键字筛选、静态风险提示、注释高亮，兼容 Hibernate / P6Spy / log4j2 JSON 日志。
- 能力仍只有 `editor.read` + `storage` 两个 Low；不联网、不跑程序、不改项目文件。
- 单元测试 37 个全过，clippy 零告警，包已 `check` / `verify`。
- 插件介绍含参与开发引导与 Java 技术交流 Q 群 700072697；registry 条目新增 `suggest`。

Release 附件：`jiusi.mybatis-log-0.2.1.oiplugin`，源码 Apache-2.0，`engines.openidea = "^1.4"`。

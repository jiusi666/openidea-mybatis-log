# MyBatis Log 0.1.0

把 MyBatis 控制台日志里的 `Preparing:` + `Parameters:` 还原成可直接执行的 SQL。

- 面板粘贴自动还原，或选中日志按 `Alt+Shift+M`（macOS `⌘⇧M`）；一次可处理多组日志，兼容带时间戳 / logger 名的前缀。
- 按 Java 类型回填参数：数值与布尔原样，字符串 / 日期 / 枚举加引号并转义，`null` 与空串分别处理；只替换引号、注释之外的 `?`。
- 子句换行美化、关键字大写、末尾分号、单双引号、`NULL`/`null` 写法可配置；复制出去的始终是单行可执行 SQL。
- 单条 / 全部复制、历史记录、状态栏条数、明暗主题面板。
- 能力只有 `editor.read` + `storage`（均 Low）：不联网、不执行程序、不读写项目文件或系统剪贴板。

官方示例插件 `openidea.mybatis`（MyBatis Navigator）覆盖的是 Mapper ↔ XML 跳转；本插件只做日志还原，不重复导航功能。

构建：`wasm32-wasip2` + component model，`engines.openidea = "^1.4"`，源码 Apache-2.0，包由本仓库源码构建、无压缩混淆。

//! MyBatis 日志 → 可执行 SQL 的纯文本还原。
//!
//! 这里全部是不碰 host 的纯逻辑：解析 `Preparing:` / `Parameters:` 日志、
//! 按 `?` 顺序回填参数、按 Java 类型决定加不加引号，以及展示用的换行美化。
//! 放在独立模块是为了能直接用 `cargo test` 覆盖各种真实日志格式。

use serde::{Deserialize, Serialize};

/// 单次输入的上限，避免把整份控制台输出推给沙箱面板。
pub const MAX_INPUT_BYTES: usize = 512 * 1024;
/// 单次最多还原多少条 SQL。
pub const MAX_ENTRIES: usize = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    /// `single` / `double`：字符串与日期参数使用的引号。
    pub quote: String,
    /// `NULL` / `null`：空值的写法。
    pub null_literal: String,
    /// 展示时把 SQL 关键字转成大写（只影响展示，不影响复制的单行 SQL）。
    pub uppercase_keywords: bool,
    /// 展示时按子句换行。
    pub pretty: bool,
    /// 还原结果末尾补分号。
    pub semicolon: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            quote: "single".to_string(),
            null_literal: "NULL".to_string(),
            uppercase_keywords: false,
            pretty: true,
            semicolon: false,
        }
    }
}

impl Options {
    pub fn normalize(mut self) -> Self {
        if self.quote != "double" {
            self.quote = "single".to_string();
        }
        if self.null_literal != "null" {
            self.null_literal = "NULL".to_string();
        }
        self
    }

    fn quote_char(&self) -> char {
        if self.quote == "double" { '"' } else { '\'' }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Restored {
    /// 可直接执行、单行的 SQL。
    pub sql: String,
    /// 按当前展示选项美化后的版本。
    pub pretty: String,
    /// 原始 Preparing 行（去掉日志前缀），方便对照。
    pub original: String,
    /// 回填后每个参数长什么样。
    pub params: Vec<String>,
    /// 参数在日志里的 Java 类型名。
    pub types: Vec<String>,
    pub warnings: Vec<String>,
    /// SQL 类型（SELECT / INSERT / UPDATE / DELETE …），供筛选与徽章使用。
    pub kind: String,
    /// 日志前缀里的 logger / mapper 名，供分组着色使用。
    pub logger: String,
    /// 日志里解析出的耗时（毫秒）。
    pub elapsed_ms: Option<u64>,
    /// `<== Total:` / `Updates:` 之类的结果行数。
    pub rows: Option<u64>,
    /// 参数对照表：序号、原始值、Java 类型、回填结果。
    pub param_rows: Vec<ParamRow>,
    /// 静态风险提示（无 WHERE 的 UPDATE/DELETE 等）。
    pub risks: Vec<String>,
    /// 识别到的日志方言。
    pub dialect: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ParamRow {
    pub index: usize,
    pub raw: String,
    pub java_type: String,
    pub rendered: String,
}

#[derive(Default, Clone, Debug)]
struct Entry {
    sql: String,
    params: Option<String>,
    /// 从日志前缀里提取的 logger / mapper 名。
    logger: String,
    elapsed_ms: Option<u64>,
    rows: Option<u64>,
}

#[derive(Clone, Debug)]
struct Param {
    value: String,
    ty: Option<String>,
}

/// 入口：还原整段日志。第二个返回值是全局提示（截断、没识别到日志等）。
pub fn restore(log: &str, opt: &Options) -> (Vec<Restored>, Vec<String>) {
    let mut notes = Vec::new();
    let truncated = log.len() > MAX_INPUT_BYTES;
    let safe = truncate_utf8(log, MAX_INPUT_BYTES);

    let mut entries = parse_entries(safe);
    if entries.is_empty() && looks_like_sql(safe) {
        entries.push(new_entry(safe.trim().to_string(), ""));
        notes.push("没有 Preparing 前缀，已按单条 SQL 原样格式化".to_string());
    }
    if entries.len() >= MAX_ENTRIES {
        notes.push(format!("最多只处理前 {MAX_ENTRIES} 条 SQL"));
    }
    if truncated {
        notes.push("输入超过 512 KiB，已截断到前半部分".to_string());
    }
    if entries.is_empty() {
        notes.push(
            "没有识别到 Preparing: 日志行；请粘贴含 Preparing / Parameters 的 MyBatis 日志"
                .to_string(),
        );
        return (Vec::new(), notes);
    }

    let lower = safe.to_ascii_lowercase();
    let dialect = if lower.contains("binding parameter") {
        "hibernate"
    } else if lower.contains("p6spy") || lower.contains("|statement|") {
        "p6spy"
    } else if lower.contains("druid") {
        "druid"
    } else {
        "mybatis"
    }
    .to_string();

    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let params = parse_params(entry.params.as_deref().unwrap_or(""));
        let rendered: Vec<String> = params.iter().map(|p| render_param(p, opt)).collect();
        let (mut sql, placeholders) = replace_placeholders(&entry.sql, &rendered);
        let mut warnings: Vec<String> = Vec::new();
        if entry.params.is_none() && placeholders > 0 {
            warnings.push("日志里缺 Parameters 行，? 保持原样".to_string());
        }
        if placeholders > rendered.len() {
            warnings.push(format!(
                "占位符比参数多 {} 个，未匹配到的 ? 已保留",
                placeholders - rendered.len()
            ));
        }
        if rendered.len() > placeholders {
            warnings.push(format!(
                "参数比占位符多 {} 个，末尾参数已忽略",
                rendered.len() - placeholders
            ));
        }
        if opt.semicolon && !sql.trim_end().ends_with(';') {
            sql.push(';');
        }
        let pretty = display_sql(&sql, opt);
        let kind = sql_kind(&sql);
        let risks = detect_risks(&sql);
        let param_rows = params
            .iter()
            .zip(rendered.iter())
            .enumerate()
            .map(|(i, (p, r))| ParamRow {
                index: i + 1,
                raw: p.value.clone(),
                java_type: p.ty.clone().unwrap_or_default(),
                rendered: r.clone(),
            })
            .collect();
        out.push(Restored {
            sql,
            pretty,
            original: entry.sql,
            params: rendered,
            types: params
                .iter()
                .map(|p| p.ty.clone().unwrap_or_default())
                .collect(),
            warnings,
            kind,
            logger: entry.logger,
            elapsed_ms: entry.elapsed_ms,
            rows: entry.rows,
            param_rows,
            risks,
            dialect: dialect.clone(),
        });
    }
    (out, notes)
}

pub fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn find_marker(line: &str, marker: &str) -> Option<usize> {
    line.to_ascii_lowercase().find(&marker.to_ascii_lowercase())
}

/// 取 `[...]` 里的第一个值，Hibernate 的 binding parameter 用。
fn bracket_value(s: &str) -> String {
    let t = s.trim();
    if let Some(a) = t.find('[')
        && let Some(b) = t[a + 1..].find(']')
    {
        return t[a + 1..a + 1 + b].trim().to_string();
    }
    t.to_string()
}

/// 把常见的其它日志格式规整成带 `Preparing:` / `Parameters:` 标记的普通文本。
fn normalize_line(raw: &str) -> String {
    let t = raw.trim();
    // log4j2 JSON 布局：取 message 字段。
    if t.starts_with('{')
        && t.contains("\"message\"")
        && let Some(v) = json_message(t)
    {
        return v;
    }
    let low = t.to_ascii_lowercase();
    if let Some(pos) = low.find("|statement|") {
        return format!("Preparing: {}", t[pos + "|statement|".len()..].trim());
    }
    if let Some(pos) = low.find("|parameters|") {
        return format!("Parameters: {}", t[pos + "|parameters|".len()..].trim());
    }
    if let Some(pos) = low.find("hibernate:") {
        return format!("Preparing: {}", t[pos + "hibernate:".len()..].trim());
    }
    t.to_string()
}

/// 只处理简单转义；JSON 布局里 message 就是一行日志文本。
fn json_message(line: &str) -> Option<String> {
    let key = "\"message\"";
    let p = line.find(key)?;
    let rest = &line[p + key.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for c in rest[1..].chars() {
        if escaped {
            match c {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                other => out.push(other),
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

fn new_entry(sql: String, prefix: &str) -> Entry {
    Entry {
        sql,
        params: None,
        logger: extract_logger(prefix),
        elapsed_ms: parse_duration_ms(prefix),
        rows: None,
    }
}

/// Hibernate 的 SQL 与参数是分开若干行打印的，单独走一条解析路径。
fn parse_hibernate(log: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Option<Entry> = None;
    let mut bindings: Vec<String> = Vec::new();
    for raw in log.lines() {
        let line = normalize_line(raw);
        let low = line.to_ascii_lowercase();
        if let Some(pos) = low.find("binding parameter") {
            let rest = &line[pos + "binding parameter".len()..];
            let value_part = rest.rsplit_once(" - ").map_or(rest, |(_, v)| v);
            let value = bracket_value(value_part);
            let ty = rest
                .split_once(" as ")
                .map(|(_, v)| bracket_value(v))
                .unwrap_or_default();
            let token = if ty.is_empty() {
                value
            } else {
                format!("{value}({ty})")
            };
            bindings.push(token);
            continue;
        }
        if let Some(pos) = find_marker(&line, "Preparing:") {
            if let Some(mut e) = current.take() {
                e.params = Some(bindings.join(", "));
                bindings.clear();
                push_entry(&mut entries, e);
            }
            let prefix = line[..pos].to_string();
            let sql = line[pos + "Preparing:".len()..].trim().to_string();
            current = Some(new_entry(sql, &prefix));
            continue;
        }
        if let Some(e) = current.as_mut() {
            if e.rows.is_none() {
                e.rows = parse_rows(&line);
            }
            if e.elapsed_ms.is_none() {
                e.elapsed_ms = parse_duration_ms(&line);
            }
        }
    }
    if let Some(mut e) = current {
        e.params = Some(bindings.join(", "));
        push_entry(&mut entries, e);
    }
    entries
}

/// 逐行抽取 Preparing / Parameters 对，兼容 `==>`、时间戳、logger 名等前缀。
fn parse_entries(log: &str) -> Vec<Entry> {
    if log.to_ascii_lowercase().contains("binding parameter") {
        return parse_hibernate(log);
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Option<Entry> = None;
    for raw in log.lines() {
        let line = normalize_line(raw);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(pos) = find_marker(line, "Preparing:") {
            if let Some(e) = current.take() {
                push_entry(&mut entries, e);
            }
            let prefix = line[..pos].to_string();
            let sql = line[pos + "Preparing:".len()..].trim().to_string();
            current = Some(new_entry(sql, &prefix));
            continue;
        }
        if let Some(pos) = find_marker(line, "Parameters:")
            && let Some(e) = current.as_mut()
        {
            e.params = Some(line[pos + "Parameters:".len()..].trim().to_string());
            continue;
        }
        // 结果行：<== Total: 1 / Updates: 2 / Time: 12 ms
        if let Some(e) = current.as_mut() {
            if e.rows.is_none() {
                e.rows = parse_rows(line);
            }
            if e.elapsed_ms.is_none() {
                e.elapsed_ms = parse_duration_ms(line);
            }
        }
    }
    if let Some(e) = current {
        push_entry(&mut entries, e);
    }
    entries
}

fn push_entry(entries: &mut Vec<Entry>, e: Entry) {
    if entries.len() < MAX_ENTRIES && !e.sql.is_empty() {
        entries.push(e);
    }
}

/// 从 Preparing 行的前缀里挑一个短 logger / mapper 名。
fn extract_logger(prefix: &str) -> String {
    let cleaned = prefix
        .replace("==>", " ")
        .replace("<==", " ")
        .replace("-->", " ");
    let mut best = "";
    for tok in cleaned.split_whitespace() {
        let t = tok.trim_end_matches(':');
        if t.contains('.')
            && !t.contains(':')
            && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        {
            best = t;
        }
    }
    if best.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = best.split('.').collect();
    let short = if parts.len() > 2 {
        parts[parts.len() - 2..].join(".")
    } else {
        best.to_string()
    };
    short.chars().take(48).collect()
}

/// 从 `(123 ms)` / `Time: 12 ms` / `耗时 12 毫秒` 里取毫秒数。
fn parse_duration_ms(s: &str) -> Option<u64> {
    let low = s.to_ascii_lowercase();
    let marker = low.find("ms").or_else(|| low.find("毫秒"))?;
    let before = s[..marker].trim_end();
    let digits: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.trim_end_matches('.').parse::<f64>().ok().map(|n| n.round() as u64)
}

/// 从 `<== Total: 1` / `Updates: 2` 里取影响或返回行数。
fn parse_rows(s: &str) -> Option<u64> {
    let low = s.to_ascii_lowercase();
    for key in [
        "total:",
        "updates:",
        "rows:",
        "row:",
        "影响行数:",
        "affected rows:",
    ] {
        if let Some(i) = low.find(key) {
            let rest = s[i + key.len()..].trim();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
    }
    None
}

fn sql_kind(sql: &str) -> String {
    let first = sql
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if [
        "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "REPLACE", "SHOW", "EXPLAIN", "TRUNCATE",
        "ALTER", "DROP", "MERGE",
    ]
    .contains(&first.as_str())
    {
        first
    } else {
        "SQL".to_string()
    }
}

/// 纯文本级的静态风险提示，不做 SQL 解析。
fn detect_risks(sql: &str) -> Vec<String> {
    let low = sql.to_ascii_lowercase();
    let kind = sql_kind(sql);
    let has_where = low
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == "where");
    let mut out = Vec::new();
    if matches!(kind.as_str(), "UPDATE" | "DELETE") && !has_where {
        out.push("UPDATE / DELETE 没有 WHERE 条件，会作用于全表".to_string());
    }
    if [
        "truncate table",
        "drop table",
        "drop database",
        "alter table",
    ]
    .iter()
    .any(|p| low.contains(p))
    {
        out.push("包含 TRUNCATE / DROP / ALTER，执行前请确认".to_string());
    }
    if kind == "SELECT" && low.contains("select *") && !has_where {
        out.push("SELECT 无过滤条件，可能返回全表数据".to_string());
    }
    out
}

fn looks_like_sql(text: &str) -> bool {
    let t = text.trim_start().to_ascii_lowercase();
    [
        "select ", "select\n", "insert ", "update ", "delete ", "with ", "explain ",
    ]
    .iter()
    .any(|p| t.starts_with(p))
}

/// 把 `1(Long), 张三(String), null` 拆成参数。
///
/// MyBatis 打印值时不会转义逗号，所以字符串值里的逗号会被 `split(',')` 误切；
/// 修复办法是：没有类型后缀、也不像合法裸值的片段，并入后一段再判一次。
fn parse_params(raw: &str) -> Vec<Param> {
    let mut raw = raw.trim();
    // 有些框架把参数列表包成 [a, b]；`[]` 表示一个参数都没有。
    if raw.starts_with('[') && raw.ends_with(']') {
        raw = raw[1..raw.len() - 1].trim();
    }
    if raw.is_empty() {
        return Vec::new();
    }
    let mut parts: Vec<String> = raw.split(',').map(str::to_string).collect();
    let mut i = 0;
    while i + 1 < parts.len() {
        let (value, ty) = split_type_suffix(&parts[i]);
        if ty.is_none() && !value.is_empty() && !is_bare_value(&value) {
            let next = parts.remove(i + 1);
            parts[i].push(',');
            parts[i].push_str(&next);
            continue; // 合并后再检查一次，别急着往后走
        }
        i += 1;
    }
    parts
        .iter()
        .map(|s| {
            let (value, ty) = split_type_suffix(s);
            Param { value, ty }
        })
        .collect()
}

/// 拆出末尾的 `(JavaType)`。类型名只允许字母数字与 `_ . [ ] $`。
fn split_type_suffix(token: &str) -> (String, Option<String>) {
    let t = token.trim();
    if !t.ends_with(')') {
        return (t.to_string(), None);
    }
    let Some(open) = t.rfind('(') else {
        return (t.to_string(), None);
    };
    let inner = &t[open + 1..t.len() - 1];
    let valid = !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '[' | ']' | '$'));
    if !valid {
        return (t.to_string(), None);
    }
    (t[..open].trim_end().to_string(), Some(inner.to_string()))
}

fn is_bare_value(v: &str) -> bool {
    let v = v.trim();
    if v.is_empty() {
        return true;
    }
    if matches!(
        v.to_ascii_lowercase().as_str(),
        "null" | "nil" | "none" | "true" | "false"
    ) {
        return true;
    }
    v.parse::<f64>().is_ok()
}

const NUMERIC_TYPES: &[&str] = &[
    "byte",
    "short",
    "int",
    "integer",
    "long",
    "float",
    "double",
    "bigdecimal",
    "biginteger",
    "number",
    "atomicinteger",
    "atomiclong",
    "bigint",
];

fn is_numeric(ty: Option<&str>, value: &str) -> bool {
    if let Some(t) = ty {
        let short = t.rsplit('.').next().unwrap_or(t).to_ascii_lowercase();
        if NUMERIC_TYPES.contains(&short.as_str()) {
            return true;
        }
    }
    value.parse::<f64>().is_ok()
}

fn render_param(p: &Param, opt: &Options) -> String {
    let mut v = p.value.trim().to_string();
    // 日志里偶尔会把字符串值再包一层引号，去掉避免生成 '"abc"'。
    if v.len() >= 2 {
        let bytes = v.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            v = v[1..v.len() - 1].to_string();
        }
    }
    let low = v.to_ascii_lowercase();
    if matches!(low.as_str(), "null" | "nil" | "none") {
        return opt.null_literal.clone();
    }
    if v.is_empty() {
        return if p.ty.is_some() {
            let q = opt.quote_char();
            format!("{q}{q}")
        } else {
            opt.null_literal.clone()
        };
    }
    if is_numeric(p.ty.as_deref(), &v) || matches!(low.as_str(), "true" | "false") {
        return v;
    }
    let q = opt.quote_char();
    let escaped = if q == '\'' {
        v.replace('\'', "''")
    } else {
        v.replace('"', "\"\"")
    };
    format!("{q}{escaped}{q}")
}

/// 按 `?` 顺序回填；跳过字符串字面量与注释里的问号。
/// 返回 (回填后的 SQL, 遇到的占位符总数)。
fn replace_placeholders(sql: &str, params: &[String]) -> (String, usize) {
    let mut out = String::with_capacity(sql.len() + params.len() * 8);
    let mut chars = sql.chars().peekable();
    let mut quote: Option<char> = None;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut used = 0usize;
    let mut placeholders = 0usize;

    while let Some(c) = chars.next() {
        if line_comment {
            out.push(c);
            if c == '\n' {
                line_comment = false;
            }
            continue;
        }
        if block_comment {
            out.push(c);
            if c == '*' && chars.peek() == Some(&'/') {
                out.push(chars.next().unwrap());
                block_comment = false;
            }
            continue;
        }
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                if chars.peek() == Some(&q) {
                    out.push(chars.next().unwrap()); // '' 转义
                } else {
                    quote = None;
                }
            } else if c == '\\' && q == '\'' && chars.peek().is_some() {
                out.push(chars.next().unwrap());
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                quote = Some(c);
                out.push(c);
            }
            '-' if chars.peek() == Some(&'-') => {
                out.push(c);
                out.push(chars.next().unwrap());
                line_comment = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                out.push(c);
                out.push(chars.next().unwrap());
                block_comment = true;
            }
            '?' => {
                placeholders += 1;
                if let Some(p) = params.get(used) {
                    out.push_str(p);
                    used += 1;
                } else {
                    out.push('?');
                }
            }
            _ => out.push(c),
        }
    }
    (out, placeholders)
}

const KEYWORDS: &[&str] = &[
    "select", "from", "where", "and", "or", "not", "null", "is", "in", "like", "between", "order",
    "by", "group", "having", "limit", "offset", "insert", "into", "values", "update", "set",
    "delete", "join", "left", "right", "inner", "cross", "on", "union", "all", "distinct", "as",
    "case", "when", "then", "else", "end", "asc", "desc", "count", "sum", "avg", "min", "max",
    "exists",
];

const BREAK_BEFORE: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "order by",
    "group by",
    "having",
    "limit",
    "offset",
    "insert into",
    "values",
    "update",
    "set",
    "delete from",
    "left join",
    "right join",
    "inner join",
    "cross join",
    "join",
    "union all",
    "union",
];

const PAIR_BREAKS: &[&str] = &[
    "order by",
    "group by",
    "insert into",
    "delete from",
    "left join",
    "right join",
    "inner join",
    "cross join",
    "union all",
];

const INDENTED: &[&str] = &["and", "or", "values", "set", "limit", "offset", "having"];

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// 收集引号与注释之外的单词（字节范围 + 小写内容）。
fn words_outside_quotes(sql: &str) -> Vec<(usize, usize, String)> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    let mut line_comment = false;
    let mut block_comment = false;
    while i < bytes.len() {
        let c = bytes[i];
        if line_comment {
            if c == b'\n' {
                line_comment = false;
            }
            i += 1;
            continue;
        }
        if block_comment {
            if c == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                if i + 1 < bytes.len() && bytes[i + 1] == q {
                    i += 2;
                    continue;
                }
                quote = None;
            } else if c == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                quote = Some(c);
                i += 1;
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                line_comment = true;
                i += 2;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                block_comment = true;
                i += 2;
            }
            _ if is_word_byte(c) => {
                let start = i;
                while i < bytes.len() && is_word_byte(bytes[i]) {
                    i += 1;
                }
                let word = String::from_utf8_lossy(&bytes[start..i]).to_ascii_lowercase();
                out.push((start, i, word));
            }
            _ => i += 1,
        }
    }
    out
}

fn uppercase_keywords(sql: &str) -> String {
    let words = words_outside_quotes(sql);
    let mut out = String::with_capacity(sql.len());
    let mut cursor = 0usize;
    for (s, e, w) in &words {
        if KEYWORDS.contains(&w.as_str()) {
            out.push_str(&sql[cursor..*s]);
            out.push_str(&w.to_ascii_uppercase());
            cursor = *e;
        }
    }
    out.push_str(&sql[cursor..]);
    out
}

fn insert_breaks(sql: &str) -> String {
    let words = words_outside_quotes(sql);
    let mut points: Vec<(usize, &'static str)> = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let (s, _, w) = &words[i];
        let mut phrase = w.clone();
        let mut consumed = 1usize;
        if let Some((_, _, next)) = words.get(i + 1) {
            let pair = format!("{w} {next}");
            if PAIR_BREAKS.contains(&pair.as_str()) {
                phrase = pair;
                consumed = 2;
            }
        }
        if BREAK_BEFORE.contains(&phrase.as_str()) {
            if *s > 0 {
                let indent = if INDENTED.contains(&phrase.as_str()) {
                    "  "
                } else {
                    ""
                };
                points.push((*s, indent));
            }
            i += consumed;
            continue;
        }
        i += 1;
    }
    let mut out = sql.to_string();
    for (pos, indent) in points.iter().rev() {
        if *pos < out.len() && out.as_bytes().get(pos.saturating_sub(1)) != Some(&b'\n') {
            out.insert_str(*pos, &format!("\n{indent}"));
        }
    }
    // 换行后上一行末尾会残留空格，清掉；字符串字面量里的换行尾空格极少见。
    out.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 展示用版本：关键字大写 + 子句换行 + 可选分号。不影响复制出去的单行 SQL。
pub fn display_sql(sql: &str, opt: &Options) -> String {
    let mut s = if opt.uppercase_keywords {
        uppercase_keywords(sql)
    } else {
        sql.to_string()
    };
    if opt.pretty {
        s = insert_breaks(&s);
    }
    if opt.semicolon && !s.trim_end().ends_with(';') {
        s.push(';');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_kind_logger_rows_duration_and_param_rows() {
        let log = "2026-11-13 10:00:00.123 DEBUG 1 --- [main] c.e.mapper.UserMapper.selectById : ==>  Preparing: select * from user where id = ?\n\
                   2026-11-13 10:00:00.130 DEBUG 1 --- [main] c.e.mapper.UserMapper.selectById : ==> Parameters: 7(Long)\n\
                   <==      Total: 1\nTime: 12 ms";
        let r = one(log, &plain());
        assert_eq!(r.sql, "select * from user where id = 7");
        assert_eq!(r.kind, "SELECT");
        assert_eq!(r.logger, "UserMapper.selectById");
        assert_eq!(r.rows, Some(1));
        assert_eq!(r.elapsed_ms, Some(12));
        assert_eq!(r.dialect, "mybatis");
        assert_eq!(r.param_rows.len(), 1);
        assert_eq!(r.param_rows[0].index, 1);
        assert_eq!(r.param_rows[0].raw, "7");
        assert_eq!(r.param_rows[0].java_type, "Long");
        assert_eq!(r.param_rows[0].rendered, "7");
    }

    #[test]
    fn hibernate_binding_parameters_are_restored() {
        let log = "Hibernate: select name from user where id = ? and status = ?\n\
                   binding parameter [1] as [BIGINT] - [7]\n\
                   binding parameter [2] as [VARCHAR] - [ACTIVE]";
        let r = one(log, &plain());
        assert_eq!(r.dialect, "hibernate");
        assert_eq!(r.sql, "select name from user where id = 7 and status = 'ACTIVE'");
        assert_eq!(r.param_rows.len(), 2);
    }

    #[test]
    fn p6spy_and_json_layout_lines_are_normalized() {
        let r = one(
            "|statement| select * from user where id = ?\n|parameters| 7(Long)",
            &plain(),
        );
        assert_eq!(r.dialect, "p6spy");
        assert_eq!(r.sql, "select * from user where id = 7");

        let json = "{\"message\":\"==>  Preparing: select 1 where id = ?\",\"level\":\"DEBUG\"}\n\
                    {\"message\":\"==> Parameters: 9(Long)\"}";
        let r2 = one(json, &plain());
        assert_eq!(r2.sql, "select 1 where id = 9");
    }

    #[test]
    fn risks_flag_unbounded_writes_and_select_all() {
        assert!(one("Preparing: update user set status = 1\nParameters: ", &plain())
            .risks
            .iter()
            .any(|x| x.contains("WHERE")));
        assert!(one("Preparing: delete from user\nParameters: ", &plain())
            .risks
            .iter()
            .any(|x| x.contains("WHERE")));
        assert!(one("Preparing: select * from user\nParameters: ", &plain())
            .risks
            .iter()
            .any(|x| x.contains("全表")));
        assert!(one("Preparing: select * from user where id = ?\nParameters: 1(Long)", &plain())
            .risks
            .is_empty());
    }

    #[test]
    fn duration_rows_and_kind_helpers() {
        assert_eq!(parse_duration_ms("2026-01-01 10:00:00 Time: 12 ms"), Some(12));
        assert_eq!(parse_duration_ms("(123 ms)"), Some(123));
        assert_eq!(parse_duration_ms("no timing here"), None);
        assert_eq!(parse_rows("<==      Total: 42"), Some(42));
        assert_eq!(parse_rows("<==    Updates: 3"), Some(3));
        assert_eq!(parse_rows("nothing"), None);
        assert_eq!(sql_kind("  select 1"), "SELECT");
        assert_eq!(sql_kind("insert into t values (1)"), "INSERT");
        assert_eq!(sql_kind("update t set a=1 where id=1"), "UPDATE");
        assert_eq!(sql_kind("whatever"), "SQL");
        assert!(detect_risks("drop table user").iter().any(|r| r.contains("DROP")));
    }

    /// 交付给用户手动验收用的综合样本，同时作为回归用例固定下来。
    #[test]
    fn manual_test_sample_bundle_parses() {
        let log = "2026-11-13 14:02:11.101 DEBUG 12345 --- [main] com.example.mapper.UserMapper.selectById : ==>  Preparing: select id, name, status from t_user where id = ? and status = ?\n\
                   2026-11-13 14:02:11.104 DEBUG 12345 --- [main] com.example.mapper.UserMapper.selectById : ==> Parameters: 1001(Long), ACTIVE(String)\n\
                   <==      Total: 1\n\
                   Time: 12 ms\n\
                   2026-11-13 14:02:12.220  INFO 12345 --- [main] com.example.mapper.OrderMapper.insert : ==>  Preparing: insert into t_order (user_id, amount, remark) values (?, ?, ?)\n\
                   2026-11-13 14:02:12.221  INFO 12345 --- [main] com.example.mapper.OrderMapper.insert : ==> Parameters: 1001(Long), 199.50(BigDecimal), 含,逗号(String)\n\
                   <==    Updates: 1\n\
                   Time: 8 ms\n\
                   2026-11-13 14:02:13.300  WARN 12345 --- [main] com.example.mapper.UserMapper.updateAll : ==>  Preparing: update t_user set status = ?\n\
                   2026-11-13 14:02:13.301  WARN 12345 --- [main] com.example.mapper.UserMapper.updateAll : ==> Parameters: DISABLED(String)\n\
                   <==    Updates: 3\n\
                   Time: 5 ms\n\
                   2026-11-13 14:02:14.400 DEBUG 12345 --- [main] com.example.mapper.LogMapper.deleteAll : ==>  Preparing: delete from t_log\n\
                   2026-11-13 14:02:14.401 DEBUG 12345 --- [main] com.example.mapper.LogMapper.deleteAll : ==> Parameters: \n\
                   Time: 2 ms\n\
                   2026-11-13 14:02:15.500 DEBUG 12345 --- [main] com.example.mapper.UserMapper.search : ==>  Preparing: select * from t_user\n\
                   2026-11-13 14:02:15.501 DEBUG 12345 --- [main] com.example.mapper.UserMapper.search : ==> Parameters: \n\
                   <==      Total: 42\n\
                   Time: 31 ms\n\
                   2026-11-13 14:02:16.600 DEBUG 12345 --- [main] com.example.mapper.UserMapper.quoteTest : ==>  Preparing: select * from t_user where name = ? and note = ? and created = ? and flag = ?\n\
                   2026-11-13 14:02:16.601 DEBUG 12345 --- [main] com.example.mapper.UserMapper.quoteTest : ==> Parameters: O'Brien(String), null, 2026-01-02 03:04:05.0(Timestamp), true(Boolean)\n\
                   <==      Total: 0\n\
                   Time: 7 ms";
        let (rs, notes) = restore(log, &plain());
        assert!(notes.is_empty(), "unexpected notes: {notes:?}");
        assert_eq!(rs.len(), 6);
        assert_eq!(rs[0].kind, "SELECT");
        assert_eq!(rs[0].logger, "UserMapper.selectById");
        assert_eq!(rs[0].rows, Some(1));
        assert_eq!(rs[0].elapsed_ms, Some(12));
        assert_eq!(rs[0].param_rows.len(), 2);
        assert_eq!(rs[1].kind, "INSERT");
        assert_eq!(rs[1].logger, "OrderMapper.insert");
        assert_eq!(rs[1].rows, Some(1));
        assert_eq!(rs[1].param_rows.len(), 3);
        assert!(rs[2].risks.iter().any(|x| x.contains("WHERE")));
        assert!(rs[3].risks.iter().any(|x| x.contains("WHERE")));
        assert!(rs[4].risks.iter().any(|x| x.contains("全表")));
        assert_eq!(rs[4].rows, Some(42));
        assert_eq!(rs[4].elapsed_ms, Some(31));
        assert!(rs[5].sql.contains("O''Brien"));
        assert!(rs[5].sql.contains("NULL"));
        assert_eq!(rs[5].param_rows.len(), 4);
    }

    fn plain() -> Options {
        Options {
            pretty: false,
            ..Options::default()
        }
    }

    fn one(log: &str, opt: &Options) -> Restored {
        let (mut r, notes) = restore(log, opt);
        assert!(notes.is_empty(), "unexpected notes: {notes:?}");
        assert_eq!(r.len(), 1, "expected exactly one SQL");
        r.pop().unwrap()
    }

    #[test]
    fn standard_mybatis_log() {
        let log = "==>  Preparing: select id, name from user where id = ? and name = ?\n\
                   ==> Parameters: 1(Long), 张三(String)\n\
                   <==      Total: 1";
        let r = one(log, &plain());
        assert_eq!(
            r.sql,
            "select id, name from user where id = 1 and name = '张三'"
        );
        assert_eq!(r.params, vec!["1", "'张三'"]);
        assert_eq!(r.types, vec!["Long", "String"]);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn null_and_numeric_and_boolean() {
        let log = "Preparing: insert into t(a,b,c,d,e) values (?,?,?,?,?)\n\
                   Parameters: 1(Integer), 3.14(Double), null, true(Boolean), 0(Long)";
        let r = one(log, &plain());
        assert_eq!(
            r.sql,
            "insert into t(a,b,c,d,e) values (1,3.14,NULL,true,0)"
        );
    }

    #[test]
    fn dates_are_quoted_and_quotes_are_escaped() {
        let log = "==>  Preparing: select * from t where created = ? and name = ?\n\
                   ==> Parameters: 2026-01-02 03:04:05.0(Timestamp), O'Brien(String)";
        let r = one(log, &plain());
        assert_eq!(
            r.sql,
            "select * from t where created = '2026-01-02 03:04:05.0' and name = 'O''Brien'"
        );
    }

    #[test]
    fn double_quote_mode_and_lowercase_null() {
        let opt = Options {
            quote: "double".into(),
            null_literal: "null".into(),
            ..plain()
        };
        let log = "Preparing: select * from t where a = ? and b = ?\nParameters: hi(String), null";
        let r = one(log, &opt);
        assert_eq!(r.sql, "select * from t where a = \"hi\" and b = null");
    }

    #[test]
    fn value_containing_comma_is_not_split() {
        let log = "Preparing: select * from t where a = ? and b = ?\n\
                   Parameters: hello,world(String), 2(Long)";
        let r = one(log, &plain());
        assert_eq!(r.params, vec!["'hello,world'", "2"]);
        assert_eq!(r.sql, "select * from t where a = 'hello,world' and b = 2");
    }

    #[test]
    fn question_marks_inside_strings_and_comments_are_kept() {
        let log = "Preparing: select '?', /* ? */ a from t where b = ? -- ?\nParameters: 7(Long)";
        let r = one(log, &plain());
        assert_eq!(r.sql, "select '?', /* ? */ a from t where b = 7 -- ?");
    }

    #[test]
    fn placeholder_and_param_count_mismatch_warns() {
        let log = "Preparing: select ?, ? from t\nParameters: 1(Long)";
        let r = one(log, &plain());
        assert_eq!(r.sql, "select 1, ? from t");
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("占位符比参数多 1 个"));

        let log = "Preparing: select ? from t\nParameters: 1(Long), 2(Long)";
        let r = one(log, &plain());
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("参数比占位符多 1 个"));
    }

    #[test]
    fn missing_parameters_line_is_reported() {
        let log = "==>  Preparing: select * from t where id = ?";
        let r = one(log, &plain());
        assert_eq!(r.sql, "select * from t where id = ?");
        assert!(r.warnings[0].contains("缺 Parameters"));
    }

    #[test]
    fn logback_prefix_and_batch() {
        let log = "2026-11-11 14:00:00.001 DEBUG 1 --- [nio-8080-exec-1] c.e.UserMapper.selectById : ==>  Preparing: select * from user where id = ?\n\
                   2026-11-11 14:00:00.002 DEBUG 1 --- [nio-8080-exec-1] c.e.UserMapper.selectById : ==> Parameters: 42(Long)\n\
                   <==      Total: 1\n\
                   ==>  Preparing: delete from user where id = ?\n\
                   ==> Parameters: 42(Long)\n\
                   <==    Updates: 1";
        let (r, notes) = restore(log, &plain());
        assert!(notes.is_empty());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].sql, "select * from user where id = 42");
        assert_eq!(r[1].sql, "delete from user where id = 42");
    }

    #[test]
    fn statement_only_input_gets_formatted() {
        let log = "select a, b from t where a = 1 and b = 2 order by a desc";
        let (r, notes) = restore(log, &Options::default());
        assert_eq!(r.len(), 1);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("没有 Preparing 前缀"));
        assert!(r[0].pretty.contains('\n'));
    }

    #[test]
    fn unrecognized_input_returns_hint() {
        let (r, notes) = restore("hello world", &plain());
        assert!(r.is_empty());
        assert!(notes[0].contains("没有识别到"));
    }

    #[test]
    fn empty_input_returns_hint() {
        let (r, notes) = restore("", &plain());
        assert!(r.is_empty());
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn pretty_breaks_clauses_but_keeps_sql_executable() {
        let opt = Options::default();
        let log = "Preparing: select id, name from user where id = ? and status = ? order by id desc limit 10\n\
                   Parameters: 1(Long), ACTIVE(String)";
        let r = one(log, &opt);
        assert_eq!(
            r.sql,
            "select id, name from user where id = 1 and status = 'ACTIVE' order by id desc limit 10"
        );
        assert!(r.pretty.contains("\nfrom user"));
        assert!(r.pretty.contains("\nwhere id = 1"));
        assert!(r.pretty.contains("\n  and status"));
        assert!(r.pretty.contains("\norder by id desc"));
        // 美化不能破坏引号里的内容
        assert!(r.pretty.contains("'ACTIVE'"));
    }

    #[test]
    fn uppercase_does_not_touch_string_literals() {
        let opt = Options {
            uppercase_keywords: true,
            pretty: false,
            ..Options::default()
        };
        let log = "Preparing: select 'select and from' from t where a = ?\nParameters: x(String)";
        let r = one(log, &opt);
        assert_eq!(r.pretty, "SELECT 'select and from' FROM t WHERE a = 'x'");
    }

    #[test]
    fn semicolon_option_appends_once() {
        let opt = Options {
            semicolon: true,
            ..plain()
        };
        let log = "Preparing: select 1;\nParameters: ";
        let r = one(log, &opt);
        assert_eq!(r.sql, "select 1;");
        assert_eq!(r.pretty, "select 1;");
    }

    #[test]
    fn fully_qualified_type_and_empty_string() {
        let log = "Preparing: update t set a = ?, b = ? where id = ?\n\
                   Parameters: 7(java.lang.Long), (String), 9(java.lang.Long)";
        // 带包名的数值类型按数值原样；空串 (String) 渲染成 '' 而不是 NULL。
        let r = one(log, &plain());
        assert_eq!(r.sql, "update t set a = 7, b = '' where id = 9");
    }

    #[test]
    fn empty_string_param_and_bracketed_empty_list() {
        let log = "Preparing: select * from t where a = ?\nParameters: []";
        let r = one(log, &plain());
        assert!(r.params.is_empty());
        assert_eq!(r.sql, "select * from t where a = ?");
    }

    #[test]
    fn options_normalize_bad_values() {
        let o = Options {
            quote: "weird".into(),
            null_literal: "weird".into(),
            ..Options::default()
        }
        .normalize();
        assert_eq!(o.quote, "single");
        assert_eq!(o.null_literal, "NULL");
    }

    #[test]
    fn utf8_truncation_never_panics() {
        let mut big = String::from("==>  Preparing: select ?, ? from t\n==> Parameters: ");
        big.push_str(&"张三".repeat(MAX_INPUT_BYTES));
        let (r, notes) = restore(&big, &plain());
        assert!(!r.is_empty());
        assert!(notes.iter().any(|n| n.contains("512 KiB")));
    }

    #[test]
    fn batch_limit_is_enforced() {
        let mut log = String::new();
        for i in 0..MAX_ENTRIES + 5 {
            log.push_str(&format!("Preparing: select {i} from dual\nParameters: \n"));
        }
        let (r, notes) = restore(&log, &plain());
        assert_eq!(r.len(), MAX_ENTRIES);
        assert!(notes.iter().any(|n| n.contains("最多只处理前")));
    }

    #[test]
    fn cjk_and_double_quote_escaping() {
        let opt = Options {
            quote: "double".into(),
            ..plain()
        };
        let log = "Preparing: select * from t where a = ?\nParameters: say \"hi\"(String)";
        let r = one(log, &opt);
        assert_eq!(r.sql, "select * from t where a = \"say \"\"hi\"\"\"");
    }
}

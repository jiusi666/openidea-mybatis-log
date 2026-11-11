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
#[serde(rename_all = "camelCase")]
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
}

#[derive(Default, Clone, Debug)]
struct Entry {
    sql: String,
    params: Option<String>,
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
        entries.push(Entry {
            sql: safe.trim().to_string(),
            params: None,
        });
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

/// 逐行抽取 Preparing / Parameters 对，兼容 `==>`、时间戳、logger 名等前缀。
fn parse_entries(log: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Option<Entry> = None;
    for line in log.lines() {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(pos) = find_marker(raw, "Preparing:") {
            if let Some(e) = current.take() {
                push_entry(&mut entries, e);
            }
            let rest = raw[pos + "Preparing:".len()..].trim().to_string();
            current = Some(Entry {
                sql: rest,
                params: None,
            });
            continue;
        }
        if let Some(pos) = find_marker(raw, "Parameters:")
            && let Some(e) = current.as_mut()
        {
            e.params = Some(raw[pos + "Parameters:".len()..].trim().to_string());
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

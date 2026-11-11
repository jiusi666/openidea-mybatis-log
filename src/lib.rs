//! MyBatis Log — 把 MyBatis 控制台日志还原成可执行 SQL 的 OpenIDEA 插件。
//!
//! WASM 侧只做三件事：读当前编辑器的选中文本、调用纯文本还原逻辑、把结果推给
//! 沙箱面板。界面全部由 `web/index.html` 渲染。不读项目文件、不写文件、不联网。
//!
//! 插件实例随时会被 IDE 回收再重建，所以持久状态一律放 `host::storage`，
//! 且每个入口都先 `ensure_loaded()`；还原结果不从存储读，init 时重算即可（纯文本，微秒级）。

mod restore;

use openidea_plugin::{
    Error, Invocation, Plugin, StatusItem, host, register_plugin,
    views::{View, ViewEvent, Viewport, Views},
};
use restore::{MAX_INPUT_BYTES, Options, Restored, restore as restore_log, truncate_utf8};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 与 plugin.toml 的 `[[contributes.views]] id` 一致。
const VIEW: &str = "mybatis-log";
const KEY_STATE: &str = "jiusi.mybatis-log:state";
/// 与 plugin.toml 的 `[[contributes.statusBar]] id` 一致。
const STATUS_ID: &str = "jiusi.mybatis-log.count";
const MAX_HISTORY: usize = 30;
/// 单条历史最多留 32 KiB，避免 storage 被一整份控制台输出撑爆。
const MAX_HISTORY_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HistoryItem {
    label: String,
    text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct State {
    /// 面板输入框里的内容，重开面板时据此重算结果。
    input: String,
    options: Options,
    history: Vec<HistoryItem>,
    show_status: bool,
    max_history: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            input: String::new(),
            options: Options::default(),
            history: Vec::new(),
            show_status: true,
            max_history: 10,
        }
    }
}

impl State {
    /// 首次运行取设置面板的默认值；之后一律以本机 storage 为准。
    fn from_settings() -> Self {
        let options = Options {
            quote: host::setting_string("quoteStyle").unwrap_or_else(|| "single".to_string()),
            null_literal: host::setting_string("nullLiteral").unwrap_or_else(|| "NULL".to_string()),
            uppercase_keywords: host::setting_bool("uppercaseKeywords").unwrap_or(false),
            pretty: host::setting_bool("prettyPrint").unwrap_or(true),
            semicolon: host::setting_bool("semicolon").unwrap_or(false),
        }
        .normalize();
        Self {
            options,
            show_status: host::setting_bool("showStatusBar").unwrap_or(true),
            max_history: host::setting_number("maxHistory")
                .map(|n| (n as usize).clamp(1, MAX_HISTORY))
                .unwrap_or(10),
            ..Self::default()
        }
    }

    fn normalize(mut self) -> Self {
        self.options = self.options.normalize();
        self.max_history = self.max_history.clamp(1, MAX_HISTORY);
        let input = truncate_utf8(&self.input, MAX_INPUT_BYTES).to_string();
        self.input = input;
        for item in &mut self.history {
            let text = truncate_utf8(&item.text, MAX_HISTORY_BYTES).to_string();
            item.text = text;
        }
        self.history.truncate(self.max_history);
        self
    }
}

#[derive(Default)]
pub struct MybatisLog {
    state: State,
    /// 还原结果只在内存里；init 时用 state.input 重算，不必存进 storage。
    results: Vec<Restored>,
    notes: Vec<String>,
    source: String,
    loaded: bool,
}

impl MybatisLog {
    fn load(&mut self) {
        self.state = host::storage::get(KEY_STATE)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<State>(&s).ok())
            .unwrap_or_else(State::from_settings)
            .normalize();
        self.loaded = true;
    }

    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.load();
        }
    }

    fn save(&self) {
        if let Ok(s) = serde_json::to_string(&self.state) {
            let _ = host::storage::set(KEY_STATE, &s);
        }
    }

    fn recompute(&mut self) {
        let (results, notes) = restore_log(&self.state.input, &self.state.options);
        self.results = results;
        self.notes = notes;
    }

    fn snapshot(&self) -> Value {
        json!({
            "type": "snapshot",
            "input": self.state.input,
            "options": self.state.options,
            "results": self.results,
            "notes": self.notes,
            "history": self.state.history,
            "source": self.source,
            "maxHistory": self.state.max_history,
        })
    }

    fn push(&self) {
        let _ = host::post_view_message(VIEW, &self.snapshot().to_string());
    }

    fn update_status(&self) {
        if !self.state.show_status {
            host::remove_status_item(STATUS_ID);
            return;
        }
        let text = if self.results.is_empty() {
            "SQL —".to_string()
        } else {
            format!("SQL · {}", self.results.len())
        };
        host::set_status_item(
            STATUS_ID,
            &StatusItem {
                text,
                tooltip: Some("点击还原当前编辑器选中的 MyBatis 日志".to_string()),
                command: Some("restore-selection".to_string()),
            },
        );
    }

    /// 记一条历史：与最近一条相同就不重复记，自动还原也不记。
    fn add_history(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let text = truncate_utf8(text, MAX_HISTORY_BYTES).to_string();
        if self.state.history.first().is_some_and(|h| h.text == text) {
            return;
        }
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        let mut label: String = first.chars().take(56).collect();
        if first.chars().count() > 56 {
            label.push('…');
        }
        if label.is_empty() {
            label = "（空日志）".to_string();
        }
        self.state.history.insert(0, HistoryItem { label, text });
        self.state.history.truncate(self.state.max_history);
    }

    fn set_input(&mut self, text: &str, remember: bool, source: &str) {
        let input = truncate_utf8(text, MAX_INPUT_BYTES).to_string();
        self.state.input = input.clone();
        self.source = source.to_string();
        if remember {
            self.add_history(&input);
        }
        self.recompute();
        self.save();
        self.update_status();
        self.push();
    }

    fn clear(&mut self) {
        self.set_input("", false, "");
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        let changed = match key {
            "pretty" => value.as_bool().map(|v| {
                self.state.options.pretty = v;
            }),
            "uppercaseKeywords" => value.as_bool().map(|v| {
                self.state.options.uppercase_keywords = v;
            }),
            "semicolon" => value.as_bool().map(|v| {
                self.state.options.semicolon = v;
            }),
            "quote" => value.as_str().map(|v| {
                self.state.options.quote = v.to_string();
            }),
            "nullLiteral" => value.as_str().map(|v| {
                self.state.options.null_literal = v.to_string();
            }),
            _ => None,
        };
        if changed.is_some() {
            self.state.options = self.state.options.clone().normalize();
            self.recompute();
            self.save();
            self.push();
        }
    }

    fn on_message(&mut self, text: &str) {
        let msg: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return,
        };
        let kind = msg.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "init" {
            self.ensure_loaded();
        }
        match kind {
            "init" => {
                self.load();
                self.recompute();
                self.update_status();
                self.push();
            }
            "restore" => {
                if let Some(text) = msg.get("text").and_then(Value::as_str) {
                    let remember = msg
                        .get("remember")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    self.set_input(text, remember, "面板输入");
                }
            }
            "setOption" => {
                if let Some(key) = msg.get("key").and_then(Value::as_str) {
                    self.set_option(key, &msg["value"]);
                }
            }
            "clear" => self.clear(),
            "history" => {
                if let Some(i) = msg.get("index").and_then(Value::as_u64)
                    && let Some(item) = self.state.history.get(i as usize)
                {
                    let text = item.text.clone();
                    self.set_input(&text, false, "历史记录");
                }
            }
            "deleteHistory" => {
                if let Some(i) = msg.get("index").and_then(Value::as_u64) {
                    let i = i as usize;
                    if i < self.state.history.len() {
                        self.state.history.remove(i);
                        self.save();
                        self.push();
                    }
                }
            }
            _ => {}
        }
    }
}

/// 读当前编辑器：有选中就读选中文本，只有光标时读整篇（常见于直接打开 .log 文件）。
fn read_editor_text(ctx: &Invocation) -> Result<String, Error> {
    let editor = ctx
        .editor()
        .ok_or_else(|| Error::Failed("请先打开一个编辑器，再执行还原".to_string()))?;
    let doc = editor.document().map_err(friendly)?;
    let selections = editor.selections().map_err(friendly)?;
    let mut parts: Vec<String> = Vec::new();
    for range in &selections {
        if let Some(text) = doc.text_in(*range)
            && !text.is_empty()
        {
            parts.push(text.to_string());
        }
    }
    if parts.is_empty() {
        Ok(doc.text)
    } else {
        Ok(parts.join("\n"))
    }
}

fn friendly(e: Error) -> Error {
    match e {
        Error::Denied(_) => Error::Failed(
            "缺少 editor.read 权限，请在 Settings › Plugins › MyBatis Log 里开启".to_string(),
        ),
        other => other,
    }
}

impl Plugin for MybatisLog {
    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        self.ensure_loaded();
        match id {
            "restore-selection" => {
                let text = read_editor_text(ctx)?;
                self.set_input(&text, true, "编辑器选中");
                let _ = host::reveal_view(VIEW);
                if self.results.is_empty() {
                    return Ok(Some(
                        "没有识别到 Preparing: 日志行，面板里说明了支持的格式".to_string(),
                    ));
                }
                Ok(Some(format!("已还原 {} 条 SQL", self.results.len())))
            }
            "clear" => {
                self.clear();
                Ok(None)
            }
            other => Err(Error::NotFound(format!("command {other}"))),
        }
    }
}

impl Views for MybatisLog {
    fn render(
        &mut self,
        _view: &str,
        _viewport: Option<Viewport>,
        _ctx: &Invocation,
    ) -> Result<View, Error> {
        // 声明了 web view，真正的界面由 web/index.html 画。
        Ok(View::message("正在加载 MyBatis Log…"))
    }

    fn handle(&mut self, _view: &str, event: ViewEvent, _ctx: &Invocation) -> Result<(), Error> {
        if let ViewEvent::Message(text) = event {
            self.on_message(&text);
        }
        Ok(())
    }
}

register_plugin!(MybatisLog, views);

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "==>  Preparing: select * from user where id = ? and name = ?\n\
                       ==> Parameters: 7(Long), 张三(String)";

    fn plugin() -> MybatisLog {
        MybatisLog {
            state: State::default(),
            loaded: true,
            ..MybatisLog::default()
        }
    }

    #[test]
    fn state_normalize_fixes_bad_saved_values() {
        let s = State {
            max_history: 999,
            options: Options {
                quote: "bogus".into(),
                null_literal: "bogus".into(),
                ..Options::default()
            },
            ..State::default()
        }
        .normalize();
        assert_eq!(s.max_history, MAX_HISTORY);
        assert_eq!(s.options.quote, "single");
        assert_eq!(s.options.null_literal, "NULL");
    }

    #[test]
    fn restoring_sets_results_and_snapshot_shape() {
        let _host = openidea_plugin::mock::MockHost::install();
        let mut p = plugin();
        p.set_input(LOG, true, "编辑器选中");
        assert_eq!(p.results.len(), 1);
        assert_eq!(
            p.results[0].sql,
            "select * from user where id = 7 and name = '张三'"
        );
        let snap = p.snapshot();
        assert_eq!(snap["type"], "snapshot");
        assert_eq!(snap["source"], "编辑器选中");
        assert_eq!(snap["results"].as_array().unwrap().len(), 1);
        assert_eq!(snap["history"].as_array().unwrap().len(), 1);
        assert_eq!(p.notes.len(), 0);
    }

    #[test]
    fn history_deduplicates_and_respects_limit() {
        let _host = openidea_plugin::mock::MockHost::install();
        let mut p = plugin();
        p.state.max_history = 3;
        p.set_input(LOG, true, "面板输入");
        p.set_input(LOG, true, "面板输入");
        assert_eq!(p.state.history.len(), 1);
        p.set_input("Preparing: select 1\nParameters: ", true, "面板输入");
        p.set_input("Preparing: select 2\nParameters: ", true, "面板输入");
        p.set_input("Preparing: select 3\nParameters: ", true, "面板输入");
        assert_eq!(p.state.history.len(), 3);
    }

    #[test]
    fn options_change_recomputes_display() {
        let _host = openidea_plugin::mock::MockHost::install();
        let mut p = plugin();
        p.set_input(LOG, false, "面板输入");
        assert!(p.results[0].pretty.contains('\n'));
        p.set_option("pretty", &json!(false));
        assert!(!p.results[0].pretty.contains('\n'));
        p.set_option("quote", &json!("double"));
        assert!(p.results[0].sql.contains("\"张三\""));
        p.set_option("quote", &json!("nonsense"));
        assert_eq!(p.state.options.quote, "single");
    }

    #[test]
    fn clear_empties_results_but_keeps_history() {
        let _host = openidea_plugin::mock::MockHost::install();
        let mut p = plugin();
        p.set_input(LOG, true, "面板输入");
        p.clear();
        assert!(p.results.is_empty());
        assert_eq!(p.state.input, "");
        assert_eq!(p.state.history.len(), 1);
    }

    #[test]
    fn view_messages_are_routed() {
        let _host = openidea_plugin::mock::MockHost::install();
        let mut p = plugin();
        p.on_message(&json!({ "type": "init" }).to_string());
        p.on_message(&json!({ "type": "restore", "text": LOG, "remember": true }).to_string());
        assert_eq!(p.results.len(), 1);
        p.on_message(&json!({ "type": "history", "index": 0 }).to_string());
        assert_eq!(p.source, "历史记录");
        p.on_message(&json!({ "type": "deleteHistory", "index": 0 }).to_string());
        assert!(p.state.history.is_empty());
        p.on_message(&json!({ "type": "clear" }).to_string());
        assert!(p.results.is_empty());
        p.on_message("not json");
        p.on_message(&json!({ "type": "unknown" }).to_string());
    }
}

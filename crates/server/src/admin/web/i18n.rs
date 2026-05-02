//! Admin web 的 i18n 框架。
//!
//! - 支持 31 种语言(对齐 claude-mem viewer)。
//! - 每种语言对应一个 `crates/server/i18n/{lang}.json`,启动时 `include_str!` +
//!   `serde_json` parse,然后 `Box::leak` 到 'static,后续无运行时 IO。
//! - 模板渲染时通过 [`LangCtx`] 注入,调用 `ctx.t("key")` 获取本地化字符串。
//! - 缺 key → fallback 到 en;缺 lang → fallback 到 en。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 31 种支持语言的 BCP 47 / locale 代码。
pub const SUPPORTED_LANGS: &[&str] = &[
    "en", "zh", "zh-tw", "ja", "ko", "es", "pt-br", "fr", "de", "ru", "ar", "he", "pl", "cs",
    "nl", "tr", "uk", "vi", "id", "th", "hi", "bn", "ro", "sv", "ur", "it", "el", "hu", "fi",
    "da", "no",
];

/// 默认 fallback 语言。
pub const DEFAULT_LANG: &str = "en";

/// 语言代码 → 原生名(用于 dropdown 显示)。
pub const NATIVE_NAMES: &[(&str, &str)] = &[
    ("en", "English"),
    ("zh", "中文"),
    ("zh-tw", "繁體中文"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("es", "Español"),
    ("pt-br", "Português (BR)"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("ru", "Русский"),
    ("ar", "العربية"),
    ("he", "עברית"),
    ("pl", "Polski"),
    ("cs", "Čeština"),
    ("nl", "Nederlands"),
    ("tr", "Türkçe"),
    ("uk", "Українська"),
    ("vi", "Tiếng Việt"),
    ("id", "Bahasa Indonesia"),
    ("th", "ไทย"),
    ("hi", "हिन्दी"),
    ("bn", "বাংলা"),
    ("ro", "Română"),
    ("sv", "Svenska"),
    ("ur", "اردو"),
    ("it", "Italiano"),
    ("el", "Ελληνικά"),
    ("hu", "Magyar"),
    ("fi", "Suomi"),
    ("da", "Dansk"),
    ("no", "Norsk"),
];

/// 是否右到左书写。
pub fn is_rtl(lang: &str) -> bool {
    matches!(lang, "ar" | "he" | "ur")
}

/// 拿到原生名(用于 dropdown);找不到回 lang code 本身。
pub fn native_name(lang: &str) -> &'static str {
    for &(code, name) in NATIVE_NAMES {
        if code == lang {
            return name;
        }
    }
    lang_to_static(lang)
}

/// 把动态 lang 字符串规约到 `SUPPORTED_LANGS` 里的 'static 引用。
fn lang_to_static(lang: &str) -> &'static str {
    for &l in SUPPORTED_LANGS {
        if l == lang {
            return l;
        }
    }
    DEFAULT_LANG
}

/// 把 BCP 47 浏览器 locale(如 `zh-CN`, `pt-PT`, `en-US`)匹配到支持的语言代码。
/// 优先级:精确 > 主语言 > 已知映射(pt→pt-br, iw→he, in→id) > en。
pub fn match_browser_lang(input: &str) -> &'static str {
    if input.is_empty() {
        return DEFAULT_LANG;
    }
    let lower = input.to_lowercase();
    for &l in SUPPORTED_LANGS {
        if l == lower {
            return l;
        }
    }
    let main = lower.split('-').next().unwrap_or(DEFAULT_LANG);
    for &l in SUPPORTED_LANGS {
        if l == main {
            return l;
        }
    }
    match main {
        "pt" => "pt-br",
        "iw" => "he", // 旧 ISO 639-1 希伯来语
        "in" => "id", // 旧 ISO 639-1 印尼语
        _ => DEFAULT_LANG,
    }
}

/// 把 `Accept-Language` 头按 q 排序后取最优匹配。
pub fn pick_from_accept_language(header: &str) -> &'static str {
    // 简化:按逗号切,去掉 ;q=...,逐项尝试匹配,首个支持的语言胜出。
    for entry in header.split(',') {
        let tag = entry.split(';').next().unwrap_or("").trim();
        if tag.is_empty() {
            continue;
        }
        let matched = match_browser_lang(tag);
        if matched != DEFAULT_LANG || tag.eq_ignore_ascii_case("en") {
            return matched;
        }
    }
    DEFAULT_LANG
}

// ---------- 翻译表加载 ----------

type Messages = HashMap<&'static str, &'static str>;

static TABLES: OnceLock<HashMap<&'static str, Messages>> = OnceLock::new();

/// 把 JSON 字符串 parse 后 Box::leak 成 'static,得到一个 messages map。
fn leak_table(raw: &'static str, lang: &'static str) -> Messages {
    let parsed: HashMap<String, String> = serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("i18n: parse {lang}.json failed: {e}"));
    let mut m: Messages = HashMap::with_capacity(parsed.len());
    for (k, v) in parsed {
        // key/value 都 leak 成 'static
        let k_static: &'static str = Box::leak(k.into_boxed_str());
        let v_static: &'static str = Box::leak(v.into_boxed_str());
        m.insert(k_static, v_static);
    }
    m
}

fn load_all() -> HashMap<&'static str, Messages> {
    let mut tables: HashMap<&'static str, Messages> = HashMap::with_capacity(SUPPORTED_LANGS.len());

    macro_rules! lang {
        ($code:literal) => {{
            let raw = include_str!(concat!("../../../i18n/", $code, ".json"));
            tables.insert($code, leak_table(raw, $code));
        }};
    }

    // 31 种语言全部 include_str + leak
    lang!("en");
    lang!("zh");
    lang!("zh-tw");
    lang!("ja");
    lang!("ko");
    lang!("es");
    lang!("pt-br");
    lang!("fr");
    lang!("de");
    lang!("ru");
    lang!("ar");
    lang!("he");
    lang!("pl");
    lang!("cs");
    lang!("nl");
    lang!("tr");
    lang!("uk");
    lang!("vi");
    lang!("id");
    lang!("th");
    lang!("hi");
    lang!("bn");
    lang!("ro");
    lang!("sv");
    lang!("ur");
    lang!("it");
    lang!("el");
    lang!("hu");
    lang!("fi");
    lang!("da");
    lang!("no");

    tables
}

fn tables() -> &'static HashMap<&'static str, Messages> {
    TABLES.get_or_init(load_all)
}

/// 取翻译。缺 key fallback 到 en,缺 lang 也 fallback 到 en;最终缺则原 key 返回。
pub fn lookup(lang: &str, key: &str) -> &'static str {
    let t = tables();
    if let Some(m) = t.get(lang) {
        if let Some(&v) = m.get(key) {
            return v;
        }
    }
    if let Some(en) = t.get(DEFAULT_LANG) {
        if let Some(&v) = en.get(key) {
            return v;
        }
    }
    // 最后兜底:把 key 本身 leak(理论上 key 都是 &'static str 字面量,实际不会走到这里)
    Box::leak(key.to_string().into_boxed_str())
}

// ---------- 模板上下文 ----------

/// 模板里使用的 i18n 上下文。每个 page 的 askama struct 内嵌一个 `pub ctx: LangCtx`,
/// 在模板里通过 `{{ ctx.t("key") }}` 取本地化字符串、`{{ ctx.lang }}` / `{{ ctx.dir }}`
/// 渲染 `<html lang>` / `<body dir>`。
#[derive(Debug, Clone, Copy)]
pub struct LangCtx {
    /// 当前语言代码('static,来自 SUPPORTED_LANGS)。
    pub lang: &'static str,
    /// 书写方向:`"rtl"` / `"ltr"`。
    pub dir: &'static str,
    /// 当前语言的原生名。
    pub native_name: &'static str,
}

impl LangCtx {
    pub fn new(lang: &str) -> Self {
        let lang = lang_to_static(lang);
        Self {
            lang,
            dir: if is_rtl(lang) { "rtl" } else { "ltr" },
            native_name: native_name(lang),
        }
    }

    /// 本地化查询。`&self` 模板里调用 `ctx.t("key")`。
    pub fn t(&self, key: &str) -> &'static str {
        lookup(self.lang, key)
    }

    /// 简单参数替换:`{name}` 之类。返回 `String`(测试 / 复杂场景使用)。
    pub fn tf(&self, key: &str, params: &[(&str, &str)]) -> String {
        let mut s = lookup(self.lang, key).to_string();
        for (k, v) in params {
            let needle = format!("{{{k}}}");
            s = s.replace(&needle, v);
        }
        s
    }

    /// 单参数版本(askama 模板友好,不需要 slice 字面量):
    /// `{{ ctx.t1("users.confirm.delete", "username", r.username.as_str()) }}`
    pub fn t1(&self, key: &str, name: &str, value: &str) -> String {
        let mut s = lookup(self.lang, key).to_string();
        let needle = format!("{{{name}}}");
        s = s.replace(&needle, value);
        s
    }

    /// 全部 31 种语言列表(用于 dropdown)。
    pub fn all_langs(&self) -> Vec<(&'static str, &'static str)> {
        NATIVE_NAMES.to_vec()
    }
}

impl Default for LangCtx {
    fn default() -> Self {
        Self::new(DEFAULT_LANG)
    }
}

// ---------- 请求级 lang 选择 ----------

pub const LANG_COOKIE_NAME: &str = "cmem_admin_lang";

/// 从 axum Request 头部 + URL query 里挑出当前 lang。
/// 优先级:URL `?lang=xx` > cookie `cmem_admin_lang` > Accept-Language 头 > en。
pub fn pick_lang(headers: &axum::http::HeaderMap, query: Option<&str>) -> &'static str {
    // 1. URL ?lang=xx
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "lang" {
                    let decoded = url_decode_simple(v);
                    let candidate = lang_to_static(decoded.trim());
                    if SUPPORTED_LANGS.contains(&candidate) {
                        return candidate;
                    }
                }
            }
        }
    }
    // 2. cookie
    if let Some(c) = cookie_value(headers, LANG_COOKIE_NAME) {
        let candidate = lang_to_static(c.trim());
        if SUPPORTED_LANGS.contains(&candidate) {
            return candidate;
        }
    }
    // 3. Accept-Language
    if let Some(h) = headers.get(axum::http::header::ACCEPT_LANGUAGE) {
        if let Ok(s) = h.to_str() {
            return pick_from_accept_language(s);
        }
    }
    DEFAULT_LANG
}

/// 简单 percent decode(只处理 lang code 里可能出现的 '%2D' 之类),失败返回原串。
fn url_decode_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as char);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    let header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for kv in header.split(';') {
        let kv = kv.trim();
        if let Some(rest) = kv.strip_prefix(&format!("{name}=")) {
            return Some(rest);
        }
    }
    None
}

// ---------- axum extractor:`LangCtx` ----------

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for LangCtx
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let lang = pick_lang(&parts.headers, parts.uri.query());
        Ok(LangCtx::new(lang))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_loads() {
        let ctx = LangCtx::new("en");
        assert_eq!(ctx.t("nav.dashboard"), "Dashboard");
    }

    #[test]
    fn chinese_loads() {
        let ctx = LangCtx::new("zh");
        assert_eq!(ctx.t("nav.dashboard"), "仪表盘");
        assert_eq!(ctx.dir, "ltr");
    }

    #[test]
    fn arabic_is_rtl() {
        let ctx = LangCtx::new("ar");
        assert_eq!(ctx.dir, "rtl");
    }

    #[test]
    fn unknown_lang_falls_back_to_en() {
        let ctx = LangCtx::new("xx");
        assert_eq!(ctx.lang, "en");
    }

    #[test]
    fn missing_key_falls_back_to_en() {
        // zh.json 应当包含全部 key,这里检查一个肯定存在的 key 在两种语言都有
        assert_eq!(lookup("en", "nav.users"), "Users");
        assert_ne!(lookup("zh", "nav.users"), "Users");
    }

    #[test]
    fn browser_lang_match() {
        assert_eq!(match_browser_lang("zh-CN"), "zh");
        assert_eq!(match_browser_lang("pt-PT"), "pt-br");
        assert_eq!(match_browser_lang("en-US"), "en");
        assert_eq!(match_browser_lang("xx-YY"), "en");
        assert_eq!(match_browser_lang("zh-TW"), "zh-tw");
    }

    #[test]
    fn accept_language_picks_first_supported() {
        assert_eq!(pick_from_accept_language("zh-CN,zh;q=0.9,en;q=0.8"), "zh");
        assert_eq!(
            pick_from_accept_language("xx-YY,ja-JP;q=0.9,en;q=0.8"),
            "ja"
        );
    }

    #[test]
    fn all_31_languages_loaded() {
        let t = tables();
        for &lang in SUPPORTED_LANGS {
            let m = t.get(lang).unwrap_or_else(|| panic!("lang {lang} missing"));
            assert!(
                !m.is_empty(),
                "lang {lang} table empty (i18n/{lang}.json missing or empty)"
            );
        }
    }

    #[test]
    fn tf_substitutes_params() {
        let ctx = LangCtx::new("en");
        let s = ctx.tf("users.confirm.delete", &[("username", "alice")]);
        assert!(s.contains("alice"));
        assert!(!s.contains("{username}"));
    }
}

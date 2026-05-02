//! Admin web UI(askama 模板 + HTMX + Tailwind via CDN)。
//!
//! - [`handlers`]:每个 GET /admin/xxx 渲染一个模板;
//! - [`templates`]:askama struct 与 .html 模板对应。
//! - [`export`]:CSV / .db.gz / per-user .zip 导出。

pub mod export;
pub mod handlers;
pub mod templates;

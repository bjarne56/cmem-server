//! 管理后台:JSON REST API + 服务端渲染 web UI。
//!
//! 路由结构:
//!
//! ```text
//! /api/admin/...     # JSON,所有响应都是 application/json,失败 4xx/5xx
//! /admin/login       # 公开:用户名 + 密码登录,设 cmem_admin_session cookie
//! /admin/logout      # 清 cookie
//! /admin/...         # 受 require_admin 保护,模板渲染
//! ```
//!
//! 鉴权:复用现有 JWT(access token)。登录后把 access token 作为 HttpOnly cookie 写入,
//! 中间件按 Cookie 优先 / Authorization Bearer 兜底解析。

pub mod handlers;
pub mod middleware;
pub mod web;

pub use middleware::require_admin;

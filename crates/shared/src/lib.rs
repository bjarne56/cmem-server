//! cmem-shared:服务器与客户端共享的 API 类型、模型与错误。
//!
//! 所有线上协议字段在这里定义,任何变更都会影响双端。

pub mod api;
pub mod error;
pub mod models;
pub mod share_mode;

pub use error::ApiErrorBody;
pub use share_mode::ShareMode;

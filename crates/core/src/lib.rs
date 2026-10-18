//! Types and arithmetic every crate agrees on. No drivers and no runtime.

pub mod error_code;
pub mod logging;
pub mod money;

pub use error_code::{ErrorCode, ERROR_CODE_KEY};
pub use money::{Rate, RateError};

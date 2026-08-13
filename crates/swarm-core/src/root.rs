include!("lib.rs");

mod lifecycle;
pub use lifecycle::{verify_join_request_signature, verify_sleep_record_signature};

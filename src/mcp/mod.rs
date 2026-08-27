pub mod codegen;
pub mod component_matcher;
pub mod design_pack;
pub mod protocol;
pub mod semantic_optimizer;
pub mod server;
pub mod state_engine;
pub mod tokens;
pub mod tools;
pub mod verify;

pub use server::{handle_jsonrpc_request, run_mcp_server};

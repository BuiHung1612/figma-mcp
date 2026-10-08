pub mod codegen;
pub mod color;
pub mod component_matcher;
pub mod design_pack;
pub mod project_tools;
pub mod protocol;
pub mod read_tools;
pub mod semantic_optimizer;
pub mod server;
pub mod state_engine;
pub mod tokens;
pub mod tools;
pub mod verify;

pub use server::{handle_jsonrpc_request, run_mcp_server};

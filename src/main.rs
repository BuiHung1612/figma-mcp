pub mod bridge;
pub mod docs;
pub mod executor;
pub mod mcp;

use bridge::{start_bridge_server, BridgeHandle, HttpProxy, DEFAULT_PORT};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "figma-mcp", version = "2.5.26", about = "High-performance Rust MCP bridge for Figma")]
struct Args {
    #[arg(short, long, default_value_t = DEFAULT_PORT)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let port = std::env::var("FIGMA_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(args.port);

    // Check for an existing bridge on the port
    let proxy = HttpProxy::new(port);
    let existing_health = proxy.check_health().await;

    if existing_health.plugin_connected {
        eprintln!("[figma-mcp] Existing bridge detected with plugin connected on port {}, using HTTP proxy", port);
        let bridge = BridgeHandle::Proxy(proxy);
        mcp::run_mcp_server(bridge).await?;
    } else {
        match start_bridge_server(port).await {
            Ok((state, actual_port)) => {
                eprintln!("[figma-mcp] Bridge started on port {}", actual_port);
                let bridge = BridgeHandle::Direct(state);
                mcp::run_mcp_server(bridge).await?;
            }
            Err(e) => {
                eprintln!("[figma-mcp] Bridge failed ({}), connecting via proxy on port {}", e, port);
                let bridge = BridgeHandle::Proxy(proxy);
                mcp::run_mcp_server(bridge).await?;
            }
        }
    }

    Ok(())
}

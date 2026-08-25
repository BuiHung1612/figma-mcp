pub mod bridge;
pub mod docs;
pub mod executor;
pub mod mcp;

use bridge::{start_bridge_server, BridgeHandle, HttpProxy, DEFAULT_PORT};
use clap::Parser;
use std::io::IsTerminal;

#[derive(Parser, Debug)]
#[command(
    name = "figma-mcp",
    version = "2.5.26",
    about = "High-performance Rust MCP bridge & server for Figma"
)]
struct Args {
    /// Port to listen on (default: 38451)
    #[arg(short, long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Force standalone server mode (interactive terminal dashboard)
    #[arg(short, long)]
    server: bool,

    /// Force stdio mode for JSON-RPC MCP clients (e.g. spawned subprocesses)
    #[arg(long)]
    stdio: bool,
}

fn print_banner(port: u16) {
    eprintln!();
    eprintln!("╔══════════════════════════════════════════════════════════════════╗");
    eprintln!("║             🎨 Figma MCP Server & Bridge v2.5.26                ║");
    eprintln!("╠══════════════════════════════════════════════════════════════════╣");
    eprintln!("║  • Figma Bridge (WS/HTTP):  http://127.0.0.1:{:<5}               ║", port);
    eprintln!("║  • MCP SSE Transport:       http://127.0.0.1:{:<5}/sse           ║", port);
    eprintln!("║  • MCP HTTP Direct:         http://127.0.0.1:{:<5}/mcp           ║", port);
    eprintln!("╠══════════════════════════════════════════════════════════════════╣");
    eprintln!("║  Status: 🚀 Server running. Ready for Figma & AI connections!    ║");
    eprintln!("║                                                                  ║");
    eprintln!("║  💡 Antigravity Setup (~/.gemini/config/mcp_config.json):        ║");
    eprintln!("║     \"figma-mcp\": {{                                              ║");
    eprintln!("║       \"serverUrl\": \"http://127.0.0.1:{:<5}/sse\"                 ║", port);
    eprintln!("║     }}                                                            ║");
    eprintln!("║                                                                  ║");
    eprintln!("║  💡 Figma Desktop:                                               ║");
    eprintln!("║     Plugins → Development → Figma MCP Bridge → Run               ║");
    eprintln!("║                                                                  ║");
    eprintln!("║  Press Ctrl+C to stop server.                                    ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════╝");
    eprintln!();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let port = std::env::var("FIGMA_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(args.port);

    let is_interactive = (args.server || std::io::stdin().is_terminal()) && !args.stdio;

    if is_interactive {
        // Standalone Server Mode (Terminal Dashboard)
        let proxy = HttpProxy::new(port);
        if proxy.is_running().await {
            eprintln!(
                "[figma-mcp] ⚠️ Server already running on port {}. Attached to active instance.",
                port
            );
            print_banner(port);
            tokio::signal::ctrl_c().await?;
            eprintln!("[figma-mcp] Exiting.");
            return Ok(());
        }

        match start_bridge_server(port).await {
            Ok((_state, actual_port)) => {
                print_banner(actual_port);
                tokio::signal::ctrl_c().await?;
                eprintln!("\n[figma-mcp] Server stopped cleanly. Goodbye!");
            }
            Err(e) => {
                eprintln!("[figma-mcp] Failed to start server on port {}: {}", port, e);
                std::process::exit(1);
            }
        }
    } else {
        // Stdio MCP Client Mode (Piped or Subprocess)
        let proxy = HttpProxy::new(port);

        if proxy.is_running().await {
            eprintln!(
                "[figma-mcp] Existing bridge detected on port {}, using HTTP proxy",
                port
            );
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
                    eprintln!(
                        "[figma-mcp] Bridge failed ({}), connecting via proxy on port {}",
                        e, port
                    );
                    let bridge = BridgeHandle::Proxy(proxy);
                    mcp::run_mcp_server(bridge).await?;
                }
            }
        }
    }

    Ok(())
}

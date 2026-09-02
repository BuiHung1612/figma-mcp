pub mod bridge;
pub mod docs;
pub mod executor;
pub mod mcp;
pub mod protocol;

use bridge::{start_bridge_server, BridgeHandle, HttpProxy, DEFAULT_PORT};
use clap::Parser;
use std::io::IsTerminal;

const LOADER_MANIFEST: &str = include_str!("../plugin/manifest.json");
const LOADER_CODE: &str = include_str!("../plugin/code.js");
const LOADER_UI: &str = include_str!("../plugin/ui.html");

#[derive(Parser, Debug)]
#[command(
    name = "figma-mcp",
    version = env!("CARGO_PKG_VERSION"),
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

    /// Register figma-mcp:// URL protocol scheme with the OS
    #[arg(long)]
    register_scheme: bool,

    /// Setup permanent Figma thin plugin in ~/.figma-mcp/plugin
    #[arg(long)]
    setup_plugin: bool,

    /// Export/setup permanent Figma thin plugin in ~/.figma-mcp/plugin
    #[arg(long)]
    export_plugin: bool,

    /// Setup permanent Figma thin plugin (alias for setup-plugin)
    #[arg(long)]
    setup: bool,

    /// Optional target directory for plugin setup
    #[arg(long)]
    plugin_dir: Option<String>,

    /// Show service installation status
    #[arg(long)]
    service_status: bool,

    /// Upgrade figma-mcp
    #[arg(long)]
    upgrade: bool,

    /// Update figma-mcp
    #[arg(long)]
    update: bool,

    /// Configure shell alias
    #[arg(long)]
    alias: bool,

    /// Install background service
    #[arg(long)]
    install_service: bool,

    /// Uninstall background service
    #[arg(long)]
    uninstall_service: bool,

    /// URL or argument passed by OS protocol handler (e.g. figma-mcp://start)
    #[arg(hide = true)]
    protocol_url: Option<String>,
}

fn setup_thin_plugin(custom_dir: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let target_dir = match custom_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => std::path::PathBuf::from(home).join(".figma-mcp").join("plugin"),
    };
    std::fs::create_dir_all(&target_dir)?;

    let manifest_path = target_dir.join("manifest.json");
    let code_path = target_dir.join("code.js");
    let ui_path = target_dir.join("ui.html");

    std::fs::write(&manifest_path, LOADER_MANIFEST)?;
    std::fs::write(&code_path, LOADER_CODE)?;
    std::fs::write(&ui_path, LOADER_UI)?;

    println!();
    println!("\x1b[32m✓ Figma MCP Dynamic Thin Plugin installed to:\x1b[0m");
    println!("  \x1b[36m{}\x1b[0m", manifest_path.display());
    println!();
    println!("\x1b[1mTo connect Figma (Do this ONCE forever):\x1b[0m");
    println!("  1. Open Figma Desktop");
    println!("  2. Go to Plugins → Development → Import plugin from manifest...");
    println!("  3. Select: {}", manifest_path.display());
    println!("  4. Run plugin: Plugins → Development → Figma MCP Bridge");
    println!("  5. Done! All future updates load dynamically without re-importing.\n");

    Ok(())
}

fn print_banner(port: u16) {
    let ver = env!("CARGO_PKG_VERSION");
    let title = format!("🎨 Figma MCP Server & Bridge v{}", ver);
    eprintln!();
    eprintln!("╔══════════════════════════════════════════════════════════════════╗");
    eprintln!("║ {:^64} ║", title);
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

    if args.setup_plugin || args.export_plugin || args.setup {
        setup_thin_plugin(args.plugin_dir.as_deref())?;
        return Ok(());
    }

    if args.upgrade || args.update {
        println!("\x1b[36m[figma-mcp]\x1b[0m To upgrade to the latest version, run:");
        println!("  \x1b[32mnpx -y figma-rust-mcp@latest --upgrade\x1b[0m\n");
        return Ok(());
    }

    if args.alias {
        println!("\x1b[36m[figma-mcp]\x1b[0m To configure shell alias, run:");
        println!("  \x1b[32mnpx -y figma-rust-mcp@latest --alias\x1b[0m\n");
        return Ok(());
    }

    if args.install_service {
        println!("\x1b[36m[figma-mcp]\x1b[0m To install auto-start background service, run:");
        println!("  \x1b[32mnpx -y figma-rust-mcp@latest --install-service\x1b[0m\n");
        return Ok(());
    }

    if args.uninstall_service {
        println!("\x1b[36m[figma-mcp]\x1b[0m To uninstall background service, run:");
        println!("  \x1b[32mnpx -y figma-rust-mcp@latest --uninstall-service\x1b[0m\n");
        return Ok(());
    }

    if args.service_status {
        println!("\x1b[36m[figma-mcp]\x1b[0m Checking service status via NPX runner...");
        println!("  \x1b[32mnpx -y figma-rust-mcp@latest --service-status\x1b[0m\n");
        return Ok(());
    }

    if args.register_scheme {
        match protocol::register_url_scheme() {
            Ok(_) => {
                eprintln!("[figma-mcp] ✓ Successfully registered figma-mcp:// URL scheme!");
                return Ok(());
            }
            Err(e) => {
                eprintln!("[figma-mcp] ⚠️ Failed to register URL scheme: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Auto-register URL scheme in background on startup
    let _ = protocol::register_url_scheme();

    let port = std::env::var("FIGMA_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(args.port);

    let is_interactive = (args.server || std::io::stdin().is_terminal()) && !args.stdio;

    if is_interactive {
        // Standalone Server Mode (Terminal Dashboard or Daemon)
        let proxy = HttpProxy::new(port);
        if proxy.is_running().await {
            eprintln!(
                "[figma-mcp] ⚠️ Server already running on port {}. Attached to active instance.",
                port
            );
            print_banner(port);
            if std::io::stdin().is_terminal() {
                tokio::signal::ctrl_c().await?;
                eprintln!("[figma-mcp] Exiting.");
            } else {
                std::future::pending::<()>().await;
            }
            return Ok(());
        }

        match start_bridge_server(port).await {
            Ok((_state, actual_port)) => {
                print_banner(actual_port);
                // Keep server process alive
                if std::io::stdin().is_terminal() {
                    tokio::signal::ctrl_c().await?;
                    eprintln!("\n[figma-mcp] Server stopped cleanly. Goodbye!");
                } else {
                    // Daemon mode (LaunchAgent / systemd / background daemon): run indefinitely
                    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
                    let _ = shutdown_rx.await;
                }
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

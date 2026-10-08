use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;

/// Registers custom URL scheme (figma-rust-mcp://) across macOS, Windows, and Linux
/// so that clicking a button in Figma Plugin or browser can automatically launch figma-rust-mcp.
pub fn register_url_scheme() -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe path: {}", e))?;

    #[cfg(target_os = "macos")]
    {
        register_macos(&current_exe)
    }

    #[cfg(target_os = "windows")]
    {
        register_windows(&current_exe)
    }

    #[cfg(target_os = "linux")]
    {
        register_linux(&current_exe)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn register_macos(current_exe: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let app_dir = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("figma-rust-mcp")
        .join("FigmaRustMCP.app");

    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    fs::create_dir_all(&macos_dir).map_err(|e| format!("Failed to create App dir: {}", e))?;

    let plist_content = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>io.github.BuiHung1612.figma-rust-mcp</string>
    <key>CFBundleName</key>
    <string>Figma Rust MCP</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{}</string>
    <key>LSUIElement</key>
    <true/>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key>
            <string>Figma Rust MCP Protocol</string>
            <key>CFBundleURLSchemes</key>
            <array>
                <string>figma-rust-mcp</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
"#, env!("CARGO_PKG_VERSION"));

    let plist_path = contents_dir.join("Info.plist");
    fs::write(&plist_path, plist_content).map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    let exe_str = current_exe.to_string_lossy();
    let launcher_script = format!(
        r#"#!/bin/bash
if curl -s -m 1 http://127.0.0.1:41730/ >/dev/null 2>&1; then
    exit 0
fi

BIN="{}"
if [ ! -f "$BIN" ]; then
    BIN="$HOME/Library/Caches/figma-rust-mcp/v{}/figma-rust-mcp"
fi
if [ ! -f "$BIN" ]; then
    BIN="$HOME/.cargo/bin/figma-rust-mcp"
fi

if [ -f "$BIN" ]; then
    nohup "$BIN" --server >/dev/null 2>&1 &
fi
"#,
        exe_str,
        env!("CARGO_PKG_VERSION")
    );

    let launcher_path = macos_dir.join("FigmaRustMCP");
    fs::write(&launcher_path, launcher_script).map_err(|e| format!("Failed to write launcher script: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&launcher_path, fs::Permissions::from_mode(0o755));
    }

    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    if Path::new(lsregister).exists() {
        let _ = Command::new(lsregister)
            .args(["-f", app_dir.to_str().unwrap_or_default()])
            .output();
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn register_windows(current_exe: &Path) -> Result<(), String> {
    let exe_str = current_exe.to_string_lossy();
    let command_str = format!("\"{}\" --background", exe_str);

    // Write registry keys using reg.exe
    let _ = Command::new("reg")
        .args(["add", r"HKCU\Software\Classes\figma-rust-mcp", "/ve", "/d", "URL:Figma Rust MCP Protocol", "/f"])
        .output();
    let _ = Command::new("reg")
        .args(["add", r"HKCU\Software\Classes\figma-rust-mcp", "/v", "URL Protocol", "/d", "", "/f"])
        .output();
    let _ = Command::new("reg")
        .args(["add", r"HKCU\Software\Classes\figma-rust-mcp\shell\open\command", "/ve", "/d", &command_str, "/f"])
        .output();

    Ok(())
}

#[cfg(target_os = "linux")]
fn register_linux(current_exe: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let app_dir = PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("applications");

    fs::create_dir_all(&app_dir).map_err(|e| format!("Failed to create applications dir: {}", e))?;

    let exe_str = current_exe.to_string_lossy();
    let desktop_content = format!(
        r#"[Desktop Entry]
Name=Figma Rust MCP
Exec="{}" --server %u
Type=Application
Terminal=false
NoDisplay=true
MimeType=x-scheme-handler/figma-rust-mcp;
"#,
        exe_str
    );

    let desktop_file = app_dir.join("figma-rust-mcp.desktop");
    fs::write(&desktop_file, desktop_content).map_err(|e| format!("Failed to write desktop file: {}", e))?;

    let _ = Command::new("xdg-mime")
        .args(["default", "figma-rust-mcp.desktop", "x-scheme-handler/figma-rust-mcp"])
        .output();

    Ok(())
}

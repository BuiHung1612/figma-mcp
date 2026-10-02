use std::{env, fs};

fn main() {
    for path in [
        "package.json",
        "plugin-runtime/code.js",
        "plugin-runtime/ui.html",
        "plugin-src",
        "scripts/build-plugin.js",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    fn hash(bytes: &[u8]) -> String {
        let value = bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        format!("{value:016x}")
    }
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let package = fs::read_to_string("package.json").unwrap();
    let package_version = package
        .lines()
        .find(|line| line.trim_start().starts_with("\"version\":"))
        .unwrap()
        .split('"')
        .nth(3)
        .unwrap();
    assert_eq!(
        package_version, version,
        "package.json and Cargo.toml versions differ"
    );
    let code = fs::read_to_string("plugin-runtime/code.js").unwrap();
    let ui = fs::read_to_string("plugin-runtime/ui.html").unwrap();
    assert!(
        code.contains(&format!("runtimeVersion: \"{version}\"")),
        "Plugin runtime version differs; run npm run build:plugin"
    );
    assert!(
        ui.contains(&format!("id=\"runtime-version\">v{version}</span>")),
        "Plugin UI version differs; run npm run build:plugin"
    );
    println!(
        "cargo:rustc-env=FIGMA_RUNTIME_CODE_HASH={}",
        hash(code.as_bytes())
    );
    println!(
        "cargo:rustc-env=FIGMA_RUNTIME_UI_HASH={}",
        hash(ui.as_bytes())
    );
}

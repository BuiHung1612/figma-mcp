#!/bin/bash
set -e

BIN_PATH="target/release/figma-mcp"
if [ ! -f "$BIN_PATH" ]; then
  BIN_PATH="target/debug/figma-mcp"
fi

echo "Testing binary: $BIN_PATH"

# Test 1: CLI Version
echo "--- Test 1: Version check ---"
$BIN_PATH --version

# Test 2: MCP Protocol JSON-RPC
echo "--- Test 2: MCP Stdio JSON-RPC ---"
INPUT=$(cat << 'JSON'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0.0"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"figma_docs","arguments":{"section":"icons"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"figma_status","arguments":{}}}
JSON
)

OUTPUT=$(echo "$INPUT" | $BIN_PATH --port 38499 2>/dev/null)
echo "$OUTPUT" | head -n 4

# Verify tool registration in JSON-RPC output
echo "--- Test 3: Verify Tool Registrations (12 tools) ---"
for TOOL in figma_status figma_get_selection figma_inspect_node figma_to_code figma_get_tokens figma_export_asset figma_export_assets figma_index figma_read figma_write figma_rules figma_docs; do
  echo "$OUTPUT" | grep -q "\"name\":\"$TOOL\"" && echo "✓ $TOOL registered"
done

# Test 4: Node/NPX wrapper test
echo "--- Test 4: NPX Wrapper ---"
node bin/figma-mcp.js --version

echo "--- All Tests completed successfully! ---"

$ErrorActionPreference = 'Stop'

$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$targetDir = Join-Path $workspaceRoot 'src-tauri/target-vscode-mcp'
$manifestPath = Join-Path $workspaceRoot 'src-tauri/Cargo.toml'

$env:CARGO_TARGET_DIR = $targetDir

& cargo build --quiet --manifest-path $manifestPath --bin rustyagent-board-mcp
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$binaryPath = Join-Path $targetDir 'debug/rustyagent-board-mcp.exe'
if (-not (Test-Path $binaryPath)) {
    throw "Built MCP binary not found at $binaryPath"
}

& $binaryPath
exit $LASTEXITCODE
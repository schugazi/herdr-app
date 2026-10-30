# Fork only (schugazi/herdr-app): builds this checkout with CI's toolchain and
# installs it over the app (install-windows.ps1). -Test runs the tests first
# (a new target under tests/ needs adding to its list).
# A build from source never updates itself or the cores: update by hand.
param([switch]$Test)
$ErrorActionPreference = "Stop"
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:LOCALAPPDATA\Programs\zig-x86_64-windows-0.16.0;$env:ProgramFiles\LLVM\bin;$env:Path"
$env:LIBCLANG_PATH = "$env:ProgramFiles\LLVM\bin"
$env:RUSTUP_TOOLCHAIN = "1.95.0"
Push-Location "$PSScriptRoot\.."
try {
    git submodule update --init --recursive
    if ($LASTEXITCODE) { throw "submodule update failed (exit $LASTEXITCODE)." }
    if ($Test) {
        # Every target but shot_pixels, which opens a real window: an SSH session has no desktop.
        cargo test --locked --bins --test commands --test notices --test skill
        if ($LASTEXITCODE) { throw "tests failed (exit $LASTEXITCODE)." }
    }
} finally {
    Pop-Location
}
# A release update the old app staged would replace this build when it quits.
$app = Join-Path $env:LOCALAPPDATA "Programs\herdr-app"
Remove-Item -LiteralPath "$app.new", "$app.new.json" -Recurse -Force -ErrorAction SilentlyContinue
& "$PSScriptRoot\install-windows.ps1"

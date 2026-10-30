# Fork only (schugazi/herdr-app): builds this checkout with CI's toolchain and
# installs it over the app (install-windows.ps1) once Herdr is closed. -Test
# runs the tests first (a new target under tests/ needs adding to its list).
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
    # Built before the app must close; install-windows.ps1 finds it built.
    cargo build --release --locked --target-dir target
    if ($LASTEXITCODE) { throw "build failed (exit $LASTEXITCODE)." }
} finally {
    Pop-Location
}
# A running release app may stage an update and swap it over this build when
# it quits, so it must be closed, and anything it staged goes.
if (Get-Process herdr-app -ErrorAction SilentlyContinue) {
    throw "Built. Quit Herdr, then run this again (without -Test) to install it."
}
$app = Join-Path $env:LOCALAPPDATA "Programs\herdr-app"
foreach ($staged in "$app.new", "$app.new.json") {
    if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged -Recurse -Force }
}
& "$PSScriptRoot\install-windows.ps1"

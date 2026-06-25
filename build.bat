@echo off
:: ─── chatbang build script (Windows) ─────────────────────────────────────────
:: Prerequisites
::   1. Rust toolchain: https://rustup.rs
::   2. MSVC Build Tools (included with Visual Studio, or install standalone):
::      https://visualstudio.microsoft.com/visual-cpp-build-tools/

echo [chatbang] Checking Rust toolchain...
where rustc >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Rust is not installed. Visit https://rustup.rs to install it.
    exit /b 1
)

echo [chatbang] Adding Windows MSVC target...
rustup target add x86_64-pc-windows-msvc

echo [chatbang] Building release binary...
cargo build --release

if %errorlevel% neq 0 (
    echo ERROR: Build failed. See output above.
    exit /b 1
)

echo.
echo  Build complete!
echo  Binary: target\release\chatbang.exe
echo.
echo  To install, copy it somewhere on your PATH, e.g.:
echo    copy target\release\chatbang.exe C:\Windows\System32\chatbang.exe
echo.
echo  First-time setup:
echo    chatbang config
echo.
echo  Start chatting:
echo    chatbang

@echo off
setlocal

if defined RUST_ROOT (
  if exist "%RUST_ROOT%\cargo\bin\cargo.exe" (
    set "CARGO_HOME=%RUST_ROOT%\cargo"
    set "RUSTUP_HOME=%RUST_ROOT%\rustup"
    set "PATH=%RUST_ROOT%\cargo\bin;%PATH%"
  )
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust/Cargo is required for Tauri dev mode.
  echo Install from https://www.rust-lang.org/tools/install and run this script again.
  exit /b 1
)

call npm install
if errorlevel 1 exit /b 1

call npm run tauri:dev

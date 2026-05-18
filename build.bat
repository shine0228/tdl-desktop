@echo off
setlocal

echo Building TDL Desktop with Tauri...

if defined RUST_ROOT (
  if exist "%RUST_ROOT%\cargo\bin\cargo.exe" (
    set "CARGO_HOME=%RUST_ROOT%\cargo"
    set "RUSTUP_HOME=%RUST_ROOT%\rustup"
    set "PATH=%RUST_ROOT%\cargo\bin;%PATH%"
  )
)

where npm >nul 2>nul
if errorlevel 1 (
  echo npm is required.
  exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust/Cargo is required for Tauri builds.
  echo Install from https://www.rust-lang.org/tools/install and run this script again.
  exit /b 1
)

call npm install
if errorlevel 1 goto :error

call npm run tauri:build
if errorlevel 1 goto :error

echo Build complete.
echo Output: release
exit /b 0

:error
echo Build failed.
exit /b 1

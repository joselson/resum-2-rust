@echo off
rem Builds Lumen on Windows with the user-level Rust (x86_64-pc-windows-gnu) and the
rem portable MinGW in %USERPROFILE%\lumen-ref\tools\mingw64, then packages
rem target\bundled\Lumen.clap, target\bundled\Lumen.vst3 and lumen-standalone.exe.
rem The VST3 build uses nih-plug's GPLv3 bindings: do not distribute it under another licence.
setlocal
set PATH=%USERPROFILE%\.cargo\bin;%USERPROFILE%\lumen-ref\tools\mingw64\bin;%PATH%
cd /d %~dp0..
set OUT=target\bundled
if not exist %OUT%\Lumen.vst3\Contents\x86_64-win mkdir %OUT%\Lumen.vst3\Contents\x86_64-win
rem Copy the plugin DLL before the standalone build, which rebuilds it with the standalone feature.
cargo build --release -p lumen-plugin --features gui,vst3 || exit /b 1
copy /y target\release\lumen_plugin.dll %OUT%\Lumen.clap >nul || exit /b 1
copy /y target\release\lumen_plugin.dll %OUT%\Lumen.vst3\Contents\x86_64-win\Lumen.vst3 >nul || exit /b 1
cargo build --release -p lumen-plugin --features standalone --bin lumen-standalone || exit /b 1
copy /y target\release\lumen-standalone.exe %OUT%\ >nul || exit /b 1
echo Built %CD%\%OUT%

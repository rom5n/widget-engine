@echo off
rem Builds main.exe against widget.dll. Needs the MSVC tools (VS 2022 Build Tools).
setlocal
cd /d %~dp0
cargo build --release -p widgine-ffi || exit /b 1
if not defined VCINSTALLDIR call "%ProgramFiles%\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cl /nologo /W4 /I ..\..\crates\engine-ffi\include main.c ..\..\target\release\widget.dll.lib || exit /b 1
copy /y ..\..\target\release\widget.dll . >nul
echo Built main.exe. Run it from this folder.

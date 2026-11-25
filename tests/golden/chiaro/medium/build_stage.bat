@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set C=D:\Entwicklung\chiaro
set B=C:\cargo-target\chiaro-build
set K=D:\Entwicklung\Knorke
cd /d C:\cargo-target\chiaro-build-p30
cl /nologo /O2 /std:c++20 /EHsc /utf-8 /MD /I%C%\src /I%C%\include /I%B%\generated /I%B%\_deps\sdl3-build\include-revision /I%B%\_deps\sdl3-src\include /I%C%\tests\gpu %K%\tests\golden\chiaro\medium\medium_stage_ref.cpp %C%\tests\gpu\GpuTestHarness.cpp /link %B%\chiaro_core.dir\Release\chiaro_core.lib %B%\lib\Release\Chiaro.lib %B%\lib\Release\SDL3.lib
if errorlevel 1 exit /b 1
copy /y %B%\bin\Release\Chiaro.dll . >nul 2>nul
copy /y %B%\bin\Release\SDL3.dll . >nul 2>nul
if not exist out mkdir out
set CHIARO_TEST_GPU_DRIVER=vulkan
medium_stage_ref.exe out

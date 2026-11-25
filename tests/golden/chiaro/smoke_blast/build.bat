@echo off
rem Builds smoke_blast_ref.cpp against the Chiaro 9bc5b9b build in %B% and runs it (Vulkan) into %O%\smoke_blast\
rem (raw dumps and smoke_blast.txt; see the head of smoke_blast_ref.cpp).
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set C=D:\Entwicklung\chiaro
set B=C:\cargo-target\chiaro-build
rem K: the Knorke checkout whose reference source is built.
if not defined K set K=D:\Entwicklung\Knorke
set O=C:\cargo-target\chiaro-build-scenes
if not exist %O% mkdir %O%
cd /d %O%
cl /nologo /O2 /std:c++20 /EHsc /utf-8 /MD /I%C%\include /I%C%\examples\common /I%B%\_deps\sdl3-build\include-revision /I%B%\_deps\sdl3-src\include %K%\tests\golden\chiaro\smoke_blast\smoke_blast_ref.cpp /link %B%\lib\Release\Chiaro.lib %B%\lib\Release\SDL3.lib
if errorlevel 1 exit /b 1
copy /y %B%\bin\Release\Chiaro.dll . >nul
copy /y %B%\bin\Release\SDL3.dll . >nul
set SDL_GPU_DRIVER=vulkan
if not exist smoke_blast mkdir smoke_blast
.\smoke_blast_ref.exe smoke_blast

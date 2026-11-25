@echo off
rem Builds iso_night_ref.cpp against the Chiaro 9bc5b9b build in %B% and runs every variant (Vulkan) into
rem %O%\<variant>\ (raw dumps and <variant>.txt; see the head of iso_night_ref.cpp).
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set C=D:\Entwicklung\chiaro
set B=C:\cargo-target\chiaro-build
rem K: the Knorke checkout whose reference source is built.
if not defined K set K=D:\Entwicklung\Knorke
set O=C:\cargo-target\chiaro-build-scenes
if not exist %O% mkdir %O%
cd /d %O%
cl /nologo /O2 /std:c++20 /EHsc /utf-8 /MD /I%C%\include /I%C%\examples\iso_night_demo /I%C%\examples\common /I%B%\_deps\sdl3-build\include-revision /I%B%\_deps\sdl3-src\include %K%\tests\golden\chiaro\iso_night\iso_night_ref.cpp /link %B%\lib\Release\Chiaro.lib %B%\lib\Release\SDL3.lib
if errorlevel 1 exit /b 1
copy /y %B%\bin\Release\Chiaro.dll . >nul
copy /y %B%\bin\Release\SDL3.dll . >nul
set SDL_GPU_DRIVER=vulkan
for %%v in (plain hard t5 cutaway grade_crt) do (
  if not exist %%v mkdir %%v
  .\iso_night_ref.exe %%v %%v
  if errorlevel 1 exit /b 1
)

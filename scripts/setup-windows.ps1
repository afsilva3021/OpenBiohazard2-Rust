# Baixa as bibliotecas SDL2 para Visual C++ em windows\SDL2 (ignorado pelo Git).
# Uso (na raiz do projeto):
#   powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1
param([string]$Version = "2.32.10")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root "windows\SDL2"
if (Test-Path (Join-Path $target "lib\x64\SDL2.lib")) {
    Write-Host "SDL2 ja esta em $target"
    exit 0
}

$url = "https://github.com/libsdl-org/SDL/releases/download/release-$Version/SDL2-devel-$Version-VC.zip"
$zip = Join-Path $env:TEMP "SDL2-devel-$Version-VC.zip"
$extract = Join-Path $env:TEMP "SDL2-devel-$Version-VC"

Write-Host "Baixando $url"
Invoke-WebRequest -Uri $url -OutFile $zip
if (Test-Path $extract) { Remove-Item -Recurse -Force $extract }
Expand-Archive -Path $zip -DestinationPath $extract

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
if (Test-Path $target) { Remove-Item -Recurse -Force $target }
Move-Item (Join-Path $extract "SDL2-$Version") $target
Remove-Item $zip

Write-Host "SDL2 $Version instalado em $target"

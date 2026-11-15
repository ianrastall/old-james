[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$build = Join-Path $root '.build'
$stage = Join-Path $build 'runtime'
$publish = Join-Path $build 'publish'
$tag = 'b12405'
$archiveName = "llama-$tag-bin-win-vulkan-x64.zip"
$archive = Join-Path $build $archiveName
$expectedHash = '42d13bd422e670dd5cbc830e8a952f33cad84a3343a5d64c2708e3403008ba1d'
$source = "https://raw.githubusercontent.com/ggml-org/llama.cpp/$tag"

$compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1
if (-not $compiler) {
    $compiler = Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'
}
if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) {
    $compiler = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
}
if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) {
    throw 'Inno Setup 6 is required to build the Windows installer.'
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vcInstallations = & $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$redist = foreach ($installation in $vcInstallations) {
    Get-ChildItem -Path "$installation\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT" -Directory -ErrorAction SilentlyContinue
}
$crt = $redist | Sort-Object { [version]$_.Parent.Parent.Name } -Descending | Select-Object -First 1
if (-not $crt) {
    throw 'The Visual C++ x64 redistributable files are required to build the Windows installer.'
}

New-Item -ItemType Directory -Force $build, $stage, $publish | Out-Null
if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
    $download = "$archive.partial"
    Invoke-WebRequest "https://github.com/ggml-org/llama.cpp/releases/download/$tag/$archiveName" -OutFile $download
    if ((Get-FileHash -LiteralPath $download).Hash -ine $expectedHash) {
        throw 'The downloaded llama.cpp archive failed its checksum.'
    }
    Move-Item -LiteralPath $download -Destination $archive
}
if ((Get-FileHash -LiteralPath $archive).Hash -ine $expectedHash) {
    throw 'The cached llama.cpp archive failed its checksum.'
}

# Copy only the server and its shared libraries from the pinned release.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    foreach ($entry in $zip.Entries) {
        if ($entry.Name -match '^(ggml(-.*)?|llama(-common|-server-impl)?|mtmd|libomp)\.dll$|^llama-server\.exe$|^LICENSE-LLVM-OpenMP$') {
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $stage $entry.Name), $true)
        }
    }
}
finally {
    $zip.Dispose()
}
foreach ($name in 'llama-server.exe', 'llama-server-impl.dll', 'ggml-vulkan.dll', 'ggml-cpu-x64.dll') {
    if (-not (Test-Path -LiteralPath (Join-Path $stage $name) -PathType Leaf)) {
        throw "Missing llama.cpp runtime file $name."
    }
}
foreach ($item in @(
    @{ Url = "$source/LICENSE"; Name = 'LICENSE-llama.cpp' },
    @{ Url = "$source/licenses/LICENSE-jsonhpp"; Name = 'LICENSE-jsonhpp' }
)) {
    $destination = Join-Path $stage $item.Name
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
        Invoke-WebRequest $item.Url -OutFile $destination
    }
}
Copy-Item -Path (Join-Path $crt.FullName '*.dll') -Destination $stage

Push-Location $root
try {
    & dotnet publish src/OldJames.Cards/OldJames.Cards.csproj -c Release -r win-x64 --self-contained true -p:RestoreLockedMode=true -o $publish
    if ($LASTEXITCODE -ne 0) { throw 'Old James publish failed.' }
    Copy-Item -Path (Join-Path $crt.FullName '*.dll') -Destination $publish
    & $compiler (Join-Path $root 'installer\OldJames.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Old James installer build failed.' }
}
finally {
    Pop-Location
}

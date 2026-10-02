# Downloads what Mimo's offline voice wake needs into src-tauri/resources/vosk
# (gitignored): the Vosk runtime DLLs and the small French and English speech
# models. Safe to re-run: anything already present is skipped, so it is also
# wired into `tauri dev` / `tauri build` (see tauri.conf.json).

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue' # Invoke-WebRequest is very slow with the progress bar on
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$root = Join-Path $PSScriptRoot '..\src-tauri\resources\vosk'

$items = @(
    @{ Name = 'Vosk runtime';  Url = 'https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip'; Dest = 'lib';       Marker = 'libvosk.dll' },
    @{ Name = 'French model';  Url = 'https://alphacephei.com/vosk/models/vosk-model-small-fr-0.22.zip';                     Dest = 'models\fr'; Marker = 'am' },
    @{ Name = 'English model'; Url = 'https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip';                  Dest = 'models\en'; Marker = 'am' }
)

foreach ($item in $items) {
    $dest = Join-Path $root $item.Dest
    if (Test-Path (Join-Path $dest $item.Marker)) { continue }

    Write-Host "Downloading $($item.Name)..."
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("mimo-vosk-" + [IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Force $tmp | Out-Null
    try {
        $zip = Join-Path $tmp 'download.zip'
        Invoke-WebRequest -Uri $item.Url -OutFile $zip -UseBasicParsing
        Expand-Archive $zip -DestinationPath (Join-Path $tmp 'out')

        # Each archive holds a single top-level folder; its contents are what we keep.
        $inner = Get-ChildItem (Join-Path $tmp 'out') -Directory | Select-Object -First 1
        New-Item -ItemType Directory -Force $dest | Out-Null
        Get-ChildItem $inner.FullName | Move-Item -Destination $dest -Force

        # The runtime ships an import library and header for linking; Mimo
        # loads the DLL at runtime instead, so they're just dead weight.
        Remove-Item (Join-Path $dest 'libvosk.lib'), (Join-Path $dest 'vosk_api.h') -ErrorAction SilentlyContinue
    }
    finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

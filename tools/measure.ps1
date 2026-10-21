# Measures a built app on Windows, as tools/measure.sh does on Linux and
# macOS: size, compressed size, time to the first frame, unlock time, memory
# once the vault is open, and a screenshot.
#
#   tools/measure.ps1 -Exe <passbundle.exe> -Bundle <Release folder> -Shot <screenshot.png>
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [Parameter(Mandatory = $true)] [string] $Bundle,
    [Parameter(Mandatory = $true)] [string] $Shot
)
$ErrorActionPreference = 'Stop'

# The sample vault (the core's test vectors) kept as a device keeps one, so
# the app opens locked and unlocks it by itself, offline.
$data = Join-Path ([IO.Path]::GetTempPath()) ('pb-measure-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $data | Out-Null
$sample = Join-Path $PSScriptRoot '..\app\test\sample'
Copy-Item (Join-Path $sample 'settings.json'), (Join-Path $sample 'sync.json') $data
$lines = [IO.Path]::GetTempFileName()
$env:PB_MEASURE = '1'
$env:PB_MEASURE_FILE = $lines
$env:PB_MEASURE_PASSWORD = 'correct horse battery staple 1'
$env:PB_DATA_DIR = $data
$env:PB_SECRETS = 'memory'
$started = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000
$app = Start-Process -FilePath $Exe -PassThru
$ready = $false
for ($i = 0; $i -lt 120; $i++) {
    if ((Get-Content $lines -ErrorAction SilentlyContinue) -match '^ready') { $ready = $true; break }
    if ($app.HasExited) { throw "The app quit before it was ready: $(Get-Content $lines -Raw)" }
    Start-Sleep -Milliseconds 500
}
if (-not $ready) { Stop-Process -Id $app.Id; throw "The app wasn't ready within a minute: $(Get-Content $lines -Raw)" }

$text = Get-Content $lines
$first = [long](($text | Where-Object { $_ -match '^first-frame' }) -split ' ')[1]
$unlock = (($text | Where-Object { $_ -match '^unlocked' }) -split ' ')[1]
$process = Get-Process -Id $app.Id
$process.Refresh()
$workingSet = $process.WorkingSet64
$private = $process.PrivateMemorySize64

try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
    [System.Drawing.Graphics]::FromImage($bitmap).CopyFromScreen($screen.Location, [System.Drawing.Point]::Empty, $screen.Size)
    $bitmap.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
} catch {
    Write-Warning "No screenshot: $($_.Exception.Message)"
}
Stop-Process -Id $app.Id

$size = (Get-ChildItem $Bundle -Recurse -File | Measure-Object -Property Length -Sum).Sum
$zip = Join-Path ([IO.Path]::GetTempPath()) 'passbundle-measure.zip'
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $Bundle '*') -DestinationPath $zip
$packed = (Get-Item $zip).Length
function Mb([double] $bytes) { '{0:N1} MB' -f ($bytes / 1MB) }

@"
| | Windows |
|---|---|
| App on disk | $(Mb $size) |
| Compressed | $(Mb $packed) |
| First frame after launch | $([math]::Round(($first - $started) / 1000)) ms |
| Unlock (PBKDF2, 600,000 iterations) | $unlock ms |
| Memory, vault open (working set) | $(Mb $workingSet) |
| Memory, vault open (private) | $(Mb $private) |
"@

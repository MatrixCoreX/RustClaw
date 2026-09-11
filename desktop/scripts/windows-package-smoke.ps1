param([string]$Installer, [string]$Evidence, [string]$ExpectedBinary)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'disposable_ci_runner_required' }
$installDir = Join-Path $Evidence 'Installed app 测试'
$install = Start-Process -FilePath $Installer -ArgumentList @('/S', "/D=$installDir") -PassThru -Wait
if ($install.ExitCode -ne 0) { throw "installer_failed_$($install.ExitCode)" }
$binary = Join-Path $installDir 'agent-desktop.exe'
@{ installed = (Get-FileHash $binary).Hash; expected = (Get-FileHash $ExpectedBinary).Hash } |
  ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Evidence 'installed-payload.json')
if ((Get-FileHash $binary).Hash -ne (Get-FileHash $ExpectedBinary).Hash) { throw 'installed_binary_mismatch' }
$app = Start-Process -FilePath $binary -PassThru
try {
  for ($attempt = 0; $attempt -lt 30; $attempt++) {
    Start-Sleep -Seconds 1
    $app.Refresh()
    if ($app.HasExited) { throw 'application_exited' }
    if ($app.MainWindowHandle -ne 0) { break }
  }
  if ($app.MainWindowHandle -eq 0) { throw 'application_window_missing' }
  @{ title = $app.MainWindowTitle; pid = $app.Id; visible = $true } |
    ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Evidence 'window.json')
  Start-Sleep -Seconds 3
  Add-Type -AssemblyName System.Windows.Forms
  Add-Type -AssemblyName System.Drawing
  $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $image = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
  $graphics = [System.Drawing.Graphics]::FromImage($image)
  try {
    $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $image.Size)
    $image.Save((Join-Path $Evidence 'desktop.png'), [System.Drawing.Imaging.ImageFormat]::Png)
  } finally { $graphics.Dispose(); $image.Dispose() }
} finally { if (-not $app.HasExited) { Stop-Process -Id $app.Id } }

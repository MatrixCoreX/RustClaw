$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'disposable_ci_runner_required' }
$clients = @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients', 'HKCU:\Software\Microsoft\EdgeUpdate\Clients')
$runtime = $clients | Where-Object { Test-Path $_ } | ForEach-Object {
  Get-ChildItem $_ | Get-ItemProperty | Where-Object { $_.name -like '*WebView2*' }
} | Select-Object -First 1
$version = $runtime.pv
if ($version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'webview2_version_unavailable' }
$tools = Join-Path $PWD '.build/tools'
New-Item -ItemType Directory -Path $tools -Force | Out-Null
$archive = Join-Path $tools "edgedriver-$version.zip"
Invoke-WebRequest "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip" -OutFile $archive
Expand-Archive $archive -DestinationPath $tools -Force
& (Join-Path $tools 'msedgedriver.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'webdriver_unavailable' }
choco install ffmpeg --yes --no-progress
if ($LASTEXITCODE -ne 0) { throw 'fixture_media_dependency_unavailable' }

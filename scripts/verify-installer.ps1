param([string]$SetupPath)
$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $SetupPath) { $SetupPath = Join-Path $projectRoot 'release/SuperCode_0.1.0_x64-setup.exe' }
$setup = [System.IO.Path]::GetFullPath($SetupPath)
if (-not (Test-Path -LiteralPath $setup)) { throw '请先生成安装包。' }
$testRoot = Join-Path $projectRoot ('.supercode/installer-verification-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$testTarget = [System.IO.Path]::GetFullPath((Join-Path $testRoot '中文目录 with spaces/SuperCode'))
if (-not $testTarget.StartsWith($projectRoot + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) { throw '隔离测试目录超出了项目范围。' }
New-Item -ItemType Directory -Path $testTarget -Force | Out-Null
$formal = Join-Path $env:LOCALAPPDATA 'SuperCode/supercode.exe'
$formalHash = if (Test-Path -LiteralPath $formal) { (Get-FileHash -LiteralPath $formal -Algorithm SHA256).Hash } else { $null }
$formalUninstaller = Join-Path $env:LOCALAPPDATA 'SuperCode/uninstall.exe'
$uninstallerHash = if (Test-Path -LiteralPath $formalUninstaller) { (Get-FileHash -LiteralPath $formalUninstaller -Algorithm SHA256).Hash } else { $null }
$registration = Get-ItemProperty -LiteralPath 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/SuperCode' -ErrorAction SilentlyContinue
$oldLocation = $registration.InstallLocation
$manifest = Get-Content -LiteralPath (Join-Path $projectRoot 'installer/generated/manifest.json') -Encoding utf8 -Raw | ConvertFrom-Json
function Install-TestTarget {
    $process = Start-Process -FilePath $setup -ArgumentList ('/VERIFY="' + $testTarget + '"') -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(45000)) { throw '安装器隔离验证超时。' }
    if ($process.ExitCode -ne 0) {
        $errorPath = Join-Path $testTarget 'verification-error.json'
        if (Test-Path -LiteralPath $errorPath) { throw (Get-Content -LiteralPath $errorPath -Encoding utf8 -Raw) }
        throw ('安装器退出码：' + $process.ExitCode)
    }
    foreach ($file in $manifest.entries) {
        if ($file.name -eq 'registration.exe') { continue }
        $actual = (Get-FileHash -LiteralPath (Join-Path $testTarget $file.name) -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $file.sha256) { throw ('安装文件校验失败：' + $file.name) }
    }
    $verification = Get-Content -LiteralPath (Join-Path $testTarget 'verification.json') -Encoding utf8 -Raw | ConvertFrom-Json
    if (-not $verification.isolated -or $verification.events[-1].percent -ne 100) { throw '没有收到真实安装完成事件。' }
    return $verification
}
$fresh = Install-TestTarget
# Reinstall uses the same production engine and retains the previous program in a backup.
$upgrade = Install-TestTarget
$previousExe = Join-Path $upgrade.installed.backup 'supercode.exe'
if ((Get-FileHash -LiteralPath $previousExe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.entries[0].sha256) { throw '更新没有保留原程序备份。' }
$uninstaller = Join-Path $testTarget 'uninstall.exe'
$uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/S' -WindowStyle Hidden -PassThru
if (-not $uninstall.WaitForExit(45000)) { throw '卸载验证超时。' }
if ($uninstall.ExitCode -ne 0) { throw ('卸载退出码：' + $uninstall.ExitCode) }
# NSIS starts a temporary uninstaller child; the original launcher may exit first.
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (((Test-Path -LiteralPath (Join-Path $testTarget 'supercode.exe')) -or (Test-Path -LiteralPath $uninstaller)) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
}
if (Test-Path -LiteralPath (Join-Path $testTarget 'supercode.exe')) { throw '卸载后程序文件仍存在。' }
if (Test-Path -LiteralPath $uninstaller) { throw '卸载子进程尚未结束。' }
if (-not (Test-Path -LiteralPath $previousExe)) { throw '卸载意外删除了保留的版本备份。' }
if ($formalHash -and (Get-FileHash -LiteralPath $formal -Algorithm SHA256).Hash -ne $formalHash) { throw '隔离测试改变了正式程序。' }
if ($uninstallerHash -and (Get-FileHash -LiteralPath $formalUninstaller -Algorithm SHA256).Hash -ne $uninstallerHash) { throw '隔离测试改变了正式卸载器。' }
$currentRegistration = Get-ItemProperty -LiteralPath 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/SuperCode' -ErrorAction SilentlyContinue
if ($currentRegistration.InstallLocation -ne $oldLocation) { throw '隔离测试改变了正式注册信息。' }
$report = [ordered]@{ setup = $setup; isolatedDirectory = $testTarget; freshInstall = $true; upgrade = $true; oldBinaryBackedUp = $true; uninstall = $true; productionUnchanged = $true; progressEvents = $fresh.events.Count; appSha256 = $manifest.entries[0].sha256 }
$json = $report | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText((Join-Path $testRoot 'report.json'), $json + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
Write-Output $json

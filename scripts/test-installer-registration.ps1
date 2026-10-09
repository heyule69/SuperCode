$ErrorActionPreference = 'Stop'
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testRoot = Join-Path $root ('.supercode/registration-test-' + [guid]::NewGuid().ToString())
if (-not $testRoot.StartsWith($root + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) { throw '测试路径越界。' }
New-Item -ItemType Directory -Path $testRoot | Out-Null
$target = Join-Path $testRoot '旧版安装目录 with spaces'
New-Item -ItemType Directory -Path $target | Out-Null
$keyName = 'Software\SuperCode\InstallerTests\' + [guid]::NewGuid().ToString()
$keyPath = 'HKCU:/' + $keyName.Replace('\', '/')
$desktop = Join-Path $testRoot 'desktop.lnk'
$programs = Join-Path $testRoot 'programs.lnk'
$worker = Join-Path $testRoot 'registration.exe'
$icon = Join-Path $root 'src-tauri/icons/icon.ico'
$iconName = 'supercode-icon-' + (Get-FileHash -LiteralPath $icon -Algorithm SHA256).Hash.Substring(0, 12).ToLowerInvariant() + '.ico'
$version = (Get-Content -LiteralPath (Join-Path $root 'package.json') -Encoding utf8 -Raw | ConvertFrom-Json).version
$nsis = if ($env:SUPERCODE_NSIS) { $env:SUPERCODE_NSIS } else { Join-Path $env:LOCALAPPDATA 'tauri/NSIS/makensis.exe' }
$formalKey = 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/SuperCode'
function Read-FormalRegistration {
    $values = [ordered]@{}
    $registration = Get-ItemProperty -LiteralPath $formalKey -ErrorAction SilentlyContinue
    if ($registration) {
        foreach ($property in $registration.PSObject.Properties) {
            if ($property.Name -notlike 'PS*') { $values[$property.Name] = $property.Value }
        }
    }
    return ($values | ConvertTo-Json -Depth 3)
}
$formalBefore = Read-FormalRegistration
$shell = New-Object -ComObject WScript.Shell
function Invoke-Registration {
    $start = [System.Diagnostics.ProcessStartInfo]::new($worker)
    $start.ArgumentList.Add('/S')
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.Environment['SUPERCODE_INSTALL_TARGET'] = $target
    $process = [System.Diagnostics.Process]::Start($start)
    if (-not $process.WaitForExit(15000)) { throw '注册入口验证超时。' }
    return $process.ExitCode
}
try {
    $options = @('/INPUTCHARSET','UTF8',"/DOUTPUT=$worker","/DICON_SOURCE=$icon","/DICON_NAME=$iconName","/DVERSION=$version",'/DSIZE_KB=100',"/DKEY=$keyName","/DDESKTOP_LINK=$desktop","/DPROGRAMS_LINK=$programs")
    & $nsis @options (Join-Path $root 'installer/registration.nsi') | Out-Null
    if ($LASTEXITCODE -ne 0) { throw '测试注册入口编译失败。' }
    Copy-Item -LiteralPath (Join-Path $root 'src-tauri/target/release/supercode.exe') -Destination (Join-Path $target 'supercode.exe')
    Copy-Item -LiteralPath $icon -Destination (Join-Path $target $iconName)
    # Reproduce the legacy NSIS registry shape: a quoted path, no default value.
    New-Item -Path $keyPath -Force | Out-Null
    New-ItemProperty -LiteralPath $keyPath -Name InstallLocation -Value ('"' + $target + '"') -PropertyType String | Out-Null
    foreach ($link in @($desktop,$programs)) {
        $shortcut = $shell.CreateShortcut($link)
        $shortcut.TargetPath = Join-Path $target 'supercode.exe'
        $shortcut.Save()
    }
    if ((Invoke-Registration) -ne 0) { throw '带引号路径的旧版无法直接升级。' }
    if ((Get-ItemProperty -LiteralPath $keyPath).DisplayVersion -ne $version) { throw '旧版注册信息没有更新。' }
    foreach ($link in @($desktop,$programs)) {
        if ($shell.CreateShortcut($link).TargetPath -ne (Join-Path $target 'supercode.exe')) { throw '升级破坏了快捷方式目标。' }
        if ($shell.CreateShortcut($link).IconLocation -notlike "*$iconName*") { throw '快捷方式没有刷新图标。' }
    }
    if ((Invoke-Registration) -ne 0) { throw '已安装版本无法重复安装更新。' }
    if (-not (Test-Path -LiteralPath (Join-Path $target 'uninstall.exe'))) { throw '卸载入口未创建。' }
    $report = [ordered]@{version=$version; quotedLegacyPath=$true; ownShortcutsUpdated=$true; repeatUpgrade=$true; scopedRegistry=$keyName; productionUnchanged=$true}
    [System.IO.File]::WriteAllText((Join-Path $testRoot 'report.json'), ($report | ConvertTo-Json) + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
    $report | ConvertTo-Json
} finally {
    if ($keyName.StartsWith('Software\SuperCode\InstallerTests\') -and (Test-Path -LiteralPath $keyPath)) { Remove-Item -LiteralPath $keyPath -Recurse -Force }
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($shell) | Out-Null
    $formalAfter = Read-FormalRegistration
    if ($formalBefore -ne $formalAfter) { throw '隔离测试改变了正式卸载注册信息。' }
}

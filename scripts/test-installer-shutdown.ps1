$ErrorActionPreference = 'Stop'
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testRoot = Join-Path $root ('.supercode/shutdown-test-' + [guid]::NewGuid().ToString())
if (-not $testRoot.StartsWith($root + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) { throw '测试路径越界。' }
New-Item -ItemType Directory -Path $testRoot | Out-Null
$source = Join-Path $root 'src-tauri/target/release/supercode.exe'
$installer = Join-Path $root 'src-tauri/target/release/supercode-installer.exe'
$formal = Join-Path $env:LOCALAPPDATA 'SuperCode/supercode.exe'
$formalHash = if (Test-Path -LiteralPath $formal) { (Get-FileHash -LiteralPath $formal -Algorithm SHA256).Hash } else { $null }
function Start-Hidden([string]$File, [string[]]$Arguments) {
    $start = [System.Diagnostics.ProcessStartInfo]::new($File)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WorkingDirectory = $root
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    return [System.Diagnostics.Process]::Start($start)
}
function Wait-File([string]$Path) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (Test-Path -LiteralPath $Path)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw ('没有收到测试就绪事件：' + $Path) }
        Start-Sleep -Milliseconds 100
    }
}
$results = @()
foreach ($legacy in @($false,$true)) {
    foreach ($busy in @($false,$true)) {
        $name = ('legacy-' + $legacy + '-busy-' + $busy)
        $target = Join-Path $testRoot ($name + '/中文路径 with spaces')
        New-Item -ItemType Directory -Path $target -Force | Out-Null
        $exe = Join-Path $target 'supercode.exe'
        Copy-Item -LiteralPath $source -Destination $exe
        $ready = Join-Path $testRoot ($name + '.json')
        $mode = if ($legacy) { '--legacy-installation-exit-test' } else { '--installation-exit-test' }
        $arguments = @($mode,$ready)
        if ($busy) { $arguments += '--busy' }
        $app = Start-Hidden $exe $arguments
        Wait-File $ready
        $fixture = Get-Content -LiteralPath $ready -Encoding utf8 -Raw | ConvertFrom-Json
        if (-not $fixture.isolated -or $fixture.pid -ne $app.Id) { throw '没有启动隔离的原生退出测试。' }
        $before = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
        $upgrade = Start-Hidden $installer @('--verify-install',$target,$fixture.data)
        if (-not $upgrade.WaitForExit(40000)) { throw '运行中升级测试超时。' }
        if ($busy) {
            if ($upgrade.ExitCode -eq 0 -or $app.HasExited) { throw '仍有任务时错误地退出了软件。' }
            if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash -ne $before) { throw '忙碌任务期间程序被覆盖。' }
            $errorText = Get-Content -LiteralPath (Join-Path $target 'verification-error.json') -Encoding utf8 -Raw
            if ($errorText -notmatch '任务正在运行') { throw ('没有明确提示运行中的任务：' + $errorText) }
            [System.IO.File]::WriteAllText([System.IO.Path]::ChangeExtension($ready,'resume'), 'resume', [System.Text.UTF8Encoding]::new($false))
            Wait-File ([System.IO.Path]::ChangeExtension($ready,'idle'))
            $upgrade = Start-Hidden $installer @('--verify-install',$target,$fixture.data)
            if (-not $upgrade.WaitForExit(40000)) { throw '任务完成后升级超时。' }
        }
        if ($upgrade.ExitCode -ne 0) { throw (Get-Content -LiteralPath (Join-Path $target 'verification-error.json') -Encoding utf8 -Raw) }
        if (-not $app.WaitForExit(5000)) { throw '安装器没有自动退出原生软件。' }
        if (-not $legacy -and -not (Test-Path -LiteralPath (Join-Path $fixture.data 'before-exit.json'))) { throw '没有经过保存后退出流程。' }
        $verify = @'
import pathlib, sqlite3, sys
path = pathlib.Path(sys.argv[1]) / 'supercode.db'
db = sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)
assert db.execute("select value from settings where key='installation-exit-sentinel'").fetchone()[0] == '保留中文配置'
assert db.execute("select count(*) from sessions where title='安装退出测试：保留会话'").fetchone()[0] == 1
'@
        & python -X utf8 -c $verify $fixture.data
        if ($LASTEXITCODE -ne 0) { throw '升级改变了会话或配置。' }
        $results += [ordered]@{legacy=$legacy; busyTaskProtected=$busy; automaticQuit=$true; savedExit=(-not $legacy); userDataPreserved=$true}
    }
}
if ($formalHash -and (Get-FileHash -LiteralPath $formal -Algorithm SHA256).Hash -ne $formalHash) { throw '测试改变了正式程序。' }
$report = [ordered]@{native=$true; productionUnchanged=$true; directory=$testRoot; cases=$results}
[System.IO.File]::WriteAllText((Join-Path $testRoot 'report.json'), ($report | ConvertTo-Json -Depth 5) + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
$report | ConvertTo-Json -Depth 5

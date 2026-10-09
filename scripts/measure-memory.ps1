param([Parameter(Mandatory=$true)][int]$SuperCodeProcessId)
$ErrorActionPreference = 'Stop'
$supercodeProcesses = @(Get-CimInstance Win32_Process)
$supercodeRoot = $supercodeProcesses | Where-Object { $_.ProcessId -eq $SuperCodeProcessId }
if (!$supercodeRoot -or $supercodeRoot.Name -ne 'supercode.exe') { throw '请指定正在运行的 SuperCode 进程 ID。' }
$supercodeIds = [System.Collections.Generic.HashSet[int]]::new()
[void]$supercodeIds.Add($SuperCodeProcessId)
for ($supercodePass = 0; $supercodePass -lt 20; $supercodePass++) {
    $supercodeAdded = $false
    foreach ($supercodeProcess in $supercodeProcesses) {
        if ($supercodeIds.Contains([int]$supercodeProcess.ParentProcessId)) {
            if ($supercodeIds.Add([int]$supercodeProcess.ProcessId)) { $supercodeAdded = $true }
        }
    }
    if (!$supercodeAdded) { break }
}
$supercodePrivateCounters = @(Get-CimInstance Win32_PerfRawData_PerfProc_Process)
$supercodeRows = @($supercodeProcesses | Where-Object { $supercodeIds.Contains([int]$_.ProcessId) } | ForEach-Object {
    $supercodeCurrent = $_
    $supercodePrivate = $supercodePrivateCounters | Where-Object { $_.IDProcess -eq $supercodeCurrent.ProcessId } | Select-Object -First 1
    [pscustomobject]@{
        pid = $_.ProcessId
        parent = $_.ParentProcessId
        name = $_.Name
        workingSetBytes = [long]$_.WorkingSetSize
        privateWorkingSetBytes = if ($supercodePrivate) { [long]$supercodePrivate.WorkingSetPrivate } else { $null }
    }
})
$supercodeReport = [pscustomobject]@{
    timestamp = [DateTime]::UtcNow.ToString('o')
    rootPid = $SuperCodeProcessId
    workingSetMiB = [math]::Round(($supercodeRows | Measure-Object workingSetBytes -Sum).Sum / 1MB, 1)
    privateWorkingSetMiB = if (@($supercodeRows | Where-Object { $null -eq $_.privateWorkingSetBytes }).Count -eq 0) {
        [math]::Round(($supercodeRows | Measure-Object privateWorkingSetBytes -Sum).Sum / 1MB, 1)
    } else { $null }
    processes = $supercodeRows
}
$supercodeReport | ConvertTo-Json -Depth 4

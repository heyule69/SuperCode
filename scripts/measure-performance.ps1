param(
  [Parameter(Mandatory=$true)][int]$SuperCodeProcessId,
  [Parameter(Mandatory=$true)][ValidateSet('startup','idle','parallel','long-chat')][string]$Scenario,
  [ValidateRange(3,120)][int]$Samples = 10,
  [ValidateRange(100,5000)][int]$IntervalMs = 1000,
  [string]$OutputPath,
  [string]$CompareWith
)
$ErrorActionPreference = 'Stop'
# Observe only the explicitly selected existing process. Never start an app or agent.
$taskSnapshots = [System.Collections.Generic.List[object]]::new()
$taskPreviousCpu = @{}
$taskCpuSecondsObserved = 0.0
for ($taskSample = 0; $taskSample -lt $Samples; $taskSample++) {
  $taskSnapshot = & (Join-Path $PSScriptRoot 'measure-memory.ps1') -SuperCodeProcessId $SuperCodeProcessId | ConvertFrom-Json
  foreach ($taskProcess in $taskSnapshot.processes) {
    $taskNative = Get-Process -Id $taskProcess.pid -ErrorAction SilentlyContinue
    if ($taskNative) {
      $taskCpu = $taskNative.TotalProcessorTime.TotalSeconds
      $taskIdentity = "$($taskProcess.pid):$($taskNative.StartTime.Ticks)"
      if ($taskPreviousCpu.ContainsKey($taskIdentity)) { $taskCpuSecondsObserved += [Math]::Max(0,$taskCpu-$taskPreviousCpu[$taskIdentity]) }
      elseif ($taskSample -gt 0 -and $taskNative.StartTime.ToUniversalTime() -ge [DateTime]$taskSnapshots[0].timestamp) { $taskCpuSecondsObserved += $taskCpu }
      $taskPreviousCpu[$taskIdentity] = $taskCpu
    }
  }
  $taskSnapshots.Add([pscustomobject]@{ timestamp=$taskSnapshot.timestamp; cpuSecondsObserved=$taskCpuSecondsObserved; workingSetMiB=$taskSnapshot.workingSetMiB; privateWorkingSetMiB=$taskSnapshot.privateWorkingSetMiB; processes=$taskSnapshot.processes.Count })
  if ($taskSample + 1 -lt $Samples) { Start-Sleep -Milliseconds $IntervalMs }
}
$taskPrivate = @($taskSnapshots | Where-Object { $null -ne $_.privateWorkingSetMiB } | ForEach-Object { $_.privateWorkingSetMiB }) | Sort-Object
$taskElapsed = ([DateTime]$taskSnapshots[-1].timestamp - [DateTime]$taskSnapshots[0].timestamp).TotalSeconds
$taskReport = [ordered]@{
  scenario=$Scenario; rootPid=$SuperCodeProcessId; timestamp=[DateTime]::UtcNow.ToString('o'); samples=$Samples;
  peakWorkingSetMiB=($taskSnapshots | Measure-Object workingSetMiB -Maximum).Maximum;
  peakPrivateWorkingSetMiB=if($taskPrivate.Count){$taskPrivate[-1]}else{$null};
  p95PrivateWorkingSetMiB=if($taskPrivate.Count){$taskPrivate[[Math]::Max(0,[Math]::Ceiling($taskPrivate.Count*0.95)-1)]}else{$null};
  cpuCorePercent=if($taskElapsed -gt 0){[Math]::Round($taskCpuSecondsObserved/$taskElapsed*100,2)}else{$null};
  snapshots=$taskSnapshots
}
if ($CompareWith) {
  $taskBaseline = Get-Content -LiteralPath $CompareWith -Raw -Encoding UTF8 | ConvertFrom-Json
  if ($taskBaseline.scenario -ne $Scenario) { throw '只能比较同一场景的基线。' }
  $taskReport['privateMemoryDeltaMiB'] = if($null -ne $taskReport.peakPrivateWorkingSetMiB -and $null -ne $taskBaseline.peakPrivateWorkingSetMiB){[Math]::Round($taskReport.peakPrivateWorkingSetMiB-$taskBaseline.peakPrivateWorkingSetMiB,2)}else{$null}
}
$taskJson = $taskReport | ConvertTo-Json -Depth 5
if ($OutputPath) {
  $taskAbsoluteOutput = [System.IO.Path]::GetFullPath($OutputPath)
  [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($taskAbsoluteOutput))
  [System.IO.File]::WriteAllText($taskAbsoluteOutput, $taskJson, [System.Text.UTF8Encoding]::new($false))
}
$taskJson

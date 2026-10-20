# MemCleaner 管理员权限统一验证脚本（一次 UAC 完成全部验证）
#
# 用法（普通 PowerShell 中执行，UAC 弹窗点「是」）：
#   Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',"$PWD\tools\verify-elevated.ps1"
#   或：右键本文件 → 以管理员身份运行（Windows PowerShell）
#
# 脚本以管理员身份做以下事情（全部只读/可回滚）：
#   0. 结束所有 MemCleaner 实例（含一个卡住的旧实例），释放文件占用
#   1. BaselineProbe --diag --self-trim：独立验证 NtSetSystemInformation(80,{2,3,4}) 三条命令
#   2. 真实 PCL `--memory` 端到端：记录退出码（= 释放 KB）与耗时，作为效果基准
#   3. MemCleaner --self-test：验证本程序自身清理效果（两次，冷/热）
#   4. 计划任务自启的安装/查询/卸载验证
#   5. 管理员模式 GUI 端到端：截图 → 点击「清理内存」→ 等待 → 截图/读取日志与配置 → 采样内存占用
#   6. 汇总输出到 docs\verify\ 与 docs\verify-elevated.md
#
# 安全说明：内存清理只回收可回收缓存（standby/工作集/脏页），不改注册表、不终止其它进程；
# 短暂清空文件缓存后读盘会略慢，属预期。计划任务在验证后会被卸载（恢复原状）。

#Requires -Version 5.1
$ErrorActionPreference = 'Continue'

$Root       = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$DistExe    = Join-Path $Root 'dist\MemCleaner.exe'

$BuildExe1  = Join-Path $Root 'target\release\MemCleaner.exe'
$AppExe     = if (Test-Path $DistExe) { $DistExe } else { $BuildExe1 }
$Probe      = Join-Path $Root 'probes\BaselineProbe\bin\Release\net8.0\BaselineProbe.exe'
$PclExe     = Join-Path $Root 'Plain Craft Launcher 2.exe'   # 可选：用于同场对比，不存在则跳过
$OutDir     = Join-Path $Root 'docs\verify'
$DataDir    = Join-Path $Root 'testdata\verify'
$ShotDir    = Join-Path $Root 'docs\screenshots'
$TaskName   = 'MemCleaner AutoStart'

New-Item -ItemType Directory -Force -Path $OutDir, $DataDir, $ShotDir | Out-Null

$summary = [ordered]@{
    timestamp = (Get-Date).ToString('s')
    elevated  = $false
    os        = (Get-CimInstance Win32_OperatingSystem).Caption + ' build ' + (Get-CimInstance Win32_OperatingSystem).BuildNumber
    app_exe   = $AppExe
    steps     = [ordered]@{}
    memory    = [ordered]@{}
    errors    = @()
}

function Write-Step([string]$name, $data) {
    $summary.steps[$name] = $data
    Write-Host ("[{0}] {1}" -f $name, ($data | ConvertTo-Json -Compress -Depth 6)) -ForegroundColor Cyan
}

function Get-Mem {
    if (-not ('McMem' -as [type])) {
        Add-Type @"
using System; using System.Runtime.InteropServices;
public class McMem {
  [StructLayout(LayoutKind.Sequential)] public struct MEMORYSTATUSEX { public uint dwLength; public uint dwMemoryLoad; public ulong ullTotalPhys; public ulong ullAvailPhys; public ulong ullTotalPageFile; public ulong ullAvailPageFile; public ulong ullTotalVirtual; public ulong ullAvailVirtual; public ulong ullAvailExtendedVirtual; }
  [DllImport("kernel32.dll")] public static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX s);
  public static long Avail() { var s = new MEMORYSTATUSEX(); s.dwLength = (uint)Marshal.SizeOf(s); GlobalMemoryStatusEx(ref s); return (long)s.ullAvailPhys; }
  public static long Load()  { var s = new MEMORYSTATUSEX(); s.dwLength = (uint)Marshal.SizeOf(s); GlobalMemoryStatusEx(ref s); return (long)s.dwMemoryLoad; }
}
"@
    }
    [pscustomobject]@{ avail = [McMem]::Avail(); load = [McMem]::Load() }
}

# ---- 提权自检 ----
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$isElevated = (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$summary.elevated = $isElevated
Write-Host "=== MemCleaner 管理员验证 ===" -ForegroundColor Green
Write-Host ("用户: {0}  提权: {1}  exe: {2}" -f $identity.Name, $isElevated, $AppExe)
if (-not $isElevated) {
    Write-Host '错误：必须以管理员身份运行本脚本。' -ForegroundColor Red
    exit 1
}
if (-not (Test-Path $AppExe)) {
    Write-Host "错误：找不到 $AppExe" -ForegroundColor Red
    exit 1
}

# =====================================================================
# 0. 结束所有 MemCleaner 实例（释放被锁的 exe）
# =====================================================================
Write-Host ''
Write-Host '=== [0/6] 结束旧 MemCleaner 实例 ===' -ForegroundColor Cyan
Get-Process MemCleaner -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host ("  kill pid={0}" -f $_.Id)
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Milliseconds 800
Write-Step 'kill_instances' @{ remaining = @(Get-Process MemCleaner -ErrorAction SilentlyContinue).Count }

# =====================================================================
# 1. BaselineProbe：独立验证三条内核命令
# =====================================================================
Write-Host ''
Write-Host '=== [1/6] BaselineProbe 独立验证 NtSetSystemInformation(80,2/3/4) ===' -ForegroundColor Cyan
if (Test-Path $Probe) {
    $tag = 'verify-' + (Get-Date).ToString('HHmmss')
    & $Probe --diag --self-trim --tag $tag *>&1 | Tee-Object -FilePath (Join-Path $OutDir 'baseline-probe.log')
    $probeExit = $LASTEXITCODE
    $latest = Get-ChildItem (Join-Path $Root 'tools\baseline-results') -Filter "*$tag*.json" -File |
              Sort-Object LastWriteTime | Select-Object -Last 1
    $probeJson = $null
    if ($latest) { $probeJson = Get-Content $latest.FullName -Raw | ConvertFrom-Json }
    Write-Step 'baseline_probe' @{
        exit_code = $probeExit
        json      = if ($latest) { $latest.FullName } else { $null }
        steps     = if ($probeJson) { $probeJson.Iterations[0].Steps } else { $null }
        standby_before = if ($probeJson) { $probeJson.SystemMemoryListBefore.StandbyTotalBytes } else { $null }
        standby_after  = if ($probeJson) { $probeJson.SystemMemoryListAfter.StandbyTotalBytes } else { $null }
        modified_before = if ($probeJson) { $probeJson.SystemMemoryListBefore.ModifiedPageBytes } else { $null }
        modified_after  = if ($probeJson) { $probeJson.SystemMemoryListAfter.ModifiedPageBytes } else { $null }
    }
} else {
    Write-Host '  BaselineProbe 不存在，跳过（不影响主验证）' -ForegroundColor Yellow
    Write-Step 'baseline_probe' @{ skipped = 'probe not found' }
}

# =====================================================================
# 2. 真实 PCL --memory 端到端（效果基准）
# =====================================================================
Write-Host ''
Write-Host '=== [2/6] PCL --memory 端到端（效果基准） ===' -ForegroundColor Cyan
$pclRuns = @()
if (Test-Path $PclExe) {
    foreach ($i in 1..2) {
        $before = Get-Mem
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process -FilePath $PclExe -ArgumentList '--memory' -Wait -PassThru
        $sw.Stop()
        Start-Sleep -Milliseconds 800
        $after = Get-Mem
        $run = [pscustomobject]@{
            run = $i
            exit_code = $p.ExitCode
            freed_kb_from_exitcode = $p.ExitCode
            freed_bytes_by_gms = ($after.avail - $before.avail)
            before_avail = $before.avail
            after_avail = $after.avail
            elapsed_ms = $sw.ElapsedMilliseconds
        }
        $pclRuns += $run
        Write-Host ("  run{0}: exit={1} (KB)  GMS: {2:N1} MB -> {3:N1} MB  elapsed={4}ms" -f `
            $i, $run.exit_code, ($run.before_avail / 1MB), ($run.after_avail / 1MB), $run.elapsed_ms)
        Start-Sleep -Seconds 1
    }
} else { Write-Host '  PCL 不存在，跳过' -ForegroundColor Yellow }
Write-Step 'pcl_memory' $pclRuns

# =====================================================================
# 3. MemCleaner --self-test（本程序效果，冷/热两次）
# =====================================================================
Write-Host ''
Write-Host '=== [3/6] MemCleaner --self-test ===' -ForegroundColor Cyan
$selfTests = @()
foreach ($i in 1..2) {
    $out = Join-Path $OutDir ("selftest-{0}.json" -f $i)
    Remove-Item $out -ErrorAction SilentlyContinue
    $p = Start-Process -FilePath $AppExe -ArgumentList '--self-test', '--out', $out -Wait -PassThru `
        -RedirectStandardOutput (Join-Path $OutDir ("selftest-{0}.stdout.txt" -f $i)) `
        -RedirectStandardError  (Join-Path $OutDir ("selftest-{0}.stderr.txt" -f $i))
    $json = $null
    if (Test-Path $out) { $json = Get-Content $out -Raw | ConvertFrom-Json }
    $selfTests += [pscustomobject]@{ run = $i; exit_code = $p.ExitCode; result = $json }
    Write-Host ("  run{0}: exit={1} elevated={2} clean_success={3} freed={4} bytes duration={5}ms" -f `
        $i, $p.ExitCode, $json.elevated, $json.clean_success, $json.freed_bytes, $json.duration_ms)
    Start-Sleep -Seconds 1
}
Write-Step 'self_test' $selfTests

# =====================================================================
# 4. 开机自启计划任务 安装/查询/卸载
# =====================================================================
Write-Host ''
Write-Host '=== [4/6] 开机自启计划任务验证 ===' -ForegroundColor Cyan
$taskResult = [ordered]@{}
function Invoke-App([string[]]$appArgs) {
    $p = Start-Process -FilePath $AppExe -ArgumentList $appArgs -Wait -PassThru `
        -RedirectStandardOutput (Join-Path $OutDir 'app-cli.out.txt') `
        -RedirectStandardError  (Join-Path $OutDir 'app-cli.err.txt')
    return $p.ExitCode
}
$taskResult.install_exit = Invoke-App @('--install-autostart')
Start-Sleep -Milliseconds 500
$q1 = & schtasks /Query /TN $TaskName 2>&1 | Out-String
$taskResult.query_after_install_ok = ($LASTEXITCODE -eq 0)
$taskResult.query_snippet = ($q1 -split "`r?`n" | Select-Object -First 4) -join ' | '
$taskResult.uninstall_exit = Invoke-App @('--uninstall-autostart')
Start-Sleep -Milliseconds 500
& schtasks /Query /TN $TaskName *>&1 | Out-Null
$taskResult.query_after_uninstall_ok = ($LASTEXITCODE -eq 0)
Write-Host ("  install={0} query={1} uninstall={2} query_after={3}" -f `
    $taskResult.install_exit, $taskResult.query_after_install_ok, $taskResult.uninstall_exit, $taskResult.query_after_uninstall_ok)
Write-Step 'autostart_task' $taskResult

# =====================================================================
# 5. 管理员模式 GUI 端到端
# =====================================================================
Write-Host ''
Write-Host '=== [5/6] 管理员模式 GUI 端到端（清理 + 截图 + 内存占用） ===' -ForegroundColor Cyan

if (-not ('McGui' -as [type])) {
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class McGui {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  public static IntPtr FindMain(uint target) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h,l)=>{ uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid==target) { var c=new StringBuilder(64); GetClassName(h,c,64);
        if (c.ToString()=="MemCleanerMainWindow") { found=h; return false; } }
      return true; }, IntPtr.Zero);
    return found;
  }
  public static List<string> WindowsOf(uint target) {
    var res = new List<string>();
    EnumWindows((h,l)=>{ uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid==target) { var c=new StringBuilder(64); var t=new StringBuilder(256);
        GetClassName(h,c,64); GetWindowText(h,t,256);
        res.Add(h+"|"+c.ToString()+"|"+t.ToString()+"|"+IsWindowVisible(h)); }
      return true; }, IntPtr.Zero);
    return res;
  }
}
"@
}
Add-Type -AssemblyName System.Drawing

function Ui-Click([IntPtr]$hwnd, [double]$lx, [double]$ly) {
    # 送逻辑坐标：系统会按窗口 DPI 缩放成物理坐标（本脚本进程 DPI 不感知）
    $x = [int][Math]::Round($lx); $y = [int][Math]::Round($ly)
    $lp = [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
    [void][McGui]::PostMessage($hwnd, 0x0200, [IntPtr]::Zero, $lp); Start-Sleep -Milliseconds 60
    [void][McGui]::PostMessage($hwnd, 0x0201, [IntPtr]1, $lp);      Start-Sleep -Milliseconds 80
    [void][McGui]::PostMessage($hwnd, 0x0202, [IntPtr]::Zero, $lp); Start-Sleep -Milliseconds 400
}

function Ui-Shot([IntPtr]$hwnd, [string]$name) {
    $dpi = [McGui]::GetDpiForWindow($hwnd); if (-not $dpi) { $dpi = 96 }
    $scale = $dpi / 96.0
    $w = [int][Math]::Round(400.0 * $scale); $h = [int][Math]::Round(560.0 * $scale)
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
    $ok = [McGui]::PrintWindow($hwnd, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    $out = Join-Path $ShotDir ($name + '.png')
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Write-Host ("  shot: {0} ({1})" -f $out, $ok)
    return $out
}

# 清理测试数据目录（保留目录本身）
Remove-Item (Join-Path $DataDir 'config.json') -ErrorAction SilentlyContinue
Remove-Item (Join-Path $DataDir 'memcleaner.log') -ErrorAction SilentlyContinue

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $AppExe
$psi.UseShellExecute = $false
$psi.EnvironmentVariables['MEMCLEANER_DATA_DIR'] = $DataDir
$psi.EnvironmentVariables['MEMCLEANER_LOG'] = '1'
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$gui = [System.Diagnostics.Process]::Start($psi)
Start-Sleep -Seconds 2
$gui.Refresh()
$hwnd = [McGui]::FindMain([uint32]$gui.Id)
$guiResult = [ordered]@{ pid = $gui.Id; hwnd = $hwnd.ToInt64() }
Write-Host ("  gui pid={0} hwnd={1}" -f $gui.Id, $hwnd)
Ui-Shot $hwnd 'elev-1-main-before' | Out-Null
$before = Get-Mem
$guiResult.before_avail = $before.avail
$guiResult.before_load = $before.load

# 点击「清理内存」：逻辑坐标 (200, 395)
Ui-Click $hwnd 200 395
Start-Sleep -Seconds 3
$after = Get-Mem
$guiResult.after_avail = $after.avail
$guiResult.after_load = $after.load
$guiResult.freed_bytes_by_gms = $after.avail - $before.avail
$shot = Ui-Shot $hwnd 'elev-2-main-after'
$guiResult.after_shot = $shot

# 读取日志与配置
$logPath = Join-Path $DataDir 'memcleaner.log'
if (Test-Path $logPath) {
    $guiResult.log_tail = @(Get-Content $logPath | Select-Object -Last 8 | ForEach-Object { if ($_.Length -gt 500) { $_.Substring(0,500) + ' ...[truncated]' } else { $_ } })
}
$cfgPath = Join-Path $DataDir 'config.json'
if (Test-Path $cfgPath) {
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    $guiResult.config_last_freed_bytes = $cfg.last_freed_bytes
    $guiResult.config_clean_count = $cfg.clean_count
}
# 进程内存占用（任务管理器口径）
$gui.Refresh()
$guiResult.app_private_bytes = $gui.PrivateMemorySize64
$guiResult.app_working_set_bytes = $gui.WorkingSet64
$guiResult.app_handles = $gui.HandleCount
Write-Host ("  清理前可用 {0:N1} MB -> 清理后 {1:N1} MB (Δ {2:N1} MB)" -f `
    ($before.avail/1MB), ($after.avail/1MB), ($guiResult.freed_bytes_by_gms/1MB))
Write-Host ("  GUI 内存: Private={0:N2} MB WorkingSet={1:N2} MB Handles={2}" -f `
    ($guiResult.app_private_bytes/1MB), ($guiResult.app_working_set_bytes/1MB), $guiResult.app_handles)

# 结束 GUI
Stop-Process -Id $gui.Id -Force -ErrorAction SilentlyContinue
Write-Step 'gui_e2e' $guiResult

# =====================================================================
# 6. 汇总输出
# =====================================================================
Write-Host ''
Write-Host '=== [6/6] 汇总 ===' -ForegroundColor Cyan

# 对比：PCL 与 MemCleaner 的效果（都用各自报告的释放量）
$pclFreed = @($pclRuns | ForEach-Object { if ($_.exit_code -gt 0) { [int64]$_.exit_code * 1024 } else { 0 } } | Measure-Object -Maximum).Maximum
$selfFreed = @($selfTests | ForEach-Object { if ($_.result) { [int64]$_.result.freed_bytes } else { 0 } } | Measure-Object -Maximum).Maximum
$summary.memory = [ordered]@{
    pcl_best_freed_bytes = $pclFreed
    memcleaner_best_freed_by_selftest_bytes = $selfFreed
    memcleaner_gui_freed_bytes = $guiResult.freed_bytes_by_gms
    app_private_bytes = $guiResult.app_private_bytes
    app_working_set_bytes = $guiResult.app_working_set_bytes
    app_handles = $guiResult.app_handles
}

$jsonPath = Join-Path $OutDir 'verify-summary.json'
$summary | ConvertTo-Json -Depth 8 | Set-Content -Path $jsonPath -Encoding UTF8

$md = @()
$md += '# MemCleaner 管理员权限统一验证报告'
$md += ''
$md += ('- 时间：' + $summary.timestamp)
$md += ('- 系统：' + $summary.os)
$md += ('- 被测程序：`' + $AppExe + '`')
$md += ('- 提权：' + $summary.elevated)
$md += ''
$md += '## 独立探针（BaselineProbe，等同于 PCL 的三条内核指令）'
$bp = $summary.steps['baseline_probe']
if ($bp -and $bp.steps) {
    $md += '| 指令 | NTSTATUS |'
    $md += '|---|---|'
    foreach ($s in $bp.steps) { $md += ('| ' + $s.Name + ' | 0x' + ([uint32]$s.Status).ToString('X8') + ' |') }
    $md += ''
    $md += ('- Standby：' + $bp.standby_before + ' → ' + $bp.standby_after + ' 字节')
    $md += ('- Modified：' + $bp.modified_before + ' → ' + $bp.modified_after + ' 字节')
}
$md += ''
$md += '## 真实 PCL `--memory` 基准'
$md += '| 次数 | 退出码(KB) | GMS 实测释放 | 耗时(ms) |'
$md += '|---|---|---|---|'
foreach ($r in $pclRuns) { $md += ('| ' + $r.run + ' | ' + $r.exit_code + ' | ' + [math]::Round($r.freed_bytes_by_gms/1MB,1) + ' MB | ' + $r.elapsed_ms + ' |') }
$md += ''
$md += '## MemCleaner `--self-test`'
$md += '| 次数 | 退出码 | elevated | clean_success | freed(bytes) | duration(ms) | 步骤状态 |'
$md += '|---|---|---|---|---|---|---|'
foreach ($r in $selfTests) {
    $steps = if ($r.result) { ($r.result.steps | ForEach-Object { $_.name + '=' + $_.success }) -join ', ' } else { '' }
    $md += ('| ' + $r.run + ' | ' + $r.exit_code + ' | ' + $r.result.elevated + ' | ' + $r.result.clean_success + ' | ' + $r.result.freed_bytes + ' | ' + $r.result.duration_ms + ' | ' + $steps + ' |')
}
$md += ''
$md += '## 管理员模式 GUI 端到端'
$md += ('- 点击「清理内存」前后可用内存：' + [math]::Round($before.avail/1MB,1) + ' MB → ' + [math]::Round($after.avail/1MB,1) + ' MB（Δ ' + [math]::Round($guiResult.freed_bytes_by_gms/1MB,1) + ' MB）')
$md += ('- 程序自报释放：' + $guiResult.config_last_freed_bytes + ' 字节；clean_count=' + $guiResult.config_clean_count)
$md += ('- 日志尾部：')
foreach ($l in $guiResult.log_tail) { $md += ('  - `' + $l + '`') }
$md += ('- 进程内存：Private=' + [math]::Round($guiResult.app_private_bytes/1MB,2) + ' MB，WorkingSet=' + [math]::Round($guiResult.app_working_set_bytes/1MB,2) + ' MB，Handles=' + $guiResult.app_handles)
$md += ('- 截图：`' + $guiResult.after_shot + '`')
$md += ''
$md += '## 开机自启计划任务'
$md += ('- 安装 exit=' + $taskResult.install_exit + '，查询成功=' + $taskResult.query_after_install_ok + '，卸载 exit=' + $taskResult.uninstall_exit + '，卸载后查询成功=' + $taskResult.query_after_uninstall_ok)
$md += ''
$md += '## 结论'
$md += ('- PCL 与 MemCleaner 的效果对比：PCL 最佳释放 ' + [math]::Round($pclFreed/1MB,1) + ' MB；MemCleaner self-test 最佳 ' + [math]::Round($selfFreed/1MB,1) + ' MB；GUI 端到端 ' + [math]::Round($guiResult.freed_bytes_by_gms/1MB,1) + ' MB。')
$md += '- 三条内核指令返回 0x00000000 即代表清理成功（与 PCL 实现一致）。'
$md += ('- 原始数据：`' + $jsonPath + '`')

$mdPath = Join-Path $Root 'docs\verify-elevated.md'
$md -join "`r`n" | Set-Content -Path $mdPath -Encoding UTF8

Write-Host ''
Write-Host ("报告: {0}" -f $mdPath) -ForegroundColor Green
Write-Host ("JSON: {0}" -f $jsonPath) -ForegroundColor Green
Write-Host ''
Write-Host '完成。可以关闭此窗口。'
Start-Sleep -Seconds 2

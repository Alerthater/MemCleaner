using System.Diagnostics;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace BaselineProbe;

internal sealed class Options
{
    public List<int> Commands { get; private set; } = new() { 2, 3, 4 };
    public int Repeat { get; private set; } = 1;
    public int SettleMs { get; private set; } = 1000;
    public bool SelfTrim { get; private set; }
    public bool Diag { get; private set; }
    public bool PrivilegeOnly { get; private set; }
    public bool PclChild { get; private set; }
    public bool RunAsChild { get; private set; }
    public bool SkipAdjust { get; private set; }
    public string Tag { get; private set; } = "run";
    public string? OutDir { get; private set; }
    public bool ShowHelp { get; private set; }

    public const string HelpText = """
        BaselineProbe - PCL 2.13.1.1 内存优化原理验证探针 (C#/.NET 8)

        用法:
          BaselineProbe [选项]

        选项:
          --commands 2,3,4     以逗号分隔的 MEMORY_LIST_COMMAND 值（默认 2,3,4，即 PCL 的顺序）
          --repeat N           整个命令序列重复 N 次（默认 1）
          --settle-ms N        每轮命令后等待 N 毫秒再记录 after/settled（默认 1000，0=只记立即值）
          --self-trim          额外测试非管理员可行的本进程工作集裁剪（SetProcessWorkingSetSize/EmptyWorkingSet）
          --diag               额外跑参数诊断矩阵（info class/长度/命令值边界），用于确认错误码来源
          --privilege-only     只做 token/权限探测，不执行任何 NtSetSystemInformation
          --pcl-child          模拟 PCL 的 `--memory` 子进程协议：成功时退出码 = 释放的 KB
          --runas-child        以 Verb=runas 启动自身（--pcl-child）并等待退出码；本次会弹出 1 次 UAC
          --skip-adjust        不调用 AdjustTokenPrivileges（用于观察「无权限直接调用」的原始 NTSTATUS）
          --tag NAME           结果文件名标签（默认 run）
          --out-dir PATH       结果输出目录（默认 tools/baseline-results/）
          --help               显示本帮助

        推荐用法:
          1) 普通（非管理员）权限下:
               BaselineProbe --diag --self-trim --tag nonadmin
             预期: AdjustTokenPrivileges 返回 TRUE + 错误码 1300 (ERROR_NOT_ALL_ASSIGNED),
                   PCL 会抛出「获取内存优化权限失败（错误代码：1300）」;
                   每个 NtSetSystemInformation(80, 命令, 4) 返回 0xC0000061 (STATUS_PRIVILEGE_NOT_HELD)。
          2) 管理员权限（由用户点 UAC 后打开的提权终端里执行）:
               BaselineProbe --diag --self-trim --tag elevated
               BaselineProbe --pcl-child --tag pcl-child
             预期: SeProfileSingleProcessPrivilege 由 Disabled 变 Enabled,
                   命令 2/3/4 返回 0 (STATUS_SUCCESS), 可用物理内存增加,
                   --pcl-child 的退出码 = round(Δ可用物理内存/1024) KB。

        退出码:
          普通模式: 0 = 探测完成；1 = 参数/内部错误；--pcl-child 模式 = PCL 协议退出码（失败为 -1/0xFFFFFFFF）。
        """;

    public static Options Parse(string[] args)
    {
        var o = new Options();
        for (int i = 0; i < args.Length; i++)
        {
            string a = args[i];
            string Next() => i + 1 < args.Length ? args[++i] : throw new ArgumentException($"{a} 需要一个值");
            switch (a)
            {
                case "--commands":
                    o.Commands = Next().Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                        .Select(s => int.Parse(s, CultureInfo.InvariantCulture)).ToList();
                    break;
                case "--repeat":
                    o.Repeat = Math.Max(1, int.Parse(Next(), CultureInfo.InvariantCulture));
                    break;
                case "--settle-ms":
                    o.SettleMs = Math.Max(0, int.Parse(Next(), CultureInfo.InvariantCulture));
                    break;
                case "--self-trim": o.SelfTrim = true; break;
                case "--diag": o.Diag = true; break;
                case "--privilege-only": o.PrivilegeOnly = true; break;
                case "--pcl-child": o.PclChild = true; break;
                case "--runas-child": o.RunAsChild = true; break;
                case "--skip-adjust": o.SkipAdjust = true; break;
                case "--tag": o.Tag = Next(); break;
                case "--out-dir": o.OutDir = Next(); break;
                case "--help": case "-h": case "/?": o.ShowHelp = true; break;
                default: throw new ArgumentException($"未知参数: {a}（--help 查看帮助）");
            }
        }
        return o;
    }
}

internal static class Program
{
    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        WriteIndented = true,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never
    };

    private static int Main(string[] args)
    {
        Options opt;
        try
        {
            opt = Options.Parse(args);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("参数错误: " + ex.Message);
            return 2;
        }
        if (opt.ShowHelp)
        {
            Console.WriteLine(Options.HelpText);
            return 0;
        }

        var result = new ProbeResult
        {
            Tag = opt.Tag,
            TimestampUtc = DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture),
            TimestampLocal = DateTime.Now.ToString("o", CultureInfo.InvariantCulture)
        };

        try
        {
            result.Host = ProbeLogic.CollectHost();
            result.ProcessInfo = ProbeLogic.CollectProcess();
            result.Token = ProbeLogic.CollectToken(opt.SkipAdjust || opt.PclChild);
            PrintHeader(result, opt);

            if (opt.PrivilegeOnly)
            {
                string path = WriteResult(opt, result);
                Console.WriteLine($"JSON: {path}");
                return 0;
            }

            if (opt.PclChild)
                return RunPclChild(opt, result);

            if (opt.RunAsChild)
                return RunRunAsChild(opt, result);

            result.MemoryBefore = MemoryJson.From(Native.MemorySnapshot.Take());
            result.SystemMemoryListBefore = ProbeLogic.QueryMemoryList();

            for (int i = 0; i < opt.Repeat; i++)
            {
                var iter = ProbeLogic.RunIteration(i, opt.Commands, opt.SettleMs);
                result.Iterations.Add(iter);
                PrintIteration(iter);
            }

            result.MemoryAfter = MemoryJson.From(Native.MemorySnapshot.Take());
            result.SystemMemoryListAfter = ProbeLogic.QueryMemoryList();

            if (opt.Diag)
            {
                result.Diagnostics = ProbeLogic.RunDiagnostics(opt.Commands);
                PrintDiagnostics(result.Diagnostics);
            }

            if (opt.SelfTrim)
            {
                result.SelfTrim = ProbeLogic.RunSelfTrim();
                PrintSelfTrim(result.SelfTrim);
            }

            PrintFooter(result);
            string path2 = WriteResult(opt, result);
            Console.WriteLine();
            Console.WriteLine($"JSON: {path2}");
            return 0;
        }
        catch (Exception ex)
        {
            result.Error = ex.ToString();
            string path3 = WriteResult(opt, result);
            Console.Error.WriteLine("FATAL: " + ex.Message);
            Console.Error.WriteLine($"JSON: {path3}");
            return 1;
        }
    }

    private static int RunPclChild(Options opt, ProbeResult result)
    {
        var child = new PclChildResult();
        result.PclChild = child;
        child.HasAdminRole = result.ProcessInfo.PclHasAdminRoleEquivalent;
        if (!child.HasAdminRole)
        {
            child.Failed = true;
            child.FailReason = "HasAdminRole() == false: 内存优化功能需要管理员权限！";
            child.ExitCode = -1;
            child.ExitCodeMeaningKb = -1;
            WriteResult(opt, result);
            Console.Error.WriteLine(child.FailReason);
            Console.Error.WriteLine("PCL-child exit code = -1 (0xFFFFFFFF)");
            return -1;
        }

        IntPtr token = IntPtr.Zero;
        if (!Native.OpenProcessToken(Native.GetCurrentProcess(),
                Native.TOKEN_QUERY | Native.TOKEN_ADJUST_PRIVILEGES, out token))
        {
            child.Failed = true;
            child.FailReason = "OpenProcessToken 失败: " + Marshal.GetLastWin32Error();
            child.ExitCode = -1;
            WriteResult(opt, result);
            return -1;
        }

        try
        {
            if (!Native.LookupPrivilegeValueA(IntPtr.Zero, ProbeLogic.SeProfileSingleProcessPrivilege, out long luid))
            {
                child.Failed = true;
                child.FailReason = "LookupPrivilegeValueA 失败: " + Marshal.GetLastWin32Error();
                child.ExitCode = -1;
                WriteResult(opt, result);
                return -1;
            }

            int tpSize = Marshal.SizeOf<Native.PclPrivilegeToken>();
            var tp = new Native.PclPrivilegeToken { PrivilegeCount = 1, Luid = luid, Attributes = Native.SE_PRIVILEGE_ENABLED };
            IntPtr[] prev = new IntPtr[2];
            int[] retLen = new int[2];
            Native.SetLastError(0);
            bool ok = Native.AdjustTokenPrivileges(token, false, ref tp, tpSize, ref prev[0], ref retLen[0]);
            int err = Marshal.GetLastWin32Error();
            child.Adjust = new AdjustPrivilegesResult
            {
                Variant = "PCL-exact: PreviousState=&zero(非NULL), ReturnLength=&0(非NULL)",
                Attempted = true,
                Ok = ok,
                LastWin32Error = err,
                LastWin32ErrorName = StatusNames.Win32Error(err),
                PclWouldThrow = !ok || err != 0,
                PclMessage = (!ok || err != 0) ? $"获取内存优化权限失败（错误代码：{err}）" : ""
            };
            child.PrivilegeOk = !child.Adjust.PclWouldThrow;
        }
        finally
        {
            Native.CloseHandle(token);
        }

        if (!child.PrivilegeOk)
        {
            child.Failed = true;
            child.FailReason = child.Adjust.PclMessage;
            child.ExitCode = -1;
            child.ExitCodeMeaningKb = -1;
            WriteResult(opt, result);
            Console.Error.WriteLine(child.FailReason);
            Console.Error.WriteLine("PCL-child exit code = -1 (0xFFFFFFFF)");
            return -1;
        }

        var childSw = Stopwatch.StartNew();
        child.MemoryListBefore = ProbeLogic.QueryMemoryList();
        child.BeforeAvailBytes = Native.MemorySnapshot.Take().AvailPhys;
        foreach (int cmd in opt.Commands)
        {
            ulong before = Native.MemorySnapshot.Take().AvailPhys;
            var stepSw = Stopwatch.StartNew();
            uint status = Native.SetMemoryListCommand(cmd);
            stepSw.Stop();
            ulong after = Native.MemorySnapshot.Take().AvailPhys;
            child.Steps.Add(new StepResult
            {
                Command = cmd,
                CommandName = StatusNames.MemoryListCommandName(cmd),
                Status = status,
                StatusSigned = unchecked((int)status),
                StatusHex = $"0x{status:X8}",
                StatusName = StatusNames.Nt(status),
                AvailBeforeBytes = before,
                AvailAfterImmediateBytes = after,
                DeltaBytesImmediate = (long)after - (long)before,
                ElapsedMicroseconds = stepSw.Elapsed.TotalMicroseconds
            });
            Console.WriteLine($"  cmd {cmd} ({StatusNames.MemoryListCommandName(cmd)}): {StatusNames.Nt(status)} 0x{status:X8} ({stepSw.Elapsed.TotalMilliseconds:F1} ms)");
            if (status != 0)
            {
                child.Failed = true;
                child.FailReason = $"内存优化操作 {cmd} 失败（错误代码：{status}）";
                child.ExitCode = -1;
                child.ExitCodeMeaningKb = -1;
                WriteResult(opt, result);
                Console.Error.WriteLine(child.FailReason);
                Console.Error.WriteLine("PCL-child exit code = -1 (0xFFFFFFFF)");
                return -1;
            }
        }
        child.AfterAvailBytes = Native.MemorySnapshot.Take().AvailPhys;
        child.DeltaBytes = (long)child.AfterAvailBytes - (long)child.BeforeAvailBytes;
        child.SettleMs = opt.SettleMs;
        if (opt.SettleMs > 0) Thread.Sleep(opt.SettleMs);
        child.AfterSettledAvailBytes = Native.MemorySnapshot.Take().AvailPhys;
        child.DeltaSettledBytes = (long)child.AfterSettledAvailBytes - (long)child.BeforeAvailBytes;
        child.MemoryListAfter = ProbeLogic.QueryMemoryList();

        // PCL 协议: after < before -> 0；否则 round(delta/1024) KB
        int code = child.AfterAvailBytes < child.BeforeAvailBytes
            ? 0
            : (int)Math.Round(child.DeltaBytes / 1024.0);
        child.ExitCode = code;
        child.ExitCodeMeaningKb = code;
        child.TotalElapsedMs = childSw.Elapsed.TotalMilliseconds;

        // 协议测量完成后再跑可选诊断，避免污染 before/after
        if (opt.Diag)
        {
            result.Diagnostics = ProbeLogic.RunDiagnostics(opt.Commands);
            PrintDiagnostics(result.Diagnostics);
        }
        if (opt.SelfTrim)
        {
            result.SelfTrim = ProbeLogic.RunSelfTrim();
            PrintSelfTrim(result.SelfTrim);
        }

        string path = WriteResult(opt, result);
        Console.WriteLine($"  avail before    = {child.BeforeAvailBytes:N0} bytes ({Fmt(child.BeforeAvailBytes)})");
        Console.WriteLine($"  avail after     = {child.AfterAvailBytes:N0} bytes ({Fmt(child.AfterAvailBytes)})  delta={FmtDelta(child.DeltaBytes)}");
        Console.WriteLine($"  avail after+{opt.SettleMs}ms = {child.AfterSettledAvailBytes:N0} bytes  delta={FmtDelta(child.DeltaSettledBytes)}");
        if (child.MemoryListBefore is { Ok: true } beforeList && child.MemoryListAfter is { Ok: true } afterList)
            Console.WriteLine($"  standby {Fmt(beforeList.StandbyTotalBytes)} -> {Fmt(afterList.StandbyTotalBytes)} | modified {Fmt(beforeList.ModifiedPageBytes)} -> {Fmt(afterList.ModifiedPageBytes)}");
        Console.WriteLine($"PCL-child exit code = {code} (KB released)  elapsed={child.TotalElapsedMs:F0} ms");
        Console.WriteLine($"JSON: {path}");
        return code;
    }

    /// <summary>
    /// 复刻 PCL 的 ModBase.RunAsAdmin：ProcessStartInfo{Verb="runas"} 启动自身并等待退出码。
    /// 从标准（非提权）终端运行本模式会触发且仅触发 1 次 UAC。
    /// </summary>
    private static int RunRunAsChild(Options opt, ProbeResult result)
    {
        var r = new RunAsChildResult { Attempted = true };
        result.RunAsChild = r;
        string exe = Environment.ProcessPath ?? "";
        if (string.IsNullOrEmpty(exe))
        {
            r.Error = "Environment.ProcessPath is empty";
            string p0 = WriteResult(opt, result);
            Console.Error.WriteLine(r.Error);
            Console.WriteLine($"JSON: {p0}");
            return 1;
        }

        string innerTag = SanitizeTag(opt.Tag + "-inner");
        var argList = new List<string>
        {
            "--pcl-child", "--tag", innerTag,
            "--settle-ms", opt.SettleMs.ToString(CultureInfo.InvariantCulture)
        };
        if (opt.Diag) argList.Add("--diag");
        if (opt.SelfTrim) argList.Add("--self-trim");
        if (!string.IsNullOrEmpty(opt.OutDir)) { argList.Add("--out-dir"); argList.Add(opt.OutDir); }
        if (!opt.Commands.SequenceEqual(new[] { 2, 3, 4 })) { argList.Add("--commands"); argList.Add(string.Join(",", opt.Commands)); }
        r.ChildArguments = string.Join(" ", argList.Select(QuoteArg));

        var psi = new ProcessStartInfo
        {
            FileName = exe,
            Arguments = r.ChildArguments,
            UseShellExecute = true,
            Verb = "runas"
        };

        Console.WriteLine($"RunAs: {exe} {r.ChildArguments}");
        var sw = Stopwatch.StartNew();
        try
        {
            using var process = Process.Start(psi);
            if (process == null)
            {
                r.Error = "Process.Start returned null";
            }
            else
            {
                r.Started = true;
                process.WaitForExit();
                r.ChildExitCode = process.ExitCode;
                r.ReleasedKbFromExitCode = process.ExitCode;
                r.ReleasedBytesFromExitCode = (long)process.ExitCode * 1024L;
            }
        }
        catch (System.ComponentModel.Win32Exception ex)
        {
            r.Win32Error = ex.NativeErrorCode;
            r.Win32ErrorName = StatusNames.Win32Error(ex.NativeErrorCode);
            r.CancelledByUser = ex.NativeErrorCode == 1223;
            r.Error = ex.Message;
        }
        catch (Exception ex)
        {
            r.Error = ex.GetType().Name + ": " + ex.Message;
        }
        sw.Stop();
        r.ElapsedMs = sw.Elapsed.TotalMilliseconds;

        Console.WriteLine($"  started={r.Started} childExitCode={r.ChildExitCode} releasedKb={r.ReleasedKbFromExitCode} cancelled={r.CancelledByUser}");
        if (r.Win32Error != 0)
            Console.WriteLine($"  win32Error={r.Win32Error}({r.Win32ErrorName}) {r.Error}");
        Console.WriteLine($"  elapsed={r.ElapsedMs:F0} ms");
        string path = WriteResult(opt, result);
        Console.WriteLine($"JSON: {path}");
        return 0;
    }

    private static string QuoteArg(string s) => s.Contains(' ') ? $"\"{s}\"" : s;

    // ---------------- output ----------------

    private static string WriteResult(Options opt, ProbeResult result)
    {
        string dir = opt.OutDir ?? DefaultResultsDir();
        Directory.CreateDirectory(dir);
        string stamp = DateTime.Now.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture);
        string tag = SanitizeTag(opt.Tag);
        string json = JsonSerializer.Serialize(result, JsonOpts);
        string path = Path.Combine(dir, $"{stamp}-{tag}.json");
        File.WriteAllText(path, json, new UTF8Encoding(false));
        File.WriteAllText(Path.Combine(dir, "latest.json"), json, new UTF8Encoding(false));
        return path;
    }

    private static string SanitizeTag(string tag)
    {
        var invalid = Path.GetInvalidFileNameChars();
        var sb = new StringBuilder();
        foreach (char c in tag) sb.Append(invalid.Contains(c) ? '_' : c);
        return sb.Length == 0 ? "run" : sb.ToString();
    }

    private static string DefaultResultsDir()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir != null)
        {
            if (File.Exists(Path.Combine(dir.FullName, "BaselineProbe.csproj")))
                return Path.GetFullPath(Path.Combine(dir.FullName, "..", "baseline-results"));
            dir = dir.Parent;
        }
        return Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "tools", "baseline-results"));
    }

    private static void PrintHeader(ProbeResult r, Options opt)
    {
        Console.WriteLine("=== BaselineProbe: PCL 2.13.1.1 内存优化探针 ===");
        Console.WriteLine($"Host : {r.Host.MachineName} | Windows {r.Host.WindowsVersion} | {r.Host.ProcessorCount} CPU | 64bitOS={r.Host.Is64BitOs} 64bitProc={r.Host.Is64BitProcess}");
        Console.WriteLine($"Memory: total={Fmt(r.Host.TotalPhysicalBytes)} avail={Fmt(r.Host.AvailPhysicalBytesAtStart)} load={r.Host.MemoryLoadPercentAtStart}%");
        Console.WriteLine($"Token: elevated={r.ProcessInfo.IsElevated} elevationType={r.ProcessInfo.TokenElevationType}({r.ProcessInfo.TokenElevationTypeName}) PCL-HasAdminRole={r.ProcessInfo.PclHasAdminRoleEquivalent}");
        var t = r.Token;
        Console.WriteLine($"Priv  : SeProfileSingleProcess present={t.SeProfileSingleProcessPresent} enabledBefore={t.SeProfileSingleProcessWasEnabledBefore} enabledAfterAdjust={t.SeProfileSingleProcessEnabledAfterAdjust}");
        Console.WriteLine($"Lookup: ok={t.LookupPrivilege.Ok} luid={t.LookupPrivilege.LuidHex} err={t.LookupPrivilege.Win32Error}({t.LookupPrivilege.Win32ErrorName})");
        PrintAdjust(t.AdjustPclExact);
        PrintAdjust(t.AdjustNullSafe);
        Console.WriteLine($"Options: commands=[{string.Join(",", opt.Commands)}] repeat={opt.Repeat} settle={opt.SettleMs}ms pclChild={opt.PclChild} diag={opt.Diag} selfTrim={opt.SelfTrim}");
        Console.WriteLine();
    }

    private static void PrintAdjust(AdjustPrivilegesResult a)
    {
        if (!a.Attempted)
        {
            Console.WriteLine($"Adjust: [{a.Variant}] SKIPPED {(string.IsNullOrEmpty(a.PclMessage) ? "" : "(" + a.PclMessage + ")")}");
            return;
        }
        string verdict = a.PclWouldThrow ? "PCL 会抛异常" : "PCL 会继续执行";
        Console.WriteLine($"Adjust: [{a.Variant}] ok={a.Ok} lastError={a.LastWin32Error}({a.LastWin32ErrorName}) => {verdict}" +
                          (a.PclWouldThrow ? $" msg=\"{a.PclMessage}\"" : ""));
    }

    private static void PrintIteration(IterationResult it)
    {
        Console.WriteLine($"Iteration {it.Index}: before={Fmt(it.AvailBeforeBytes)} afterImmediate={Fmt(it.AvailAfterImmediateBytes)} deltaImmediate={FmtDelta(it.DeltaBytesImmediate)} afterSettled({it.SettleMs}ms)={Fmt(it.AvailAfterSettledBytes)} deltaSettled={FmtDelta(it.DeltaBytesSettled)}");
        foreach (var s in it.Steps)
        {
            Console.WriteLine($"  cmd {s.Command} ({s.CommandName,-32}) status={s.StatusHex} {s.StatusName,-28} delta={FmtDelta(s.DeltaBytesImmediate)} elapsed={s.ElapsedMicroseconds:F1}us");
        }
    }

    private static void PrintDiagnostics(List<DiagCaseResult> cases)
    {
        Console.WriteLine("Diagnostics (info class / length / command matrix):");
        Console.WriteLine($"  {"case",-36} {"status",-12} {"name",-30} note");
        foreach (var c in cases)
            Console.WriteLine($"  {c.Case,-36} {c.StatusHex,-12} {c.StatusName,-30} {c.Note}");
    }

    private static void PrintSelfTrim(SelfTrimResult s)
    {
        Console.WriteLine("Self working-set trim (user-level, non-admin):");
        Console.WriteLine($"  SetProcessWorkingSetSize(-1,-1): ok={s.SetProcessWorkingSetSizeOk} err={s.SetProcessWorkingSetSizeError}({s.SetProcessWorkingSetSizeErrorName})");
        Console.WriteLine($"  EmptyWorkingSet:                 ok={s.EmptyWorkingSetOk} err={s.EmptyWorkingSetError}({s.EmptyWorkingSetErrorName})");
        Console.WriteLine($"  process working set: {Fmt((ulong)Math.Max(0, s.ProcessWorkingSetBeforeBytes))} -> {Fmt((ulong)Math.Max(0, s.ProcessWorkingSetAfterBytes))}");
        Console.WriteLine($"  system avail delta: {FmtDelta(s.SystemAvailDeltaBytes)}");
    }

    private static void PrintFooter(ProbeResult r)
    {
        Console.WriteLine();
        Console.WriteLine("Memory list (NtQuerySystemInformation class 80):");
        PrintMemoryList("before", r.SystemMemoryListBefore);
        PrintMemoryList("after ", r.SystemMemoryListAfter);
        Console.WriteLine();
        Console.WriteLine("错误码含义: STATUS_PRIVILEGE_NOT_HELD=0xC0000061（令牌中没有已启用的 SeProfileSingleProcessPrivilege，");
        Console.WriteLine("           标准用户令牌根本不含该特权）; STATUS_INFO_LENGTH_MISMATCH=0xC0000004（payload 长度<4）;");
        Console.WriteLine("           STATUS_INVALID_PARAMETER=0xC000000D（命令值 <0 或 >=6）; STATUS_INVALID_INFO_CLASS=0xC0000003（信息类非法）。");
        Console.WriteLine("           ERROR_NOT_ALL_ASSIGNED=1300 是 AdjustTokenPrivileges 在「特权不在令牌中」时返回的 last error（函数本身返回 TRUE）。");
        bool admin = r.ProcessInfo.PclHasAdminRoleEquivalent;
        var successCommands = r.Iterations.SelectMany(i => i.Steps)
            .Where(s => s.Status == 0 && !s.CommandName.StartsWith("UNKNOWN"))
            .Select(s => s.Command).Distinct().OrderBy(c => c).ToList();
        if (!admin && successCommands.Count > 0)
        {
            Console.WriteLine($"[注意] 当前是标准用户令牌，但命令 {string.Join(",", successCommands)} 返回 STATUS_SUCCESS：该 Windows build 上此命令不需要 SeProfileSingleProcessPrivilege。");
        }
    }

    private static void PrintMemoryList(string label, MemoryListInfo? m)
    {
        if (m == null) return;
        if (!m.Ok)
        {
            Console.WriteLine($"  {label}: query failed status={m.StatusHex} {m.StatusName}");
            return;
        }
        Console.WriteLine($"  {label}: standby={Fmt(m.StandbyTotalBytes)} modified={Fmt(m.ModifiedPageBytes)} free={Fmt(m.FreePageBytes)} zero={Fmt(m.ZeroPageBytes)}");
        Console.WriteLine($"          standby by priority: [{string.Join(", ", m.StandbyPageCountByPriority.Select(Fmt))}]");
    }

    private static string Fmt(ulong bytes) => Fmt((double)bytes);
    private static string Fmt(long bytes) => Fmt((double)bytes);

    private static string Fmt(double bytes)
    {
        string[] units = { "B", "KB", "MB", "GB", "TB" };
        double v = Math.Abs(bytes);
        int u = 0;
        while (v >= 1024 && u < units.Length - 1) { v /= 1024; u++; }
        return (bytes < 0 ? "-" : "") + v.ToString("F2", CultureInfo.InvariantCulture) + units[u];
    }

    private static string FmtDelta(long bytes) => bytes >= 0 ? "+" + Fmt(bytes) : Fmt(bytes);
}

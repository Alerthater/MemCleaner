using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

namespace BaselineProbe;

internal static class ProbeLogic
{
    internal const string SeProfileSingleProcessPrivilege = "SeProfileSingleProcessPrivilege";

    // ---------------- host / process ----------------

    internal static HostInfo CollectHost()
    {
        var vi = Native.RTL_OSVERSIONINFOW.Create();
        int rc = Native.RtlGetVersion(ref vi);
        var mem = Native.MemorySnapshot.Take();
        var host = new HostInfo
        {
            MachineName = Environment.MachineName,
            UserName = $"{Environment.UserDomainName}\\{Environment.UserName}",
            WindowsVersion = rc == 0 ? $"{vi.dwMajorVersion}.{vi.dwMinorVersion}.{vi.dwBuildNumber}" : Environment.OSVersion.Version.ToString(),
            OsDescription = RuntimeInformation.OSDescription,
            Major = rc == 0 ? vi.dwMajorVersion : (uint)Environment.OSVersion.Version.Major,
            Minor = rc == 0 ? vi.dwMinorVersion : (uint)Environment.OSVersion.Version.Minor,
            Build = rc == 0 ? vi.dwBuildNumber : (uint)Environment.OSVersion.Version.Build,
            Is64BitOs = Environment.Is64BitOperatingSystem,
            Is64BitProcess = Environment.Is64BitProcess,
            ProcessorCount = Environment.ProcessorCount,
            TotalPhysicalBytes = mem.TotalPhys,
            AvailPhysicalBytesAtStart = mem.AvailPhys,
            MemoryLoadPercentAtStart = mem.MemoryLoadPercent,
            ProbePath = Environment.ProcessPath ?? ""
        };
        return host;
    }

    internal static ProcessInfo CollectProcess()
    {
        var info = new ProcessInfo
        {
            Pid = Environment.ProcessId,
            Path = Environment.ProcessPath ?? "",
            PclHasAdminRoleEquivalent = IsAdminRoleEquivalent()
        };

        IntPtr token = IntPtr.Zero;
        if (!Native.OpenProcessToken(Native.GetCurrentProcess(), Native.TOKEN_QUERY, out token))
            return info;

        try
        {
            info.TokenElevationType = ReadUIntTokenInfo(token, Native.TokenElevationType);
            info.IsElevated = ReadUIntTokenInfo(token, Native.TokenElevation) == 1;
            info.TokenElevationTypeName = info.TokenElevationType switch
            {
                1 => "TokenElevationTypeDefault",
                2 => "TokenElevationTypeFull",
                3 => "TokenElevationTypeLimited",
                _ => "Unknown"
            };
            info.UserSid = ReadUserSid(token);
            return info;
        }
        finally
        {
            Native.CloseHandle(token);
        }
    }

    /// <summary>复刻 PCL 的 WindowsUtils.HasAdminRole(): WindowsPrincipal.IsInRole(Administrator)。</summary>
    internal static bool IsAdminRoleEquivalent()
    {
        // WindowsPrincipal.IsInRole 内部就是 CheckTokenMembership(NULL, BUILTIN\Administrators SID)，
        // UAC 过滤令牌里 Administrators 组是 deny-only，因此返回 false。
        if (!Native.ConvertStringSidToSid("S-1-5-32-544", out IntPtr sid))
            return false;
        try
        {
            return Native.CheckTokenMembership(IntPtr.Zero, sid, out bool member) && member;
        }
        finally
        {
            Native.LocalFree(sid);
        }
    }

    private static uint ReadUIntTokenInfo(IntPtr token, int infoClass)
    {
        IntPtr buf = Marshal.AllocHGlobal(4);
        try
        {
            if (Native.GetTokenInformation(token, infoClass, buf, 4, out _))
                return (uint)Marshal.ReadInt32(buf);
            return uint.MaxValue;
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
    }

    private static string ReadUserSid(IntPtr token)
    {
        if (!Native.GetTokenInformation(token, 1 /*TokenUser*/, IntPtr.Zero, 0, out int needed) && needed <= 0)
            return "";
        IntPtr buf = Marshal.AllocHGlobal(needed);
        try
        {
            if (!Native.GetTokenInformation(token, 1, buf, needed, out _))
                return "";
            IntPtr sid = Marshal.ReadIntPtr(buf); // TOKEN_USER.Sid 是第一个字段
            if (!Native.ConvertSidToStringSidW(sid, out IntPtr sidString) || sidString == IntPtr.Zero)
                return "";
            try
            {
                return Marshal.PtrToStringUni(sidString) ?? "";
            }
            finally
            {
                Native.LocalFree(sidString);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
    }

    // ---------------- token / privilege ----------------

    internal static TokenProbeResult CollectToken(bool skipAdjust)
    {
        var result = new TokenProbeResult();
        IntPtr token = IntPtr.Zero;
        bool opened = Native.OpenProcessToken(Native.GetCurrentProcess(),
            Native.TOKEN_QUERY | Native.TOKEN_ADJUST_PRIVILEGES, out token);
        result.OpenProcessTokenOk = opened;
        result.OpenProcessTokenError = opened ? 0 : Marshal.GetLastWin32Error();
        result.OpenProcessTokenErrorName = StatusNames.Win32Error(result.OpenProcessTokenError);
        if (!opened)
        {
            if (!Native.OpenProcessToken(Native.GetCurrentProcess(), Native.TOKEN_QUERY, out token))
                return result; // 连查询都打不开，只能返回空结果
        }

        try
        {
            result.PclHasAdminRoleEquivalent = IsAdminRoleEquivalent();
            result.Privileges = EnumeratePrivileges(token);
            var seProfile = result.Privileges.FirstOrDefault(p => p.Name.Equals(SeProfileSingleProcessPrivilege, StringComparison.OrdinalIgnoreCase));
            result.SeProfileSingleProcessPresent = seProfile != null;
            result.SeProfileSingleProcessWasEnabledBefore = seProfile?.Enabled ?? false;

            var lookup = new LookupPrivilegeResult();
            Native.SetLastError(0);
            bool lookupOk = Native.LookupPrivilegeValueA(IntPtr.Zero, SeProfileSingleProcessPrivilege, out long luid);
            lookup.Ok = lookupOk;
            lookup.Win32Error = lookupOk ? 0 : Marshal.GetLastWin32Error();
            lookup.Win32ErrorName = StatusNames.Win32Error(lookup.Win32Error);
            lookup.Luid = luid;
            lookup.LuidHex = $"0x{luid:X16}";
            result.LookupPrivilege = lookup;

            var exact = new AdjustPrivilegesResult
            {
                Variant = "PCL-exact: PreviousState=&zero(非NULL), ReturnLength=&0(非NULL)",
            };
            var nullSafe = new AdjustPrivilegesResult
            {
                Variant = "documented-safe: PreviousState=NULL, ReturnLength=NULL",
            };

            if (skipAdjust)
            {
                exact.Attempted = false;
                exact.PclMessage = "skipped (--skip-adjust)";
                nullSafe.Attempted = false;
                result.AdjustPclExact = exact;
                result.AdjustNullSafe = nullSafe;
                return result;
            }

            if (!opened)
            {
                exact.Attempted = false;
                exact.PclMessage = "OpenProcessToken(TOKEN_ADJUST_PRIVILEGES) failed: " + result.OpenProcessTokenErrorName;
                result.AdjustPclExact = exact;
                result.AdjustNullSafe = nullSafe;
                return result;
            }

            int tpSize = Marshal.SizeOf<Native.PclPrivilegeToken>();

            // 1) PCL 的原始写法。为了不真的在 probe 进程里制造 12 字节栈越界（PCL 的
            //    PreviousState 指向 4 字节局部变量，内核写 TOKEN_PRIVILEGES 需要 16 字节），
            //    这里把 PreviousState/ReturnLength 放在数组里，保证指针非 NULL 且缓冲区足够大。
            var tp = new Native.PclPrivilegeToken { PrivilegeCount = 1, Luid = luid, Attributes = Native.SE_PRIVILEGE_ENABLED };
            IntPtr[] prev = new IntPtr[2]; // 16 bytes
            int[] retLen = new int[2];     // 8 bytes
            Native.SetLastError(0);
            bool ok = Native.AdjustTokenPrivileges(token, false, ref tp, tpSize, ref prev[0], ref retLen[0]);
            int err = Marshal.GetLastWin32Error();
            exact.Attempted = true;
            exact.Ok = ok;
            exact.LastWin32Error = err;
            exact.LastWin32ErrorName = StatusNames.Win32Error(err);
            exact.PclWouldThrow = !ok || err != 0;
            exact.PclMessage = exact.PclWouldThrow
                ? $"获取内存优化权限失败（错误代码：{err}）"
                : "";
            result.AdjustPclExact = exact;
            result.SeProfileSingleProcessEnabledAfterAdjust = IsPrivilegeEnabled(token, luid);

            // 2) 官方推荐写法（两个指针都传 NULL），用于对比。
            if (!result.SeProfileSingleProcessEnabledAfterAdjust)
            {
                var tp2 = new Native.PclPrivilegeToken { PrivilegeCount = 1, Luid = luid, Attributes = Native.SE_PRIVILEGE_ENABLED };
                Native.SetLastError(0);
                bool ok2 = Native.AdjustTokenPrivilegesNull(token, false, ref tp2, tpSize, IntPtr.Zero, IntPtr.Zero);
                int err2 = Marshal.GetLastWin32Error();
                nullSafe.Attempted = true;
                nullSafe.Ok = ok2;
                nullSafe.LastWin32Error = err2;
                nullSafe.LastWin32ErrorName = StatusNames.Win32Error(err2);
                nullSafe.PclWouldThrow = !ok2 || err2 != 0;
                nullSafe.PclMessage = nullSafe.PclWouldThrow
                    ? $"获取内存优化权限失败（错误代码：{err2}）"
                    : "";
                result.SeProfileSingleProcessEnabledAfterAdjust = IsPrivilegeEnabled(token, luid);
            }
            else
            {
                nullSafe.Attempted = false;
                nullSafe.PclMessage = "not needed: privilege already enabled by the PCL-exact call";
            }
            result.AdjustNullSafe = nullSafe;
            return result;
        }
        finally
        {
            if (token != IntPtr.Zero) Native.CloseHandle(token);
        }
    }

    private static List<TokenPrivilegeInfo> EnumeratePrivileges(IntPtr token)
    {
        var list = new List<TokenPrivilegeInfo>();
        if (!Native.GetTokenInformation(token, Native.TokenPrivileges, IntPtr.Zero, 0, out int needed) && needed <= 0)
            return list;
        IntPtr buf = Marshal.AllocHGlobal(needed);
        try
        {
            if (!Native.GetTokenInformation(token, Native.TokenPrivileges, buf, needed, out _))
                return list;
            int count = Marshal.ReadInt32(buf);
            for (int i = 0; i < count; i++)
            {
                int offset = 4 + i * 12; // TOKEN_PRIVILEGES: DWORD count; LUID_AND_ATTRIBUTES[]
                uint low = (uint)Marshal.ReadInt32(buf, offset);
                int high = Marshal.ReadInt32(buf, offset + 4);
                uint attributes = (uint)Marshal.ReadInt32(buf, offset + 8);
                long luid = ((long)high << 32) | low;
                list.Add(new TokenPrivilegeInfo
                {
                    Name = LookupPrivilegeName(luid),
                    Luid = luid,
                    Attributes = attributes,
                    Enabled = (attributes & Native.SE_PRIVILEGE_ENABLED) != 0,
                    EnabledByDefault = (attributes & 1) != 0
                });
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
        return list;
    }

    private static string LookupPrivilegeName(long luid)
    {
        var sb = new StringBuilder(256);
        int len = sb.Capacity;
        return Native.LookupPrivilegeName(IntPtr.Zero, ref luid, sb, ref len)
            ? sb.ToString()
            : $"Luid:0x{luid:X16}";
    }

    private static bool IsPrivilegeEnabled(IntPtr token, long luid)
    {
        var privileges = EnumeratePrivileges(token);
        var match = privileges.FirstOrDefault(p => p.Luid == luid);
        return match != null && match.Enabled;
    }

    // ---------------- SYSTEM_MEMORY_LIST_INFORMATION (class 80 query) ----------------

    internal static MemoryListInfo QueryMemoryList()
    {
        int ptrSize = IntPtr.Size;
        int count = 22; // 5 + 8 + 8 + 1 个 SIZE_T
        int length = count * ptrSize;
        IntPtr buf = Marshal.AllocHGlobal(length);
        try
        {
            for (int i = 0; i < length; i++) Marshal.WriteByte(buf, i, 0);
            uint status = Native.NtQuerySystemInformation(Native.SystemMemoryListInformation, buf, length, out _);
            var info = new MemoryListInfo
            {
                Ok = status == 0,
                Status = status,
                StatusHex = $"0x{status:X8}",
                StatusName = StatusNames.Nt(status),
                BufferLength = length
            };
            if (status != 0)
                return info;

            ulong Read(int index) => ptrSize == 8
                ? (ulong)Marshal.ReadInt64(buf, index * 8)
                : (uint)Marshal.ReadInt32(buf, index * 4);

            ulong page = (ulong)Environment.SystemPageSize;
            info.ZeroPageBytes = Read(0) * page;
            info.FreePageBytes = Read(1) * page;
            info.ModifiedPageBytes = Read(2) * page;
            info.ModifiedNoWritePageBytes = Read(3) * page;
            info.BadPageBytes = Read(4) * page;
            var standby = new ulong[8];
            ulong standbySum = 0;
            for (int i = 0; i < 8; i++)
            {
                standby[i] = Read(5 + i) * page;
                standbySum += standby[i];
            }
            info.StandbyPageCountByPriority = standby;
            info.StandbyTotalBytes = standbySum;
            var repurposed = new ulong[8];
            for (int i = 0; i < 8; i++) repurposed[i] = Read(13 + i) * page;
            info.RepurposedPagesByPriority = repurposed;
            info.ModifiedPageCountPageFile = Read(21) * page;
            return info;
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
    }

    // ---------------- sequence ----------------

    internal static IterationResult RunIteration(int index, IReadOnlyList<int> commands, int settleMs)
    {
        var iter = new IterationResult
        {
            Index = index,
            AvailBeforeBytes = Native.MemorySnapshot.Take().AvailPhys,
            SettleMs = settleMs
        };
        bool allZero = true;
        foreach (int cmd in commands)
        {
            ulong before = Native.MemorySnapshot.Take().AvailPhys;
            var sw = Stopwatch.StartNew();
            uint status = Native.SetMemoryListCommand(cmd);
            sw.Stop();
            ulong after = Native.MemorySnapshot.Take().AvailPhys;
            if (status != 0) allZero = false;
            iter.Steps.Add(new StepResult
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
                ElapsedMicroseconds = sw.Elapsed.TotalMicroseconds
            });
        }
        iter.AvailAfterImmediateBytes = Native.MemorySnapshot.Take().AvailPhys;
        iter.DeltaBytesImmediate = (long)iter.AvailAfterImmediateBytes - (long)iter.AvailBeforeBytes;
        if (settleMs > 0) Thread.Sleep(settleMs);
        iter.AvailAfterSettledBytes = Native.MemorySnapshot.Take().AvailPhys;
        iter.DeltaBytesSettled = (long)iter.AvailAfterSettledBytes - (long)iter.AvailBeforeBytes;
        iter.AllStatusesZero = allZero;
        return iter;
    }

    internal static List<DiagCaseResult> RunDiagnostics(IReadOnlyList<int> pclCommands)
    {
        var cases = new List<(string Case, int InfoClass, int Length, int CommandValue, string Note)>();
        foreach (int cmd in pclCommands)
            cases.Add(($"class=80 len=4 cmd={cmd}", 80, 4, cmd, "PCL 实际使用的组合"));
        cases.Add(("class=80 len=0 cmd=0", 80, 0, 0, "长度错误时的返回（看是否先检查长度）"));
        cases.Add(("class=80 len=8 cmd=2", 80, 8, 2, "payload 长度 8 字节"));
        cases.Add(("class=80 len=8 cmd=3", 80, 8, 3, "payload 长度 8 字节时 FlushModifiedList 是否仍执行"));
        cases.Add(("class=80 len=8 cmd=4", 80, 8, 4, "payload 长度 8 字节时 PurgeStandbyList"));
        cases.Add(("class=80 len=2 cmd=3", 80, 2, 3, "payload 长度 2 字节（不足 int）"));
        cases.Add(("class=80 len=16 cmd=3", 80, 16, 3, "payload 长度 16 字节"));
        cases.Add(("class=80 len=4 cmd=0", 80, 4, 0, "MemoryCaptureAccessedBits"));
        cases.Add(("class=80 len=4 cmd=1", 80, 4, 1, "MemoryCaptureAndResetAccessedBits"));
        cases.Add(("class=80 len=4 cmd=5", 80, 4, 5, "MemoryPurgeLowPriorityStandbyList（Mem Reduct 额外功能，PCL 未用）"));
        cases.Add(("class=80 len=4 cmd=6", 80, 4, 6, "MemoryCommandMax（越界命令）"));
        cases.Add(("class=80 len=4 cmd=-1", 80, 4, -1, "负数命令"));
        cases.Add(("class=79 len=4 cmd=2", 79, 4, 2, "相邻信息类，预期 STATS_INVALID_INFO_CLASS"));
        cases.Add(("class=81 len=4 cmd=2", 81, 4, 2, "相邻信息类"));
        cases.Add(("class=0x7FFFFFFF len=4 cmd=2", 0x7FFFFFFF, 4, 2, "非法信息类"));

        var results = new List<DiagCaseResult>();
        foreach (var c in cases)
        {
            uint status = Native.SetSystemInformationRaw(c.InfoClass, c.Length, c.CommandValue);
            results.Add(new DiagCaseResult
            {
                Case = c.Case,
                InfoClass = c.InfoClass,
                Length = c.Length,
                CommandValue = c.CommandValue,
                Status = status,
                StatusHex = $"0x{status:X8}",
                StatusName = StatusNames.Nt(status),
                Note = c.Note
            });
        }
        return results;
    }

    // ---------------- self working set (user-level fallback, non-admin OK) ----------------

    internal static SelfTrimResult RunSelfTrim()
    {
        var result = new SelfTrimResult();
        var process = Process.GetCurrentProcess();
        process.Refresh();
        result.ProcessWorkingSetBeforeBytes = process.WorkingSet64;
        result.SystemAvailBeforeBytes = Native.MemorySnapshot.Take().AvailPhys;

        Native.SetLastError(0);
        result.SetProcessWorkingSetSizeOk = Native.SetProcessWorkingSetSize(
            Native.GetCurrentProcess(), new IntPtr(-1), new IntPtr(-1));
        result.SetProcessWorkingSetSizeError = result.SetProcessWorkingSetSizeOk ? 0 : Marshal.GetLastWin32Error();
        result.SetProcessWorkingSetSizeErrorName = StatusNames.Win32Error(result.SetProcessWorkingSetSizeError);

        Native.SetLastError(0);
        result.EmptyWorkingSetOk = Native.EmptyWorkingSet(Native.GetCurrentProcess());
        result.EmptyWorkingSetError = result.EmptyWorkingSetOk ? 0 : Marshal.GetLastWin32Error();
        result.EmptyWorkingSetErrorName = StatusNames.Win32Error(result.EmptyWorkingSetError);

        process.Refresh();
        result.ProcessWorkingSetAfterBytes = process.WorkingSet64;
        result.SystemAvailAfterBytes = Native.MemorySnapshot.Take().AvailPhys;
        result.SystemAvailDeltaBytes = (long)result.SystemAvailAfterBytes - (long)result.SystemAvailBeforeBytes;
        return result;
    }
}

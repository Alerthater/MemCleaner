namespace BaselineProbe;

// JSON 结果模型。字段名刻意保持稳定、自解释，方便主程序/文档直接引用。

internal sealed class HostInfo
{
    public string MachineName { get; set; } = "";
    public string UserName { get; set; } = "";
    public string WindowsVersion { get; set; } = "";     // RtlGetVersion: 10.0.26300.0
    public string OsDescription { get; set; } = "";
    public uint Major { get; set; }
    public uint Minor { get; set; }
    public uint Build { get; set; }
    public bool Is64BitOs { get; set; }
    public bool Is64BitProcess { get; set; }
    public int ProcessorCount { get; set; }
    public ulong TotalPhysicalBytes { get; set; }
    public ulong AvailPhysicalBytesAtStart { get; set; }
    public uint MemoryLoadPercentAtStart { get; set; }
    public string ProbePath { get; set; } = "";
    public string ProbeVersion { get; set; } = "1.0.0";
}

internal sealed class ProcessInfo
{
    public int Pid { get; set; }
    public string Path { get; set; } = "";
    public bool IsElevated { get; set; }
    public uint TokenElevationType { get; set; }          // 1=Default 2=Full 3=Limited
    public string TokenElevationTypeName { get; set; } = "";
    public bool PclHasAdminRoleEquivalent { get; set; }   // WindowsPrincipal.IsInRole(Administrator) 语义
    public string UserSid { get; set; } = "";
}

internal sealed class TokenPrivilegeInfo
{
    public string Name { get; set; } = "";
    public long Luid { get; set; }
    public uint Attributes { get; set; }
    public bool Enabled { get; set; }
    public bool EnabledByDefault { get; set; }
}

internal sealed class LookupPrivilegeResult
{
    public bool Ok { get; set; }
    public int Win32Error { get; set; }
    public string Win32ErrorName { get; set; } = "";
    public long Luid { get; set; }
    public string LuidHex { get; set; } = "";
}

internal sealed class AdjustPrivilegesResult
{
    public string Variant { get; set; } = "";
    public bool Attempted { get; set; }
    public bool Ok { get; set; }
    public int LastWin32Error { get; set; }
    public string LastWin32ErrorName { get; set; } = "";
    public bool PclWouldThrow { get; set; }
    public string PclMessage { get; set; } = "";
}

internal sealed class TokenProbeResult
{
    public bool OpenProcessTokenOk { get; set; }
    public int OpenProcessTokenError { get; set; }
    public string OpenProcessTokenErrorName { get; set; } = "";
    public bool SeProfileSingleProcessPresent { get; set; }
    public bool SeProfileSingleProcessEnabledAfterAdjust { get; set; }
    public bool SeProfileSingleProcessWasEnabledBefore { get; set; }
    public List<TokenPrivilegeInfo> Privileges { get; set; } = new();
    public LookupPrivilegeResult LookupPrivilege { get; set; } = new();
    public AdjustPrivilegesResult AdjustPclExact { get; set; } = new();
    public AdjustPrivilegesResult AdjustNullSafe { get; set; } = new();
    public bool PclHasAdminRoleEquivalent { get; set; }
}

internal sealed class MemoryJson
{
    public bool Ok { get; set; }
    public int Win32Error { get; set; }
    public string Win32ErrorName { get; set; } = "";
    public uint MemoryLoadPercent { get; set; }
    public ulong TotalPhysicalBytes { get; set; }
    public ulong AvailPhysicalBytes { get; set; }
    public ulong TotalPageFileBytes { get; set; }
    public ulong AvailPageFileBytes { get; set; }
    public ulong TotalVirtualBytes { get; set; }
    public ulong AvailVirtualBytes { get; set; }

    internal static MemoryJson From(Native.MemorySnapshot s) => new()
    {
        Ok = s.Ok,
        Win32Error = s.Win32Error,
        Win32ErrorName = StatusNames.Win32Error(s.Win32Error),
        MemoryLoadPercent = s.MemoryLoadPercent,
        TotalPhysicalBytes = s.TotalPhys,
        AvailPhysicalBytes = s.AvailPhys,
        TotalPageFileBytes = s.TotalPageFile,
        AvailPageFileBytes = s.AvailPageFile,
        TotalVirtualBytes = s.TotalVirtual,
        AvailVirtualBytes = s.AvailVirtual
    };
}

internal sealed class MemoryListInfo
{
    public bool Ok { get; set; }
    public uint Status { get; set; }
    public string StatusHex { get; set; } = "";
    public string StatusName { get; set; } = "";
    public int BufferLength { get; set; }
    public ulong ZeroPageBytes { get; set; }
    public ulong FreePageBytes { get; set; }
    public ulong ModifiedPageBytes { get; set; }
    public ulong ModifiedNoWritePageBytes { get; set; }
    public ulong BadPageBytes { get; set; }
    public ulong StandbyTotalBytes { get; set; }
    public ulong[] StandbyPageCountByPriority { get; set; } = Array.Empty<ulong>();
    public ulong[] RepurposedPagesByPriority { get; set; } = Array.Empty<ulong>();
    public ulong ModifiedPageCountPageFile { get; set; }
}

internal sealed class StepResult
{
    public int Command { get; set; }
    public string CommandName { get; set; } = "";
    public uint Status { get; set; }
    public int StatusSigned { get; set; }
    public string StatusHex { get; set; } = "";
    public string StatusName { get; set; } = "";
    public ulong AvailBeforeBytes { get; set; }
    public ulong AvailAfterImmediateBytes { get; set; }
    public long DeltaBytesImmediate { get; set; }
    public double ElapsedMicroseconds { get; set; }
}

internal sealed class IterationResult
{
    public int Index { get; set; }
    public ulong AvailBeforeBytes { get; set; }
    public ulong AvailAfterImmediateBytes { get; set; }
    public long DeltaBytesImmediate { get; set; }
    public ulong AvailAfterSettledBytes { get; set; }
    public long DeltaBytesSettled { get; set; }
    public int SettleMs { get; set; }
    public List<StepResult> Steps { get; set; } = new();
    public bool AllStatusesZero { get; set; }
}

internal sealed class DiagCaseResult
{
    public string Case { get; set; } = "";
    public int InfoClass { get; set; }
    public int Length { get; set; }
    public int CommandValue { get; set; }
    public uint Status { get; set; }
    public string StatusHex { get; set; } = "";
    public string StatusName { get; set; } = "";
    public string Note { get; set; } = "";
}

internal sealed class SelfTrimResult
{
    public bool SetProcessWorkingSetSizeOk { get; set; }
    public int SetProcessWorkingSetSizeError { get; set; }
    public string SetProcessWorkingSetSizeErrorName { get; set; } = "";
    public bool EmptyWorkingSetOk { get; set; }
    public int EmptyWorkingSetError { get; set; }
    public string EmptyWorkingSetErrorName { get; set; } = "";
    public long ProcessWorkingSetBeforeBytes { get; set; }
    public long ProcessWorkingSetAfterBytes { get; set; }
    public ulong SystemAvailBeforeBytes { get; set; }
    public ulong SystemAvailAfterBytes { get; set; }
    public long SystemAvailDeltaBytes { get; set; }
}

internal sealed class PclChildResult
{
    public bool HasAdminRole { get; set; }
    public bool PrivilegeOk { get; set; }
    public AdjustPrivilegesResult Adjust { get; set; } = new();
    public ulong BeforeAvailBytes { get; set; }
    public ulong AfterAvailBytes { get; set; }
    public long DeltaBytes { get; set; }
    public ulong AfterSettledAvailBytes { get; set; }
    public long DeltaSettledBytes { get; set; }
    public int SettleMs { get; set; }
    public double TotalElapsedMs { get; set; }
    public MemoryListInfo? MemoryListBefore { get; set; }
    public MemoryListInfo? MemoryListAfter { get; set; }
    public int ExitCode { get; set; }
    public int ExitCodeMeaningKb { get; set; }
    public List<StepResult> Steps { get; set; } = new();
    public bool Failed { get; set; }
    public string? FailReason { get; set; }
}

internal sealed class RunAsChildResult
{
    public bool Attempted { get; set; }
    public bool Started { get; set; }
    public int Win32Error { get; set; }
    public string Win32ErrorName { get; set; } = "";
    public bool CancelledByUser { get; set; }
    public string ChildArguments { get; set; } = "";
    public int ChildExitCode { get; set; } = -1;
    public long ReleasedKbFromExitCode { get; set; } = -1;
    public long ReleasedBytesFromExitCode { get; set; } = -1;
    public double ElapsedMs { get; set; }
    public string? Error { get; set; }
}

internal sealed class ProbeResult
{
    public string Schema { get; set; } = "baseline-probe/1";
    public string? Tag { get; set; }
    public string TimestampUtc { get; set; } = "";
    public string TimestampLocal { get; set; } = "";
    public HostInfo Host { get; set; } = new();
    public ProcessInfo ProcessInfo { get; set; } = new();
    public TokenProbeResult Token { get; set; } = new();
    public MemoryJson? MemoryBefore { get; set; }
    public MemoryJson? MemoryAfter { get; set; }
    public MemoryListInfo? SystemMemoryListBefore { get; set; }
    public MemoryListInfo? SystemMemoryListAfter { get; set; }
    public List<IterationResult> Iterations { get; set; } = new();
    public SelfTrimResult? SelfTrim { get; set; }
    public List<DiagCaseResult> Diagnostics { get; set; } = new();
    public PclChildResult? PclChild { get; set; }
    public RunAsChildResult? RunAsChild { get; set; }
    public List<string> Notes { get; set; } = new();
    public string? Error { get; set; }
}

using System.Runtime.InteropServices;
using System.Text;

namespace BaselineProbe;

/// <summary>
/// Thin P/Invoke layer. 所有签名与 PCL 2.13.1.1 反编译产物保持同构，
/// 额外的 safe 变体用于验证「PreviousState / ReturnLength 传 NULL」的官方推荐写法。
/// </summary>
internal static class Native
{
    // ---------------- memory status ----------------

    [StructLayout(LayoutKind.Sequential)]
    internal struct MEMORYSTATUSEX
    {
        public uint dwLength;
        public uint dwMemoryLoad;
        public ulong ullTotalPhys;
        public ulong ullAvailPhys;
        public ulong ullTotalPageFile;
        public ulong ullAvailPageFile;
        public ulong ullTotalVirtual;
        public ulong ullAvailVirtual;
        public ulong ullAvailExtendedVirtual;

        public static MEMORYSTATUSEX Create() => new()
        {
            dwLength = (uint)Marshal.SizeOf<MEMORYSTATUSEX>()
        };
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX lpBuffer);

    internal sealed record MemorySnapshot(
        bool Ok,
        int Win32Error,
        uint MemoryLoadPercent,
        ulong TotalPhys,
        ulong AvailPhys,
        ulong TotalPageFile,
        ulong AvailPageFile,
        ulong TotalVirtual,
        ulong AvailVirtual)
    {
        internal static MemorySnapshot Take()
        {
            var st = MEMORYSTATUSEX.Create();
            bool ok = GlobalMemoryStatusEx(ref st);
            int err = ok ? 0 : Marshal.GetLastWin32Error();
            return new MemorySnapshot(ok, err, st.dwMemoryLoad, st.ullTotalPhys, st.ullAvailPhys,
                st.ullTotalPageFile, st.ullAvailPageFile, st.ullTotalVirtual, st.ullAvailVirtual);
        }
    }

    [DllImport("kernel32.dll")]
    internal static extern IntPtr GetCurrentProcess();

    [DllImport("kernel32.dll", SetLastError = true)]
    internal static extern void SetLastError(uint dwErrCode);

    // ---------------- privilege / token ----------------

    internal const uint TOKEN_QUERY = 0x0008;
    internal const uint TOKEN_ADJUST_PRIVILEGES = 0x0020;

    internal const int TokenPrivileges = 3;
    internal const int TokenElevationType = 18;
    internal const int TokenElevation = 20;

    internal const uint SE_PRIVILEGE_ENABLED = 0x00000002;

    /// <summary>PCL 反编译产物中 PrivilegeToken 的精确布局（Pack=1, int/long/int）。</summary>
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    internal struct PclPrivilegeToken
    {
        public int PrivilegeCount;
        public long Luid;
        public uint Attributes;
    }

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool OpenProcessToken(IntPtr processHandle, uint desiredAccess, out IntPtr tokenHandle);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool CloseHandle(IntPtr handle);

    [DllImport("advapi32.dll", EntryPoint = "LookupPrivilegeValueA", CharSet = CharSet.Ansi, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool LookupPrivilegeValueA(IntPtr lpSystemName, [MarshalAs(UnmanagedType.LPStr)] string lpName, out long lpLuid);

    // PCL 的写法：ref PreviousState（指向一个值为 NULL 的局部变量）与 ref ReturnLength（指向 0），
    // 即两个指针都不是 NULL。这里保留同构签名以便验证该「非 NULL 零长度缓冲区」的行为。
    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool AdjustTokenPrivileges(
        IntPtr tokenHandle,
        [MarshalAs(UnmanagedType.Bool)] bool disableAllPrivileges,
        ref PclPrivilegeToken newState,
        int bufferLength,
        ref IntPtr previousState,
        ref int returnLength);

    // 官方文档推荐的写法：PreviousState = NULL, ReturnLength = NULL。
    [DllImport("advapi32.dll", EntryPoint = "AdjustTokenPrivileges", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool AdjustTokenPrivilegesNull(
        IntPtr tokenHandle,
        [MarshalAs(UnmanagedType.Bool)] bool disableAllPrivileges,
        ref PclPrivilegeToken newState,
        int bufferLength,
        IntPtr previousState,
        IntPtr returnLength);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool GetTokenInformation(
        IntPtr tokenHandle,
        int tokenInformationClass,
        IntPtr tokenInformation,
        int tokenInformationLength,
        out int returnLength);

    [DllImport("advapi32.dll", EntryPoint = "LookupPrivilegeNameW", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool LookupPrivilegeName(IntPtr lpSystemName, ref long lpLuid, StringBuilder lpName, ref int cchName);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool CheckTokenMembership(IntPtr tokenHandle, IntPtr sidToCheck, [MarshalAs(UnmanagedType.Bool)] out bool isMember);

    [DllImport("advapi32.dll", EntryPoint = "ConvertStringSidToSidW", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool ConvertStringSidToSid(string stringSid, out IntPtr sid);

    [DllImport("advapi32.dll", EntryPoint = "ConvertSidToStringSidW", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool ConvertSidToStringSidW(IntPtr sid, out IntPtr stringSid);

    [DllImport("kernel32.dll", SetLastError = true)]
    internal static extern IntPtr LocalFree(IntPtr hMem);

    // ---------------- NT syscalls ----------------

    public const int SystemMemoryListInformation = 80; // phnt ntexapi.h: 80, 需要 SeProfileSingleProcessPrivilege

    [DllImport("ntdll.dll")]
    internal static extern uint NtSetSystemInformation(
        int systemInformationClass,
        IntPtr systemInformation,
        int systemInformationLength);

    [DllImport("ntdll.dll")]
    internal static extern uint NtQuerySystemInformation(
        int systemInformationClass,
        IntPtr systemInformation,
        int systemInformationLength,
        out int returnLength);

    /// <summary>以 int（MEMORY_LIST_COMMAND 的大小为 4 字节）为载荷调用 class 80。</summary>
    internal static uint SetMemoryListCommand(int command)
    {
        IntPtr buf = Marshal.AllocHGlobal(4);
        try
        {
            Marshal.WriteInt32(buf, command);
            return NtSetSystemInformation(SystemMemoryListInformation, buf, 4);
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
    }

    internal static uint SetSystemInformationRaw(int infoClass, int length, int commandValue)
    {
        if (length <= 0)
            return NtSetSystemInformation(infoClass, IntPtr.Zero, 0);

        IntPtr buf = Marshal.AllocHGlobal(length);
        try
        {
            for (int i = 0; i < length; i++) Marshal.WriteByte(buf, i, 0);
            if (length >= 4) Marshal.WriteInt32(buf, 0, commandValue);
            return NtSetSystemInformation(infoClass, buf, length);
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
    }

    // ---------------- working set (self) ----------------

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool SetProcessWorkingSetSize(IntPtr process, IntPtr minimumWorkingSetSize, IntPtr maximumWorkingSetSize);

    [DllImport("psapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool EmptyWorkingSet(IntPtr process);

    // ---------------- OS version ----------------

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    internal struct RTL_OSVERSIONINFOW
    {
        public uint dwOSVersionInfoSize;
        public uint dwMajorVersion;
        public uint dwMinorVersion;
        public uint dwBuildNumber;
        public uint dwPlatformId;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string szCSDVersion;

        public static RTL_OSVERSIONINFOW Create() => new()
        {
            dwOSVersionInfoSize = (uint)Marshal.SizeOf<RTL_OSVERSIONINFOW>(),
            szCSDVersion = string.Empty
        };
    }

    [DllImport("ntdll.dll", CharSet = CharSet.Unicode)]
    internal static extern int RtlGetVersion(ref RTL_OSVERSIONINFOW versionInformation);
}

/// <summary>NTSTATUS / Win32 错误码的人类可读映射（只列出与本课题相关的值）。</summary>
internal static class StatusNames
{
    private static readonly Dictionary<uint, string> NtStatus = new()
    {
        [0x00000000] = "STATUS_SUCCESS",
        [0x80000005] = "STATUS_BUFFER_OVERFLOW",
        [0x80000006] = "STATUS_NO_MORE_ENTRIES",
        [0xC0000003] = "STATUS_INVALID_INFO_CLASS",
        [0xC0000004] = "STATUS_INFO_LENGTH_MISMATCH",
        [0xC0000008] = "STATUS_INVALID_HANDLE",
        [0xC000000D] = "STATUS_INVALID_PARAMETER",
        [0xC0000022] = "STATUS_ACCESS_DENIED",
        [0xC0000023] = "STATUS_BUFFER_TOO_SMALL",
        [0xC0000061] = "STATUS_PRIVILEGE_NOT_HELD",
        [0xC0000062] = "STATUS_CANNOT_IMPERSONATE",
        [0xC00000BB] = "STATUS_NOT_SUPPORTED",
        [0xC0000143] = "STATUS_NOT_IMPLEMENTED",
    };

    private static readonly Dictionary<int, string> Win32 = new()
    {
        [0] = "ERROR_SUCCESS",
        [5] = "ERROR_ACCESS_DENIED",
        [6] = "ERROR_INVALID_HANDLE",
        [8] = "ERROR_NOT_ENOUGH_MEMORY",
        [13] = "ERROR_INVALID_DATA",
        [87] = "ERROR_INVALID_PARAMETER",
        [1300] = "ERROR_NOT_ALL_ASSIGNED",
        [1314] = "ERROR_PRIVILEGE_NOT_HELD",
        [1223] = "ERROR_CANCELLED (用户取消了 UAC 提示)",
        [1326] = "ERROR_LOGON_FAILURE",
    };

    internal static string Nt(uint status) =>
        NtStatus.TryGetValue(status, out var name) ? name : "UNKNOWN_NTSTATUS";

    internal static string Win32Error(int error) =>
        Win32.TryGetValue(error, out var name) ? name : "UNKNOWN_WIN32_ERROR";

    internal static string MemoryListCommandName(int command) => command switch
    {
        0 => "MemoryCaptureAccessedBits",
        1 => "MemoryCaptureAndResetAccessedBits",
        2 => "MemoryEmptyWorkingSets",
        3 => "MemoryFlushModifiedList",
        4 => "MemoryPurgeStandbyList",
        5 => "MemoryPurgeLowPriorityStandbyList",
        6 => "MemoryCommandMax",
        _ => "UNKNOWN_COMMAND"
    };
}

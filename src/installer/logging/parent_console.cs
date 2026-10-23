using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Text;

namespace AssetConverter.Logging
{
    internal enum StandardStream
    {
        Output,
        Error,
    }

    /// <summary>What <see cref="ParentConsole"/> needs from the operating system.</summary>
    internal interface IConsoleHost
    {
        /// <summary>Attaches the process to the console of the process that started it; false when that has none.</summary>
        bool AttachToParent();

        /// <summary>Whether the process was started with <paramref name="stream"/> already pointing somewhere (a pipe or a file).</summary>
        bool IsRedirected(StandardStream stream);

        /// <summary>A writer to the attached console for <paramref name="stream"/>.</summary>
        TextWriter OpenConsoleWriter(StandardStream stream);
    }

    /// <summary>
    /// The installer is a Windows GUI program, which has no console. Started from a terminal it attaches to the terminal's console and
    /// points <see cref="Console.Out"/> and <see cref="Console.Error"/> at it, so progress and errors print as they did for a console
    /// program. A stream the starter redirected to a pipe or file is left alone, and nothing happens when there is no terminal (a double click).
    /// </summary>
    internal static class ParentConsole
    {
        /// <returns>True when the process is now attached to a terminal's console.</returns>
        public static bool Attach()
        {
            if (!OperatingSystem.IsWindows()) return false;
            return Attach(new WindowsConsoleHost(), (stream, writer) =>
            {
                if (stream == StandardStream.Output) Console.SetOut(writer);
                else Console.SetError(writer);
            });
        }

        internal static bool Attach(IConsoleHost host, Action<StandardStream, TextWriter> bind)
        {
            var streams = Enum.GetValues<StandardStream>();
            var redirected = streams.Where(host.IsRedirected).ToHashSet();
            if (!host.AttachToParent()) return false;
            foreach (var stream in streams.Where(stream => !redirected.Contains(stream)))
                bind(stream, host.OpenConsoleWriter(stream));
            return true;
        }

        [SupportedOSPlatform("windows")]
        private sealed class WindowsConsoleHost : IConsoleHost
        {
            private const uint AttachParentProcess = 0xFFFFFFFF;
            private const uint GenericRead = 0x80000000;
            private const uint GenericWrite = 0x40000000;
            private const uint ShareReadWrite = 0x3;
            private const uint OpenExisting = 3;
            private static readonly IntPtr InvalidHandle = new(-1);

            public bool AttachToParent() => AttachConsole(AttachParentProcess);

            public bool IsRedirected(StandardStream stream)
            {
                IntPtr handle = GetStdHandle(HandleId(stream));
                return handle != IntPtr.Zero && handle != InvalidHandle;
            }

            public TextWriter OpenConsoleWriter(StandardStream stream)
            {
                IntPtr handle = CreateFile("CONOUT$", GenericRead | GenericWrite, ShareReadWrite, IntPtr.Zero, OpenExisting, 0, IntPtr.Zero);
                if (handle == InvalidHandle) throw new Win32Exception(Marshal.GetLastWin32Error(), "The console output of the terminal could not be opened.");
                if (!SetStdHandle(HandleId(stream), handle)) throw new Win32Exception(Marshal.GetLastWin32Error(), "The console output could not be made the standard stream.");
                var bytes = stream == StandardStream.Output ? Console.OpenStandardOutput() : Console.OpenStandardError();
                Encoding encoding = Console.OutputEncoding;
                return new StreamWriter(bytes, encoding.CodePage == Encoding.UTF8.CodePage ? new UTF8Encoding(encoderShouldEmitUTF8Identifier: false) : encoding) { AutoFlush = true };
            }

            private static int HandleId(StandardStream stream) => stream == StandardStream.Output ? -11 : -12;

            [DllImport("kernel32.dll", SetLastError = true)]
            [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
            [return: MarshalAs(UnmanagedType.Bool)]
            private static extern bool AttachConsole(uint processId);

            [DllImport("kernel32.dll", SetLastError = true)]
            [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
            private static extern IntPtr GetStdHandle(int standardHandle);

            [DllImport("kernel32.dll", SetLastError = true)]
            [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
            [return: MarshalAs(UnmanagedType.Bool)]
            private static extern bool SetStdHandle(int standardHandle, IntPtr handle);

            [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
            [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
            private static extern IntPtr CreateFile(string fileName, uint desiredAccess, uint shareMode, IntPtr securityAttributes,
                uint creationDisposition, uint flagsAndAttributes, IntPtr templateFile);
        }
    }
}

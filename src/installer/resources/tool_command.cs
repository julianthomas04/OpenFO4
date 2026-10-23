using System.Diagnostics;

namespace AssetConverter
{
    /// <summary>How to start an external tool: the program, the arguments that come before the caller's own, and the environment it needs.</summary>
    /// <param name="RequiredFiles">Every file that must exist for the tool to start, the program included.</param>
    internal sealed record ToolCommand(
        string FileName,
        IReadOnlyList<string> LeadingArguments,
        IReadOnlyDictionary<string, string> Environment,
        IReadOnlyList<string> RequiredFiles)
    {
        public static ToolCommand Program(string fileName) => new(fileName, [], new Dictionary<string, string>(), [fileName]);

        public string? MissingFile() => RequiredFiles.FirstOrDefault(file => !File.Exists(file));

        public void Apply(ProcessStartInfo startInfo, IEnumerable<string> arguments)
        {
            startInfo.FileName = FileName;
            foreach (string argument in LeadingArguments.Concat(arguments)) startInfo.ArgumentList.Add(argument);
            foreach (var (name, value) in Environment) startInfo.Environment[name] = value;
        }
    }
}

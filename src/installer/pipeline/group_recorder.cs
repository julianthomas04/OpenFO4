using System.Collections.Concurrent;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Writes each asset group into the ownership marker once its conversion is complete, unless something a rerun would repair went wrong
    /// while it was made (an archive whose entries did not all extract): that group is left out, so the next install converts it again, and
    /// the warning says why. Failures that no rerun cures (a NIF the converter does not support) are the conversion's own warnings and do not
    /// come here.
    /// </summary>
    internal sealed class GroupRecorder(InstallLocations paths, string installerVersion, GameInputs? inputs)
    {
        private readonly ConcurrentDictionary<AssetGroup, ConcurrentQueue<string>> _withheld = new();

        /// <summary>Keeps every group of <paramref name="groups"/> out of the marker for <paramref name="reason"/>, whether or not it has finished. Call before the group can finish.</summary>
        public void Withhold(IEnumerable<AssetGroup> groups, string reason)
        {
            foreach (var group in groups)
                _withheld.GetOrAdd(group, _ => new ConcurrentQueue<string>()).Enqueue(reason);
        }

        /// <exception cref="IOException">The marker could not be written.</exception>
        public void Complete(AssetGroup group)
        {
            if (_withheld.TryGetValue(group, out var reasons))
            {
                Console.Error.WriteLine(
                    $"WARNING: the '{AssetGroups.Id(group)}' asset group is not recorded as complete ({string.Join("; ", reasons.Distinct())}); the next install converts it again.");
                return;
            }
            paths.MarkGroupComplete(group, installerVersion, inputs);
        }
    }
}

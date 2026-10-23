using NiflySharp;
using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>A node's own NiTransformController (not part of a controller sequence): the animated channels and the timing they play under.</summary>
    internal sealed record StandaloneTransform(NodeTrack Track, ClipTiming Timing);

    /// <summary>Reads the NiTransformControllers hanging directly off a node, the way an animated marker or light mount is driven.</summary>
    internal static class StandaloneTransformControllers
    {
        public const string WithoutKeysCounter = "standalone transform controllers without key data";
        public const string InvalidKeysCounter = "standalone transform controllers with invalid key data (controller ignored)";

        /// <summary>Controllers whose interpolator is not a plain transform interpolator (blend interpolators driven by a controller sequence) are not standalone and are skipped.</summary>
        public static IReadOnlyList<StandaloneTransform> On(NifFile nif, NiObjectNET node)
        {
            var found = new List<StandaloneTransform>();
            foreach (var (_, controller) in ShaderControllerTracks.ControllerChain(nif, node))
            {
                if (controller is not NiTransformController transform)
                    continue;
                var read = Read(nif, node, transform);
                if (read != null)
                    found.Add(read);
            }
            return found;
        }

        private static StandaloneTransform? NoKeys()
        {
            NifAnimationStats.Add(WithoutKeysCounter);
            return null;
        }

        private static StandaloneTransform? Read(NifFile nif, NiObjectNET node, NiTransformController controller)
        {
            string nodeName = node.Name?.String ?? "";
            if (!NifControllerExtractor.TryGetBlock<NiTransformInterpolator>(nif, controller.Interpolator.Index, "transform controller interpolator", out var interpolator))
                return null;
            if (!NifControllerExtractor.TryGetBlock<NiTransformData>(nif, interpolator.Data.Index, "transform key data", out var data))
                return NoKeys();
            NodeTrack track;
            try
            {
                track = NifControllerExtractor.BuildTrack("standalone controller", nodeName, data);
            }
            catch (NifAnimationException)
            {
                NifAnimationStats.Add(InvalidKeysCounter);
                return null;
            }
            if (!track.HasKeys)
                return NoKeys();

            var times = track.Translation.Select(s => s.Time)
                .Concat(track.Rotation.Select(s => s.Time))
                .Concat(track.Scale.Select(s => s.Time))
                .ToList();
            return new StandaloneTransform(track, ClipTiming.Of(controller).CoveringKeys(times.Min(), times.Max()));
        }
    }
}

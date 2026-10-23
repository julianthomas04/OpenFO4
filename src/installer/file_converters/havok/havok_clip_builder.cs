using System.Diagnostics.CodeAnalysis;
using System.Numerics;

namespace AssetConverter.Havok
{
    /// <summary>Turns animation bindings into fully sampled clips and offers the one-call entry point for a packfile.</summary>
    public static class HavokClipBuilder
    {
        public static HavokAnimationFile ReadFile(byte[] hkxBytes) =>
            HavokAnimationFileReader.Read(HavokPackfile.Load(hkxBytes));

        /// <summary>False when the file is a valid packfile without animation content (behavior graph, character data, project).</summary>
        public static bool TryReadFile(byte[] hkxBytes, [NotNullWhen(true)] out HavokAnimationFile? file) =>
            HavokAnimationFileReader.TryRead(HavokPackfile.Load(hkxBytes), out file);

        /// <summary>One clip per binding; a single binding keeps <paramref name="baseName"/>, several get a numeric suffix.</summary>
        public static IReadOnlyList<HavokClip> BuildAll(HavokAnimationFile file, string baseName)
        {
            var clips = new List<HavokClip>(file.Bindings.Count);
            for (int i = 0; i < file.Bindings.Count; i++)
            {
                string name = file.Bindings.Count == 1 ? baseName : $"{baseName}_{i}";
                clips.Add(Build(name, file.Bindings[i]));
            }
            return clips;
        }

        public static HavokClip Build(string name, HavokAnimationBinding binding)
        {
            var animation = binding.Animation;
            var sampled = animation.SampleTracks();

            var tracks = new List<HavokClipTrack>(sampled.Length);
            for (int t = 0; t < sampled.Length; t++)
            {
                int bone = binding.TransformTrackToBoneIndices[t];
                if (bone < 0) continue;
                MakeRotationsContinuous(sampled[t]);
                tracks.Add(new HavokClipTrack(bone, sampled[t]));
            }

            return new HavokClip
            {
                Name = name,
                Duration = animation.Duration,
                FrameCount = animation.FrameCount,
                FrameDuration = animation.FrameDuration,
                BlendHint = binding.BlendHint,
                OriginalSkeletonName = binding.OriginalSkeletonName,
                Tracks = tracks,
                Annotations = FlattenAnnotations(animation.AnnotationTracks),
            };
        }

        /// <summary>
        /// Clips authored for a larger skeleton (the human one) also carry tracks for bones the creature skeleton lacks;
        /// those cannot drive anything and are removed.
        /// </summary>
        public static HavokClip DropTracksBeyondBone(HavokClip clip, int boneCount, out int droppedTracks)
        {
            var kept = clip.Tracks.Where(t => t.BoneIndex < boneCount).ToList();
            droppedTracks = clip.Tracks.Count - kept.Count;
            return droppedTracks == 0 ? clip : clip with { Tracks = kept };
        }

        private static List<HavokAnnotation> FlattenAnnotations(IReadOnlyList<HavokAnnotationTrack> tracks) =>
            tracks.SelectMany(t => t.Annotations).OrderBy(a => a.Time).ToList();

        /// <summary>q and -q are the same rotation; pick the sign nearest the previous frame so LINEAR glTF sampling never takes the long way round.</summary>
        private static void MakeRotationsContinuous(HavokTransform[] frames)
        {
            for (int f = 1; f < frames.Length; f++)
            {
                if (Quaternion.Dot(frames[f - 1].Rotation, frames[f].Rotation) < 0f)
                    frames[f] = frames[f] with { Rotation = Quaternion.Negate(frames[f].Rotation) };
            }
        }
    }
}

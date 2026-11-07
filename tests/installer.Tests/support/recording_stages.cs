using System.Collections.Concurrent;
using AssetConverter;
using AssetConverter.Pipeline;

namespace InstallerTests
{
    /// <summary>
    /// Pipeline stages that do no work: each call is recorded under its name and runs the hook registered for that name, if any, so a test sees which
    /// stage work the pipeline asked for and can make one fail. The few obligations a real stage has towards the lanes behind it (completing the
    /// work queues, signalling the render skeletons) are met the way a real stage meets them on success.
    /// </summary>
    internal sealed class RecordingStages(InstallLocations paths) : IPipelineStages
    {
        private readonly ConcurrentQueue<string> _calls = new();
        private readonly ConcurrentDictionary<string, Action> _hooks = new();

        public IReadOnlyCollection<string> Calls => [.. _calls];

        /// <summary>What <see cref="ExtractMeshArchives"/> reports: whether every archive entry extracted.</summary>
        public bool MeshArchivesExtractCleanly { get; set; } = true;

        /// <summary>What <see cref="ExtractTextures"/> reports: whether every texture archive extracted.</summary>
        public bool TexturesExtractCleanly { get; set; } = true;

        public void OnCall(string name, Action hook) => _hooks[name] = hook;

        private void Record(string name)
        {
            _calls.Enqueue(name);
            if (_hooks.TryGetValue(name, out var hook))
                hook();
        }

        public IDisposable ConfigureConverters()
        {
            Record(nameof(ConfigureConverters));
            return new Nothing();
        }

        public void InstallGame(GameFiles game) => Record(nameof(InstallGame));

        public bool ExtractMeshArchives(NifWorkQueue meshes, PriorityFiles skeletons, Action onInputsReady)
        {
            Record(nameof(ExtractMeshArchives));
            onInputsReady();
            meshes.Complete();
            return MeshArchivesExtractCleanly;
        }

        public TextureArchiveSet OpenAndIndexTextureArchives()
        {
            Record(nameof(OpenAndIndexTextureArchives));
            return TextureArchiveSet.Open(paths.GameDataDirectory, paths.StagingRoot, []);
        }

        public void CopyGameData(Action onEsmCopied)
        {
            Record(nameof(CopyGameData));
            onEsmCopied();
        }

        public void CopyGameInis() => Record(nameof(CopyGameInis));

        public bool ExtractTextures(TextureArchiveSet textures)
        {
            Record(nameof(ExtractTextures));
            return TexturesExtractCleanly;
        }

        public void ExtractColourLuts() => Record(nameof(ExtractColourLuts));

        public void TranslateShaders() => Record(nameof(TranslateShaders));

        public void ExtractInterfaceAssets() => Record(nameof(ExtractInterfaceAssets));

        public void ConvertAudio() => Record(nameof(ConvertAudio));

        public void ConvertVideo() => Record(nameof(ConvertVideo));

        public void ConvertMeshes(NifWorkQueue meshes, PriorityFiles skeletons)
        {
            Record(nameof(ConvertMeshes));
            skeletons.Expect([]);
        }

        public void QueueUnobservedCollisionCandidates() => Record(nameof(QueueUnobservedCollisionCandidates));

        public void ExtractCollision() => Record(nameof(ExtractCollision));

        public void ConvertAnimations() => Record(nameof(ConvertAnimations));

        public void BakePalettes() => Record(nameof(BakePalettes));

        public Task LoadDatabase()
        {
            Record(nameof(LoadDatabase));
            return Task.CompletedTask;
        }

        public void ScanLodWater() => Record(nameof(ScanLodWater));

        public void FinalizeDatabase(Task databaseLoaded) => Record(nameof(FinalizeDatabase));

        public StagingRemovalResult RemoveStaging() => throw new NotSupportedException("The wiring tests keep the staging folder.");

        private sealed class Nothing : IDisposable
        {
            public void Dispose()
            {
            }
        }
    }
}

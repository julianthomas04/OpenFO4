using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>
    /// The pipeline's own wiring through <see cref="InstallPipeline.RunAsync"/> with stages that only record what they are asked to do: for each
    /// class of change (one independent group, collision, animations, meshes with their dependents, textures and database, nothing, everything) the
    /// lanes neither hang nor run work the plan does not ask for, each converted group starts from a purged output, and the marker ends up right.
    /// </summary>
    [Collection("Console redirection")]
    public sealed class InstallPipelineWiringTests : IDisposable
    {
        private const string Version = "2.0.0";

        private static readonly GameInputs Inputs = new("en", new string('a', 64));
        private static readonly string[] Always = [nameof(IPipelineStages.InstallGame), nameof(IPipelineStages.ConfigureConverters), nameof(IPipelineStages.CopyGameInis)];

        private readonly TextWriter _originalOut = Console.Out;
        private readonly TextWriter _originalError = Console.Error;
        private readonly StringWriter _errors = new();
        private readonly TempDirectory _root = new();

        public InstallPipelineWiringTests()
        {
            Console.SetOut(new StringWriter());
            Console.SetError(_errors);
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            InstallProgress.Stop();
            _root.Dispose();
        }

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private string FontSentinel => Path.Combine(Locations.FontsOut, "font.ttf");

        private string AudioSentinel => Path.Combine(Locations.AudioOut, "a.flac");

        private string VideoSentinel => Path.Combine(Locations.VideoOut, "a.mp4");

        private string CollisionSentinel => Path.Combine(Locations.MeshesOut, "a.col.json");

        private string TextureSentinel => Path.Combine(Locations.TexturesOut, "a.dds");

        private string TextureMarker => Path.Combine(Locations.StagingRoot, ".Fallout4 - Textures1.ba2.extracted-direct");

        public static TheoryData<string> Changes() => ["audio", "collision", "animations", "meshes", "textures-database", "nothing", "everything"];

        private static AssetGroup Group(string id) => AssetGroups.TryFromId(id, out var group) ? group : throw new ArgumentException(id);

        /// <summary>The state of an install whose record lists every group but <paramref name="missing"/> ("nothing": all of them, "everything": no record).</summary>
        private ReconversionPlan InstalledExcept(string missing)
        {
            Locations.CreateOutputDirectories();
            foreach (string sentinel in new[] { FontSentinel, AudioSentinel, VideoSentinel, CollisionSentinel, TextureSentinel, Locations.DatabasePath, TextureMarker })
            {
                Directory.CreateDirectory(Path.GetDirectoryName(sentinel)!);
                File.WriteAllText(sentinel, "old");
            }
            if (missing == "everything")
                return ReconversionPlan.ReconvertAll;

            var revisions = AssetGroups.CurrentRevisions.Where(pair => missing == "nothing" || pair.Key != Group(missing)).ToDictionary(pair => pair.Key, pair => pair.Value);
            var installed = new InstalledAssets("1.0.0", revisions, Inputs);
            AssetVersionMarker.Write(Locations.OwnershipMarker, installed);
            return ReconversionPlan.Decide(installed, AssetGroups.CurrentRevisions);
        }

        private InstallPipeline Pipeline(ReconversionPlan plan, IPipelineStages stages)
        {
            var options = new InstallOptions(KeepStaging: true, AssetVersion: Version, Reconversion: plan, GameInputs: Inputs);
            InstallProgress.Start(InstallSteps.Planned(options.KeepStaging, reconversion: plan));
            return new InstallPipeline(Locations, options, MemoryBudget.Unlimited(), stages, Ct);
        }

        private static Task Finishes(InstallPipeline pipeline) => pipeline.RunAsync().WithTimeout();

        private static List<string> ExpectedCalls(ReconversionPlan plan)
        {
            var calls = new List<string>(Always);
            void When(AssetGroup group, params string[] names)
            {
                if (plan.Reconverts(group)) calls.AddRange(names);
            }
            When(AssetGroup.Shaders, nameof(IPipelineStages.TranslateShaders));
            When(AssetGroup.Interface, nameof(IPipelineStages.ExtractInterfaceAssets));
            When(AssetGroup.Audio, nameof(IPipelineStages.ConvertAudio));
            When(AssetGroup.Video, nameof(IPipelineStages.ConvertVideo));
            When(AssetGroup.Meshes, nameof(IPipelineStages.ConvertMeshes));
            When(AssetGroup.Collision, nameof(IPipelineStages.QueueUnobservedCollisionCandidates), nameof(IPipelineStages.ExtractCollision));
            When(AssetGroup.Animations, nameof(IPipelineStages.ConvertAnimations));
            When(AssetGroup.TexturesDatabase, nameof(IPipelineStages.OpenAndIndexTextureArchives), nameof(IPipelineStages.CopyGameData), nameof(IPipelineStages.ExtractTextures),
                nameof(IPipelineStages.ExtractColourLuts), nameof(IPipelineStages.BakePalettes), nameof(IPipelineStages.LoadDatabase), nameof(IPipelineStages.ScanLodWater),
                nameof(IPipelineStages.FinalizeDatabase));
            if (new[] { AssetGroup.Meshes, AssetGroup.Collision, AssetGroup.Animations, AssetGroup.TexturesDatabase }.Any(plan.Reconverts))
                calls.Add(nameof(IPipelineStages.ExtractMeshArchives));
            return calls;
        }

        private List<AssetGroup> Recorded() => [.. (AssetVersionMarker.Read(Locations.OwnershipMarker)?.Revisions.Keys ?? []).Order()];

        [Theory]
        [MemberData(nameof(Changes))]
        public async Task EachClassOfChangeRunsExactlyTheWorkItNeedsWithoutHangingAndEndsWithEveryGroupRecorded(string change)
        {
            var plan = InstalledExcept(change);
            var stages = new RecordingStages(Locations);

            await Finishes(Pipeline(plan, stages));

            Assert.Equal(ExpectedCalls(plan).Order(), stages.Calls.Distinct().Order());
            var marker = AssetVersionMarker.Read(Locations.OwnershipMarker)!;
            Assert.Equal(AssetGroups.All.Order(), Recorded());
            Assert.Equal(AssetGroups.CurrentRevisions.OrderBy(pair => pair.Key), marker.Revisions.OrderBy(pair => pair.Key));
            Assert.Equal(Version, marker.Version);
            Assert.Equal(Inputs, marker.Inputs);
        }

        [Theory]
        [MemberData(nameof(Changes))]
        public async Task EachConvertedGroupStartsFromAPurgedOutputAndAKeptGroupKeepsIt(string change)
        {
            var plan = InstalledExcept(change);

            await Finishes(Pipeline(plan, new RecordingStages(Locations)));

            Assert.Equal(!plan.Reconverts(AssetGroup.Interface), File.Exists(FontSentinel));
            Assert.Equal(!plan.Reconverts(AssetGroup.Audio), File.Exists(AudioSentinel));
            Assert.Equal(!plan.Reconverts(AssetGroup.Video), File.Exists(VideoSentinel));
            Assert.Equal(!plan.Reconverts(AssetGroup.Collision), File.Exists(CollisionSentinel));
            Assert.Equal(!plan.Reconverts(AssetGroup.TexturesDatabase), File.Exists(TextureSentinel));
            Assert.Equal(!plan.Reconverts(AssetGroup.TexturesDatabase), File.Exists(Locations.DatabasePath));
            Assert.Equal(!plan.Reconverts(AssetGroup.TexturesDatabase), File.Exists(TextureMarker));
        }

        [Fact]
        public async Task TheTextureIndexAndTheInterfacePlanSeeAPurgedOutput()
        {
            var plan = InstalledExcept("everything");
            var stages = new RecordingStages(Locations);
            bool markerGoneAtTheIndex = false, fontsGoneAtThePlan = false;
            stages.OnCall(nameof(IPipelineStages.OpenAndIndexTextureArchives), () => markerGoneAtTheIndex = !File.Exists(TextureMarker));
            stages.OnCall(nameof(IPipelineStages.ExtractInterfaceAssets), () => fontsGoneAtThePlan = !File.Exists(FontSentinel));

            await Finishes(Pipeline(plan, stages));

            Assert.True(markerGoneAtTheIndex);
            Assert.True(fontsGoneAtThePlan);
        }

        [Fact]
        public async Task AFailedMeshConversionFailsTheRunPromptlyAndLeavesTheGroupsItWithdrewOutOfTheMarker()
        {
            var plan = InstalledExcept("meshes");
            var stages = new RecordingStages(Locations);
            var failure = new InvalidOperationException("the NIF conversion failed");
            stages.OnCall(nameof(IPipelineStages.ConvertMeshes), () => throw failure);

            var thrown = await Assert.ThrowsAsync<InvalidOperationException>(() => Finishes(Pipeline(plan, stages)));

            Assert.Same(failure, thrown);
            var recorded = Recorded();
            Assert.DoesNotContain(AssetGroup.Meshes, recorded);
            Assert.DoesNotContain(AssetGroup.Animations, recorded);
            Assert.DoesNotContain(AssetGroup.TexturesDatabase, recorded);
            Assert.Contains(AssetGroup.Shaders, recorded);
        }

        [Fact]
        public async Task AMeshArchiveThatDidNotExtractCleanlyLeavesEveryGroupThatReadItUnrecordedAndTheRunStillFinishes()
        {
            var plan = InstalledExcept("meshes");
            var stages = new RecordingStages(Locations) { MeshArchivesExtractCleanly = false };

            await Finishes(Pipeline(plan, stages));

            Assert.Equal(AssetGroups.All.Where(group => group is not (AssetGroup.Meshes or AssetGroup.Animations or AssetGroup.TexturesDatabase)).Order(), Recorded());
            Assert.Contains("the 'meshes' asset group is not recorded as complete", _errors.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public async Task AFailedMeshExtractionForCollisionAloneWithholdsOnlyCollision()
        {
            var plan = InstalledExcept("collision");
            var stages = new RecordingStages(Locations) { MeshArchivesExtractCleanly = false };

            await Finishes(Pipeline(plan, stages));

            Assert.Equal(AssetGroups.All.Where(group => group != AssetGroup.Collision).Order(), Recorded());
        }

        [Fact]
        public async Task ATextureArchiveThatDidNotExtractCleanlyLeavesTexturesAndDatabaseUnrecorded()
        {
            var plan = InstalledExcept("textures-database");
            var stages = new RecordingStages(Locations) { TexturesExtractCleanly = false };

            await Finishes(Pipeline(plan, stages));

            Assert.Equal(AssetGroups.All.Where(group => group != AssetGroup.TexturesDatabase).Order(), Recorded());
            Assert.Contains("the 'textures-database' asset group is not recorded as complete", _errors.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public async Task AFailedGameCopyLeavesTheMarkerAsItWasBeforeAnythingIsWithdrawn()
        {
            var plan = InstalledExcept("meshes");
            string before = File.ReadAllText(Locations.OwnershipMarker);
            var stages = new RecordingStages(Locations);
            stages.OnCall(nameof(IPipelineStages.InstallGame), () => throw new IOException("the game is running"));

            await Assert.ThrowsAsync<IOException>(() => Finishes(Pipeline(plan, stages)));

            Assert.Equal(before, File.ReadAllText(Locations.OwnershipMarker));
        }
    }
}

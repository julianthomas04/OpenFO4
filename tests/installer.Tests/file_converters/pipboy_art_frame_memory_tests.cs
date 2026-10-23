using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    // The in-process half of a Pip-Boy art job: the frames ffdec exported are read, rebased and written under a memory lease sized
    // from the frame files.
    public sealed class PipboyArtFrameMemoryTests
    {
        static readonly TimeSpan Generous = TimeSpan.FromSeconds(30);

        static CancellationToken Ct => TestContext.Current.CancellationToken;

        const string Frame =
            "<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"20.0px\" width=\"10.0px\"><g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 5.0, 10.0)\"><path d=\"M0 0\"/></g></svg>";

        static readonly ArtBounds Bounds = new(-5, -10, 5, 10);

        static ExportedFrameFile WriteFrame(string directory, string name, string svg)
        {
            string path = Path.Combine(directory, name);
            File.WriteAllText(path, svg);
            return new ExportedFrameFile(path, $"clip '{name}'");
        }

        static PipboyArtClip SymbolClip(IReadOnlyList<SvgFrame> frames) =>
            PipboyArtExporter.SymbolClip("icon/x", "Page.swf", null, ArtColour.Mask, 30, [], frames);

        static PipboyArtClip MovieClip(IReadOnlyList<string> frames) =>
            new("vaultboy/x", "Boy.swf", null, ArtColour.Mask, 30, [], Bounds, frames);

        [Fact]
        public void SymbolClip_ReservesFourBytesPerFrameFileByteOfEveryFrameOfTheClip()
        {
            using var dir = new TempDirectory();
            var first = WriteFrame(dir.Path, "1.svg", Frame);
            var second = WriteFrame(dir.Path, "2.svg", Frame.Replace("M0 0", "M0 0 L1 1"));
            var budget = new MemoryBudget(long.MaxValue);
            long reservedWhileBuilding = -1;

            PipboyArtExporter.AddSymbolClip(new PipboyArtWriter(Path.Combine(dir.Path, "art")), budget, [first, second, first], frames =>
            {
                reservedWhileBuilding = budget.ReservedBytes;
                return SymbolClip(frames);
            }, Ct);

            long frameBytes = 2 * new FileInfo(first.Path).Length + new FileInfo(second.Path).Length;
            Assert.Equal(MemoryEstimates.PipboySymbolClipBytes(frameBytes), reservedWhileBuilding);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public void MovieClip_ReservesTwoBytesPerFrameFileByteAndRebasesEveryFrame()
        {
            using var dir = new TempDirectory();
            var first = WriteFrame(dir.Path, "1.svg", Frame);
            var second = WriteFrame(dir.Path, "2.svg", Frame.Replace("M0 0", "M0 0 L1 1"));
            var budget = new MemoryBudget(long.MaxValue);
            long reservedWhileBuilding = -1;
            IReadOnlyList<string> rebased = [];

            PipboyArtExporter.AddMovieClip(new PipboyArtWriter(Path.Combine(dir.Path, "art")), budget, [first, second], Bounds, frames =>
            {
                reservedWhileBuilding = budget.ReservedBytes;
                rebased = frames;
                return MovieClip(frames);
            }, Ct);

            long frameBytes = new FileInfo(first.Path).Length + new FileInfo(second.Path).Length;
            Assert.Equal(MemoryEstimates.PipboyMovieClipBytes(frameBytes), reservedWhileBuilding);
            Assert.Equal([SvgFrame.Parse(File.ReadAllText(first.Path)).Rebase(Bounds), SvgFrame.Parse(File.ReadAllText(second.Path)).Rebase(Bounds)], rebased);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task SymbolClip_BudgetTakenByOtherWork_WaitsAndIsWrittenOnceItIsReleased()
        {
            using var dir = new TempDirectory();
            var frame = WriteFrame(dir.Path, "1.svg", Frame);
            var budget = new MemoryBudget(MemoryEstimates.PipboySymbolClipBytes(new FileInfo(frame.Path).Length));
            var writer = new PipboyArtWriter(Path.Combine(dir.Path, "art"));
            var held = budget.Reserve(1, Ct);

            var job = Task.Run(() => PipboyArtExporter.AddSymbolClip(writer, budget, [frame], SymbolClip, Ct), Ct);

            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Generous));
            Assert.False(job.IsCompleted);
            Assert.False(Directory.Exists(Path.Combine(dir.Path, "art", PipboyArtWriter.ClipFolder("icon/x"))));

            held.Dispose();
            await job.WaitAsync(Generous, Ct);

            Assert.Equal(1, writer.Complete(["icon/x"]));
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task MovieClip_BudgetTakenByOtherWork_WaitsAndIsWrittenOnceItIsReleased()
        {
            using var dir = new TempDirectory();
            var frame = WriteFrame(dir.Path, "1.svg", Frame);
            var budget = new MemoryBudget(MemoryEstimates.PipboyMovieClipBytes(new FileInfo(frame.Path).Length));
            var writer = new PipboyArtWriter(Path.Combine(dir.Path, "art"));
            var held = budget.Reserve(1, Ct);

            var job = Task.Run(() => PipboyArtExporter.AddMovieClip(writer, budget, [frame], Bounds, MovieClip, Ct), Ct);

            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Generous));
            Assert.False(job.IsCompleted);

            held.Dispose();
            await job.WaitAsync(Generous, Ct);

            Assert.Equal(1, writer.Complete(["vaultboy/x"]));
            Assert.Equal(0, budget.ReservedBytes);
        }
    }
}

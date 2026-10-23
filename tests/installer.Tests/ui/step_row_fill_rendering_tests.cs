using System.Globalization;
using System.Runtime.InteropServices;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Progress;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Media.Imaging;
using Avalonia.Styling;
using Avalonia.Threading;
using Avalonia.VisualTree;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Ui
{
    /// <summary>The installer window rendered by Skia to pixels, so the fill behind each step row is checked as the user sees it.</summary>
    [Collection("Avalonia")]
    public sealed class StepRowFillRenderingTests
    {
        private const int PixelTolerance = 6;
        private readonly HeadlessUnitTestSession _session = HeadlessSessions.Shared;

        private sealed record RowPixels(byte[] Frame, int Stride, Rect Bounds)
        {
            public (byte R, byte G, byte B) At(double fractionOfWidth)
            {
                int x = (int)(Bounds.X + (Bounds.Width * fractionOfWidth));
                int y = (int)Bounds.Y + 1;
                int offset = (y * Stride) + (x * 4);
                return (Frame[offset + 2], Frame[offset + 1], Frame[offset]);
            }
        }

        private static bool Differ((byte R, byte G, byte B) a, (byte R, byte G, byte B) b) =>
            Math.Abs(a.R - b.R) > PixelTolerance || Math.Abs(a.G - b.G) > PixelTolerance || Math.Abs(a.B - b.B) > PixelTolerance;

        private static (byte[] Frame, int Stride) Pixels(WriteableBitmap bitmap)
        {
            using var framebuffer = bitmap.Lock();
            byte[] bytes = new byte[framebuffer.RowBytes * framebuffer.Size.Height];
            Marshal.Copy(framebuffer.Address, bytes, 0, bytes.Length);
            return (bytes, framebuffer.RowBytes);
        }

        private (RowPixels Done, RowPixels Half, RowPixels Pending) Render(ThemeVariant theme, string pngPath)
        {
            return _session.Dispatch(() =>
            {
                var model = new ProgressModel(
                    [new StepDefinition("Extracting", "files"), new StepDefinition("Converting meshes", "meshes"), new StepDefinition("Packing archives", "files")],
                    new ManualTimeProvider());
                int done = model.BeginPhase("Extracting", 10, null);
                model.Tick(done, 10);
                model.EndPhase(done);
                int half = model.BeginPhase("Converting meshes", 100, null);
                model.Tick(half, 50);
                var viewModel = new InstallerViewModel(model.GetSnapshot, new LogBuffer(InstallerViewModel.LogCapacity), () => { }, culture: CultureInfo.InvariantCulture);
                var window = new InstallerWindow { DataContext = viewModel, RequestedThemeVariant = theme };
                window.Show();
                viewModel.Refresh();
                window.UpdateLayout();
                Dispatcher.UIThread.RunJobs();
                window.UpdateLayout();

                var bitmap = window.CaptureRenderedFrame() ?? throw new InvalidOperationException("The headless platform rendered no frame.");
                bitmap.Save(pngPath);
                var (frame, stride) = Pixels(bitmap);
                var rows = window.GetVisualDescendants().OfType<ProgressBar>().Where(b => b.Classes.Contains("rowFill"))
                    .Select(b => new RowPixels(frame, stride, new Rect(b.TranslatePoint(default, window)!.Value, b.Bounds.Size))).ToList();
                return (rows[0], rows[1], rows[2]);
            }, CancellationToken.None).GetAwaiter().GetResult();
        }

        [Theory]
        [InlineData("light", "rowfill_light.png")]
        [InlineData("dark", "rowfill_dark.png")]
        public void ARunningRowIsFilledUpToItsFractionAndPendingRowsAreNot(string theme, string fileName)
        {
            var variant = theme == "light" ? ThemeVariant.Light : ThemeVariant.Dark;

            var (done, half, pending) = Render(variant, RenderOutput.PathFor(fileName));

            var pendingBackground = pending.At(0.5);
            Assert.True(Differ(half.At(0.25), half.At(0.75)), "the left half of the half-full row must be filled and the right half not");
            Assert.True(Differ(half.At(0.25), pendingBackground), "the filled part must differ from the pending row's background");
            Assert.False(Differ(half.At(0.75), pendingBackground), "the unfilled part must look like the pending row");
            Assert.True(Differ(done.At(0.9), pendingBackground), "a done row is filled across its width");
            Assert.False(Differ(pending.At(0.1), pendingBackground));
        }
    }
}

using System.Globalization;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Progress;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Presenters;
using Avalonia.Controls.Primitives;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.Threading;
using Avalonia.VisualTree;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Ui
{
    /// <summary>The installer window at its default size, filled with the worst case the real install produces, checked for clipped or covered text.</summary>
    [Collection("Avalonia")]
    public sealed class LayoutRenderingTests
    {
        private const string LongInstallLog = @"C:\Users\someone\Desktop\Games\Fallout 4 Open Edition\OpenFO4\install.log";
        private const double Epsilon = 0.5;
        private readonly HeadlessUnitTestSession _session = HeadlessSessions.Shared;

        private static void Complete(ProgressModel model, string name, long total = 10)
        {
            int phase = model.BeginPhase(name, total, null);
            model.Tick(phase, total);
            model.EndPhase(phase);
        }

        private static LogBuffer FullLog()
        {
            var log = new LogBuffer(InstallerViewModel.LogCapacity);
            for (int i = 0; i < 400; i++)
                log.WriteLine($"11:59:{i % 60:00}.{i % 1000:000}   Unpacking Fallout4 - Textures{i % 9}.ba2 -> C:\\Users\\someone\\Desktop\\Games\\OpenFO4\\assets\\textures\\very\\long\\path\\{i}", isError: i % 50 == 0);
            return log;
        }

        private static ProgressModel FourStepsRunning()
        {
            var model = new ProgressModel(InstallSteps.Planned(keepStaging: false, unpacksPayload: true), new ManualTimeProvider());
            foreach (var name in new[] { InstallSteps.UnpackPayload, InstallSteps.InstallGame, InstallSteps.CopyGameData, InstallSteps.ExtractMeshes, InstallSteps.ExtractInterface, InstallSteps.ConvertShaders })
                Complete(model, name);
            foreach (var name in new[] { InstallSteps.ExtractTextures, InstallSteps.ConvertMeshes, InstallSteps.ExtractCollision, InstallSteps.ConvertAnimations })
                model.Tick(model.BeginPhase(name, 100, null), 40);
            return model;
        }

        private static ProgressModel Finished()
        {
            var model = new ProgressModel(InstallSteps.Planned(keepStaging: false, unpacksPayload: true), new ManualTimeProvider());
            foreach (var unit in InstallSteps.Planned(keepStaging: false, unpacksPayload: true)) Complete(model, unit.Name);
            model.Finish(RunState.Succeeded);
            return model;
        }

        private static InstallerWindow ShowWindow(ProgressModel model, ThemeVariant theme)
        {
            var viewModel = new InstallerViewModel(model.GetSnapshot, FullLog(), () => { }, LongInstallLog, CultureInfo.InvariantCulture);
            var window = new InstallerWindow { DataContext = viewModel, RequestedThemeVariant = theme };
            window.Show();
            for (int pass = 0; pass < 4; pass++)
            {
                viewModel.Refresh();
                window.UpdateLayout();
                Dispatcher.UIThread.RunJobs();
            }
            return window;
        }

        private static double UnwrappedSize(TextBlock block, bool width)
        {
            var probe = new TextBlock { Text = block.Text, FontSize = block.FontSize, FontWeight = block.FontWeight, FontFamily = block.FontFamily };
            probe.Measure(Size.Infinity);
            return width ? probe.DesiredSize.Width : probe.DesiredSize.Height;
        }

        private static void AssertShownInFull(TextBlock block)
        {
            Assert.Equal(TextTrimming.None, block.TextTrimming);
            Assert.True(UnwrappedSize(block, width: true) <= block.Bounds.Width + Epsilon, $"'{block.Text}' needs {UnwrappedSize(block, true)} px but only {block.Bounds.Width} px are available");
            Assert.True(block.Bounds.Height <= UnwrappedSize(block, width: false) + Epsilon, $"'{block.Text}' wraps to {block.Bounds.Height} px height");
        }

        private static void AssertLogIsNotClippedOrCovered(InstallerWindow window)
        {
            var scroll = window.FindControl<ScrollViewer>("LogScroll")!;
            double intoLine = scroll.Offset.Y % LayoutConverters.LogLineHeight;
            Assert.True(intoLine < Epsilon || LayoutConverters.LogLineHeight - intoLine < Epsilon, $"the first visible log line starts {intoLine} px into a line");
            Assert.True(scroll.Offset.Y + scroll.Viewport.Height >= scroll.Extent.Height - LayoutConverters.LogLineHeight, "the log follows the newest line");

            var text = scroll.GetVisualDescendants().OfType<ScrollContentPresenter>().Single();
            var textArea = new Rect(text.TranslatePoint(default, window)!.Value, text.Bounds.Size);
            foreach (var bar in scroll.GetVisualDescendants().OfType<ScrollBar>().Where(b => b.IsVisible && b.Bounds.Width > 0))
            {
                var barArea = new Rect(bar.TranslatePoint(default, window)!.Value, bar.Bounds.Size);
                Assert.False(barArea.Intersects(textArea), $"the {bar.Orientation} scrollbar {barArea} covers the log text {textArea}");
            }
        }

        private static void AssertNoTextIsLost(InstallerWindow window)
        {
            AssertShownInFull(window.GetVisualDescendants().OfType<TextBlock>().Single(b => b.Classes.Contains("overall")));
            AssertShownInFull(window.FindControl<TextBlock>("LogFileLabel")!);
            AssertLogIsNotClippedOrCovered(window);
        }

        private static void Capture(InstallerWindow window, string fileName)
        {
            var bitmap = window.CaptureRenderedFrame() ?? throw new InvalidOperationException("The headless platform rendered no frame.");
            bitmap.Save(RenderOutput.PathFor(fileName));
        }

        private void CheckAndCapture(Func<ProgressModel> model, ThemeVariant theme, string fileName) =>
            _session.Dispatch(() =>
            {
                var window = ShowWindow(model(), theme);
                AssertNoTextIsLost(window);
                Capture(window, fileName);
                return 0;
            }, CancellationToken.None).GetAwaiter().GetResult();

        [Fact]
        public void FourRunningStepsWithAFullLogAndALongPathLoseNoTextInTheDarkTheme() =>
            CheckAndCapture(FourStepsRunning, ThemeVariant.Dark, "layout_running.png");

        [Fact]
        public void FourRunningStepsWithAFullLogAndALongPathLoseNoTextInTheLightTheme() =>
            CheckAndCapture(FourStepsRunning, ThemeVariant.Light, "layout_running_light.png");

        [Fact]
        public void TheFinishedStateLosesNoText() =>
            CheckAndCapture(Finished, ThemeVariant.Dark, "layout_finished.png");

        [Theory]
        [InlineData(0, 0)]
        [InlineData(180, 0)]
        [InlineData(200, 2)]
        [InlineData(17, 17)]
        public void TheLogLeavesBlankSpaceBelowTheLastLineOnlyForTheViewportRemainderOfAWholeLine(double viewport, double expected) =>
            Assert.Equal(expected, LayoutConverters.LogEndPaddingFor(viewport));
    }
}

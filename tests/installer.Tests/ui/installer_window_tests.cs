using System.Globalization;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Progress;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Headless;
using Avalonia.Skia;
using Avalonia.Themes.Fluent;
using Avalonia.Threading;
using Avalonia.VisualTree;
using InstallerTests.Support;
using InstallerTests.Ui;
using Xunit;

namespace InstallerTests.Ui
{
    public class HeadlessTestApplication : Application
    {
        public override void Initialize() => Styles.Add(new FluentTheme());
    }

    public static class HeadlessEntry
    {
        public static AppBuilder BuildAvaloniaApp() => AppBuilder.Configure<HeadlessTestApplication>().UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false }).UseSkia();
    }

    /// <summary>The one headless session of the test process, shared by the test classes of several collections; it ends with the process.</summary>
    public static class HeadlessSessions
    {
        private static readonly Lazy<HeadlessUnitTestSession> SharedSession = new(() => Task.Run(() => HeadlessUnitTestSession.StartNew(typeof(HeadlessEntry))).GetAwaiter().GetResult());

        public static HeadlessUnitTestSession Shared => SharedSession.Value;
    }

    /// <summary>Every test class of the "Avalonia" collection shares the one headless session, because a process can run only one.</summary>
    [CollectionDefinition("Avalonia")]
    public sealed class AvaloniaSharedSession
    {
    }

    /// <summary>The window running for real on Avalonia's headless platform: layout, bindings and the log's follow-the-tail behaviour.</summary>
    [Collection("Avalonia")]
    public sealed class InstallerWindowTests
    {
        private readonly HeadlessUnitTestSession _session;
        private readonly ManualTimeProvider _clock = new();
        private readonly ProgressModel _model;
        private readonly LogBuffer _log = new(InstallerViewModel.LogCapacity);
        private readonly InstallerViewModel _viewModel;
        private int _cancelRequests;

        public InstallerWindowTests()
        {
            _session = HeadlessSessions.Shared;
            _model = new ProgressModel(
                [new StepDefinition("Extracting", "files"), new StepDefinition("Converting meshes", "meshes")], _clock);
            _viewModel = new InstallerViewModel(_model.GetSnapshot, _log, () => _cancelRequests++, culture: CultureInfo.InvariantCulture);
        }

        private Task<T> OnUiThread<T>(Func<T> action) => _session.Dispatch(action, CancellationToken.None);

        private static InstallerWindow Show(InstallerViewModel viewModel)
        {
            var window = new InstallerWindow { DataContext = viewModel };
            window.Show();
            window.UpdateLayout();
            Dispatcher.UIThread.RunJobs();
            return window;
        }

        private static string[] TextOf(Visual root) => [.. root.GetVisualDescendants().OfType<TextBlock>().Select(t => t.Text ?? "")];

        [Fact]
        public async Task TheWindowShowsTheProgressDeclaredInTheModel()
        {
            string[] texts = await OnUiThread(() =>
            {
                int phase = _model.BeginPhase("Converting meshes", 63_000, null);
                _model.Tick(phase, 12_345);
                _log.WriteLine("Unpacking archive", isError: false);
                var window = Show(_viewModel);
                _viewModel.Refresh();
                window.UpdateLayout();
                var result = TextOf(window);
                return result;
            });

            Assert.Contains("0 of 2 steps done · 1 running", texts);
            Assert.Contains("12,345 / 63,000", texts);
            Assert.Contains("Extracting", texts);
            Assert.Contains(texts, t => t.EndsWith("Unpacking archive", StringComparison.Ordinal));
        }

        [Fact]
        public async Task TheActionButtonCancelsWhileRunningAndClosesWhenFinished()
        {
            var (before, after) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                var button = window.FindControl<Button>("ActionButton")!;
                string labelWhileRunning = (string)button.Content!;
                _model.Finish(RunState.Succeeded);
                _viewModel.Refresh();
                Dispatcher.UIThread.RunJobs();
                var labelAfter = ((string)button.Content!, button.IsEnabled);
                return (labelWhileRunning, labelAfter);
            });

            Assert.Equal("Cancel", before);
            Assert.Equal(("Close", true), after);
        }

        [Fact]
        public async Task TheLogFollowsNewLinesUntilTheUserScrollsUp()
        {
            var (followedToBottom, keptPositionWhileScrolledUp) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                var scroll = window.FindControl<ScrollViewer>("LogScroll")!;

                void Append(int from, int count)
                {
                    for (int i = from; i < from + count; i++) _log.WriteLine($"line {i}", isError: false);
                    _viewModel.Refresh();
                    window.UpdateLayout();
                    Dispatcher.UIThread.RunJobs();
                    window.UpdateLayout();
                }

                Append(0, 400);
                bool atBottom = scroll.Offset.Y + scroll.Viewport.Height >= scroll.Extent.Height - 2;

                scroll.Offset = new Vector(0, 0);
                window.UpdateLayout();
                Dispatcher.UIThread.RunJobs();
                Append(400, 400);
                bool stayedAtTop = scroll.Offset.Y == 0;

                return (atBottom, stayedAtTop);
            });

            Assert.True(followedToBottom);
            Assert.True(keptPositionWhileScrolledUp);
        }

        [Fact]
        public async Task TheLogKeepsFollowingTheNewestLineWhileTheRingIsFullAndOldLinesFallOff()
        {
            var (atBottom, count, lastText) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                var scroll = window.FindControl<ScrollViewer>("LogScroll")!;

                void Append(int from, int count)
                {
                    for (int i = from; i < from + count; i++) _log.WriteLine($"line {i}", isError: false);
                    _viewModel.Refresh();
                    window.UpdateLayout();
                    Dispatcher.UIThread.RunJobs();
                    window.UpdateLayout();
                }

                Append(0, InstallerViewModel.LogCapacity);
                Append(InstallerViewModel.LogCapacity, 300);
                Append(InstallerViewModel.LogCapacity + 300, 300);
                bool followed = scroll.Offset.Y + scroll.Viewport.Height >= scroll.Extent.Height - 2;
                return (followed, _viewModel.LogLines.Count, _viewModel.LogLines[^1].Text);
            });

            Assert.True(atBottom);
            Assert.Equal(InstallerViewModel.LogCapacity, count);
            Assert.EndsWith($"line {InstallerViewModel.LogCapacity + 599}", lastText, StringComparison.Ordinal);
        }

        [Fact]
        public async Task ClickingCancelWhileAConfirmationIsOpenDoesNotStackAnotherOneAndTheDialogCanBeOpenedAgainAfterwards()
        {
            var (afterTwoClicks, afterClosingWindow, afterReopen) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                var button = window.FindControl<Button>("ActionButton")!;

                void Click()
                {
                    button.RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(Button.ClickEvent));
                    Dispatcher.UIThread.RunJobs();
                }

                void CloseDialogs()
                {
                    foreach (var dialog in window.OwnedWindows.ToList()) dialog.Close(false);
                    Dispatcher.UIThread.RunJobs();
                }

                Click();
                Click();
                int two = window.OwnedWindows.Count;
                window.Close();
                Dispatcher.UIThread.RunJobs();
                int closing = window.OwnedWindows.Count;
                CloseDialogs();
                Click();
                int reopened = window.OwnedWindows.Count;
                CloseDialogs();
                _model.Finish(RunState.Succeeded);
                _viewModel.Refresh();
                window.Close();
                return (two, closing, reopened);
            });

            Assert.Equal(1, afterTwoClicks);
            Assert.Equal(1, afterClosingWindow);
            Assert.Equal(1, afterReopen);
            Assert.Equal(0, _cancelRequests);
        }

        private static void Pump(InstallerWindow window)
        {
            Dispatcher.UIThread.RunJobs();
            window.UpdateLayout();
            Dispatcher.UIThread.RunJobs();
        }

        private void FinishRun(InstallerWindow window, RunState state)
        {
            _model.Finish(state);
            _viewModel.Refresh();
            Pump(window);
        }

        [Theory]
        [InlineData(RunState.Succeeded)]
        [InlineData(RunState.Failed)]
        [InlineData(RunState.Cancelled)]
        public async Task ARunThatEndsWhileTheCloseConfirmationIsOpenDismissesItAndClosesTheWindow(RunState endState)
        {
            var (dialogsOpen, dialogsAfter, windowStillOpen, cancelRequests) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                window.Close();
                Pump(window);
                int open = window.OwnedWindows.Count;
                FinishRun(window, endState);
                var result = (open, window.OwnedWindows.Count, window.IsVisible, _cancelRequests);
                return result;
            });

            Assert.Equal(1, dialogsOpen);
            Assert.Equal(0, dialogsAfter);
            Assert.False(windowStillOpen);
            Assert.Equal(0, cancelRequests);
        }

        [Fact]
        public async Task ARunThatEndsWhileTheCancelButtonConfirmationIsOpenDismissesItAndLeavesTheCompletionShowing()
        {
            var (dialogsOpen, dialogsAfter, windowStillOpen, buttonLabel, cancelRequests) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                var button = window.FindControl<Button>("ActionButton")!;
                button.RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(Button.ClickEvent));
                Pump(window);
                int open = window.OwnedWindows.Count;
                FinishRun(window, RunState.Succeeded);
                var result = (open, window.OwnedWindows.Count, window.IsVisible, (string)button.Content!, _cancelRequests);
                window.Close();
                return result;
            });

            Assert.Equal(1, dialogsOpen);
            Assert.Equal(0, dialogsAfter);
            Assert.True(windowStillOpen);
            Assert.Equal("Close", buttonLabel);
            Assert.Equal(0, cancelRequests);
        }

        [Fact]
        public async Task TryingToCloseWhileTheCancelButtonConfirmationIsOpenMakesTheWindowCloseOnceTheRunEnds()
        {
            var (dialogsOpen, windowOpenWhileRunning, windowOpenAfter) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                window.FindControl<Button>("ActionButton")!.RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(Button.ClickEvent));
                Pump(window);
                window.Close();
                Pump(window);
                var open = (window.OwnedWindows.Count, window.IsVisible);
                FinishRun(window, RunState.Succeeded);
                return (open.Count, open.IsVisible, window.IsVisible);
            });

            Assert.Equal(1, dialogsOpen);
            Assert.True(windowOpenWhileRunning);
            Assert.False(windowOpenAfter);
        }

        [Fact]
        public async Task ClosingAFinishedWindowWhileAStaleConfirmationIsStillOpenClosesBothAtOnce()
        {
            var (dialogsAfter, windowStillOpen) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                window.Close();
                Pump(window);
                _model.Finish(RunState.Succeeded);
                _viewModel.Refresh();
                window.Close();
                Pump(window);
                return (window.OwnedWindows.Count, window.IsVisible);
            });

            Assert.Equal(0, dialogsAfter);
            Assert.False(windowStillOpen);
        }

        [Fact]
        public async Task DecliningTheConfirmationWhileTheRunContinuesKeepsTheWindowOpen()
        {
            var (windowStillOpen, cancelRequests) = await OnUiThread(() =>
            {
                var window = Show(_viewModel);
                window.Close();
                Pump(window);
                foreach (var dialog in window.OwnedWindows.ToList()) dialog.Close(false);
                Pump(window);
                var result = (window.IsVisible, _cancelRequests);
                FinishRun(window, RunState.Succeeded);
                window.Close();
                return result;
            });

            Assert.True(windowStillOpen);
            Assert.Equal(0, cancelRequests);
        }

        private (InstallerWindow Window, ProgressModel Model, InstallerViewModel ViewModel, LogBuffer Log) ShowFullInstallWindow(string? logFilePath = null)
        {
            var model = new ProgressModel(InstallSteps.All, _clock);
            var log = new LogBuffer(InstallerViewModel.LogCapacity);
            var viewModel = new InstallerViewModel(model.GetSnapshot, log, () => { }, logFilePath, CultureInfo.InvariantCulture);
            var window = Show(viewModel);
            return (window, model, viewModel, log);
        }

        private static void Relayout(InstallerViewModel viewModel, Window window)
        {
            viewModel.Refresh();
            window.UpdateLayout();
            Dispatcher.UIThread.RunJobs();
            window.UpdateLayout();
        }

        private static List<TextBlock> StepCells(ScrollViewer stepScroll, int column) =>
            [.. stepScroll.GetVisualDescendants().OfType<TextBlock>().Where(t => t.Parent is Grid && Grid.GetColumn(t) == column)];

        [Fact]
        public async Task EveryDeclaredStepIsVisibleInTheStepListAtTheDefaultWindowSize()
        {
            var (extentHeight, viewportHeight, scrollBarVisible, rows) = await OnUiThread(() =>
            {
                var (window, _, _, _) = ShowFullInstallWindow();
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                var bar = scroll.GetVisualDescendants().OfType<ScrollBar>().Single(b => b.Orientation == Avalonia.Layout.Orientation.Vertical);
                var names = StepCells(scroll, 1);
                var inside = names.Count(n => n.TranslatePoint(new Point(0, n.Bounds.Height), scroll)!.Value.Y <= scroll.Viewport.Height + 0.5);
                var result = (scroll.Extent.Height, scroll.Viewport.Height, bar.IsVisible, inside);
                return result;
            });

            Assert.Equal(InstallSteps.VisibleNames(InstallSteps.All).Count, rows);
            Assert.True(extentHeight <= viewportHeight + 0.5, $"extent {extentHeight} exceeds viewport {viewportHeight}");
            Assert.False(scrollBarVisible);
        }

        [Fact]
        public async Task TheStepListScrollsOnlyWhenItWouldCrowdOutTheLogAndItsScrollbarStaysClearOfTheElapsedColumn()
        {
            var (scrolls, scrollBarLeft, elapsedRight) = await OnUiThread(() =>
            {
                var (window, model, viewModel, _) = ShowFullInstallWindow();
                for (int i = 0; i < 40; i++) model.BeginPhase($"Extra step {i}", 10, "things");
                Relayout(viewModel, window);
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                var bar = scroll.GetVisualDescendants().OfType<ScrollBar>().Single(b => b.Orientation == Avalonia.Layout.Orientation.Vertical);
                double left = bar.TranslatePoint(new Point(0, 0), window)!.Value.X;
                double right = StepCells(scroll, 3).Max(c => c.TranslatePoint(new Point(c.Bounds.Width, 0), window)!.Value.X);
                var result = (scroll.Extent.Height > scroll.Viewport.Height && bar.IsVisible, left, right);
                return result;
            });

            Assert.True(scrolls);
            Assert.True(scrollBarLeft >= elapsedRight, $"the scrollbar starts at {scrollBarLeft}, the elapsed column ends at {elapsedRight}");
        }

        [Fact]
        public async Task TheStepColumnsLineUpBetweenTheHeaderAndEveryRowAndNoStepNameIsTrimmedAtTheDefaultWidth()
        {
            var (headerRight, rowRights, trimmedNames) = await OnUiThread(() =>
            {
                var (window, _, _, _) = ShowFullInstallWindow();
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                var header = window.GetVisualDescendants().OfType<TextBlock>().Single(t => t.Text == "Elapsed");
                double headerEdge = RightEdge(header, window);
                var edges = StepCells(scroll, 3).Select(c => RightEdge(c, window)).Distinct().ToList();
                var trimmed = StepCells(scroll, 1).Where(c => c.Bounds.Width < NaturalWidth(c) - 0.5).Select(c => c.Text).ToList();
                return (headerEdge, edges, trimmed);
            });

            Assert.Equal([headerRight], rowRights);
            Assert.Empty(trimmedNames);
        }

        [Fact]
        public async Task TheHeaderKeepsLiningUpWithTheRowsWhileTheStepListScrolls()
        {
            var (headerRight, rowRight, scrolls) = await OnUiThread(() =>
            {
                var (window, model, viewModel, _) = ShowFullInstallWindow();
                for (int i = 0; i < 40; i++) model.BeginPhase($"Extra step {i}", 10, "things");
                Relayout(viewModel, window);
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                var header = window.GetVisualDescendants().OfType<TextBlock>().Single(t => t.Text == "Elapsed");
                var result = (RightEdge(header, window), StepCells(scroll, 3).Select(c => RightEdge(c, window)).Distinct().Single(), scroll.Extent.Height > scroll.Viewport.Height);
                return result;
            });

            Assert.True(scrolls);
            Assert.Equal(rowRight, headerRight, precision: 1);
        }

        [Fact]
        public async Task TheLogTakesTheRemainingHeightAndGrowsWithTheWindow()
        {
            var (atDefault, taller) = await OnUiThread(() =>
            {
                var (window, _, viewModel, _) = ShowFullInstallWindow();
                var log = window.FindControl<ScrollViewer>("LogScroll")!;
                double defaultHeight = log.Viewport.Height;
                window.Height += 200;
                Relayout(viewModel, window);
                var result = (defaultHeight, log.Viewport.Height);
                return result;
            });

            Assert.True(atDefault >= 120, $"the log is only {atDefault}px high");
            Assert.Equal(atDefault + 200, taller, precision: 0);
        }

        private static void ShrinkToMinimumSize(InstallerWindow window, InstallerViewModel viewModel)
        {
            window.Width = window.MinWidth;
            window.Height = window.MinHeight;
            Relayout(viewModel, window);
        }

        private static double NaturalWidth(TextBlock cell)
        {
            var probe = new TextBlock { Text = cell.Text, FontFamily = cell.FontFamily, FontSize = cell.FontSize, FontWeight = cell.FontWeight };
            probe.Measure(Size.Infinity);
            return probe.DesiredSize.Width;
        }

        private static double RightEdge(Control control, Visual relativeTo) => control.TranslatePoint(new Point(control.Bounds.Width, 0), relativeTo)!.Value.X;

        private static double BottomEdge(Control control, Visual relativeTo) => control.TranslatePoint(new Point(0, control.Bounds.Height), relativeTo)!.Value.Y;

        private void RunLongStepsWithLongTimes(ProgressModel model)
        {
            foreach (var step in InstallSteps.All.Take(InstallSteps.All.Count - 1))
            {
                int phase = model.BeginPhase(step.Name, 1_234_567, step.Unit);
                _clock.Advance(TimeSpan.FromSeconds(5));
                model.Tick(phase, 100);
                _clock.Advance(TimeSpan.FromSeconds(5));
                model.Tick(phase, 100);
            }
            _clock.Advance(TimeSpan.FromSeconds(3_700));
        }

        [Fact]
        public async Task EverythingStaysInsideTheWindowAtItsMinimumSizeWithAnOverfullStepListAndLog()
        {
            var (clientHeight, buttonBottom, logHeight, listBottom, logTop) = await OnUiThread(() =>
            {
                var (window, model, viewModel, log) = ShowFullInstallWindow();
                RunLongStepsWithLongTimes(model);
                for (int i = 0; i < 40; i++) model.BeginPhase($"Extra step {i}", 10, "things");
                for (int i = 0; i < 300; i++) log.WriteLine($"line {i}", isError: false);
                model.Finish(RunState.Succeeded);
                ShrinkToMinimumSize(window, viewModel);
                var logBox = window.FindControl<ScrollViewer>("LogScroll")!.GetVisualAncestors().OfType<Border>().First();
                var listBox = window.FindControl<ScrollViewer>("StepScroll")!.GetVisualAncestors().OfType<Border>().First();
                var result = (window.ClientSize.Height, BottomEdge(window.FindControl<Button>("ActionButton")!, window), logBox.Bounds.Height,
                    BottomEdge(listBox, window), logBox.TranslatePoint(new Point(0, 0), window)!.Value.Y);
                return result;
            });

            Assert.True(buttonBottom <= clientHeight, $"the action button ends at {buttonBottom}, the window at {clientHeight}");
            Assert.True(logHeight >= 80, $"the log is only {logHeight}px high");
            Assert.True(listBottom <= logTop, $"the step list ends at {listBottom}, the log starts at {logTop}");
        }

        [Fact]
        public async Task TheElapsedColumnStaysInsideTheWindowAtItsMinimumWidthEvenWithTheLongestTimesAndCounts()
        {
            var (clientWidth, elapsedRight, scrollBarLeft, headerRight) = await OnUiThread(() =>
            {
                var (window, model, viewModel, _) = ShowFullInstallWindow();
                RunLongStepsWithLongTimes(model);
                ShrinkToMinimumSize(window, viewModel);
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                var bar = scroll.GetVisualDescendants().OfType<ScrollBar>().Single(b => b.Orientation == Avalonia.Layout.Orientation.Vertical);
                var elapsedCells = StepCells(scroll, 3).Where(c => !string.IsNullOrEmpty(c.Text)).ToList();
                var header = window.GetVisualDescendants().OfType<TextBlock>().Single(t => t.Text == "Elapsed");
                var result = (window.ClientSize.Width, elapsedCells.Max(c => RightEdge(c, window)),
                    bar.IsVisible ? bar.TranslatePoint(new Point(0, 0), window)!.Value.X : double.PositiveInfinity, RightEdge(header, window));
                return result;
            });

            Assert.True(elapsedRight <= clientWidth - 16, $"the elapsed column ends at {elapsedRight} in a window {clientWidth}px wide");
            Assert.True(headerRight <= clientWidth - 16, $"the elapsed header ends at {headerRight} in a window {clientWidth}px wide");
            Assert.True(scrollBarLeft >= elapsedRight, $"the scrollbar starts at {scrollBarLeft}, the elapsed column ends at {elapsedRight}");
        }

        [Fact]
        public async Task ErrorLinesAreFlagged()
        {
            var (errorLine, normalLine) = await OnUiThread(() =>
            {
                var (window, _, viewModel, log) = ShowFullInstallWindow();
                log.WriteLine("converted", isError: false);
                log.WriteLine("could not read a mesh", isError: true);
                Relayout(viewModel, window);
                var lines = window.FindControl<ScrollViewer>("LogScroll")!.GetVisualDescendants().OfType<TextBlock>().ToList();
                return (lines.Single(l => l.Text!.EndsWith("could not read a mesh", StringComparison.Ordinal)).Classes.Contains("error"),
                    lines.Single(l => l.Text!.EndsWith("converted", StringComparison.Ordinal)).Classes.Contains("error"));
            });

            Assert.True(errorLine);
            Assert.False(normalLine);
        }

        [Fact]
        public async Task OnlyTheOverallBarRemainsOutsideTheStepList()
        {
            var (bars, rows, steps) = await OnUiThread(() =>
            {
                var (window, _, viewModel, _) = ShowFullInstallWindow();
                Relayout(viewModel, window);
                var all = window.GetVisualDescendants().OfType<ProgressBar>().ToList();
                return (all.Count(b => !b.Classes.Contains("rowFill")), all.Count(b => b.Classes.Contains("rowFill")), viewModel.Steps.Count);
            });

            Assert.Equal(1, bars);
            Assert.Equal(steps, rows);
        }

        [Fact]
        public async Task EveryStepRowFillsItsBackgroundByItsOwnProgress()
        {
            var fills = await OnUiThread(() =>
            {
                var (window, model, viewModel, _) = ShowFullInstallWindow();
                model.EndPhase(model.BeginPhase(InstallSteps.All[2].Name, 10, null));
                int first = model.BeginPhase(InstallSteps.All[3].Name, 100, null);
                model.Tick(first, 25);
                int second = model.BeginPhase(InstallSteps.All[4].Name, 200, null);
                model.Tick(second, 150);
                model.BeginPhase(InstallSteps.All[5].Name, 0, null);
                model.Fail([InstallSteps.All[6].Name]);
                Relayout(viewModel, window);
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                return scroll.GetVisualDescendants().OfType<ProgressBar>().Skip(1).Take(4)
                    .Select(b => (b.Value, Running: b.Classes.Contains("running"), Done: b.Classes.Contains("done"), IndicatorShare: IndicatorShare(b))).ToList();
            });

            Assert.Equal(1.0, fills[0].Value);
            Assert.True(fills[0].Done);
            Assert.Equal(0.25, fills[1].Value);
            Assert.True(fills[1].Running);
            Assert.Equal(0.25, fills[1].IndicatorShare, precision: 2);
            Assert.Equal(0.75, fills[2].Value);
            Assert.Equal(0.75, fills[2].IndicatorShare, precision: 2);
            Assert.Equal((0.0, true, 0.0), (fills[3].Value, fills[3].Running, fills[3].IndicatorShare));
        }

        [Fact]
        public async Task TheStatusGlyphsKeepTheTextColourSoTheyShowOnAnyFill()
        {
            var colours = await OnUiThread(() =>
            {
                var (window, model, viewModel, _) = ShowFullInstallWindow();
                model.BeginPhase(InstallSteps.All[0].Name, 10, null);
                Relayout(viewModel, window);
                var scroll = window.FindControl<ScrollViewer>("StepScroll")!;
                return StepCells(scroll, 0).Select(c => (c.Foreground, c.Opacity)).ToList();
            });

            Assert.Equal(1.0, colours[0].Opacity);
            Assert.All(colours, c => Assert.Equal(colours[0].Foreground?.ToString(), c.Foreground?.ToString()));
        }

        private static double IndicatorShare(ProgressBar bar)
        {
            var track = bar.GetVisualDescendants().OfType<Border>().First(b => b.Name == "PART_Track");
            var indicator = bar.GetVisualDescendants().OfType<Border>().First(b => b.Name == "PART_Indicator");
            return track.Bounds.Width > 0 ? indicator.Bounds.Width / track.Bounds.Width : 0;
        }

        [Fact]
        public async Task OpenInstallFolderAndOpenLogAppearBesideCloseOnceTheRunEnded()
        {
            string logFile = Path.Combine(Path.GetTempPath(), "openfo4-window-test", "install.log");

            var (before, after) = await OnUiThread(() =>
            {
                var (window, model, viewModel, _) = ShowFullInstallWindow(logFile);
                var folder = window.FindControl<Button>("OpenInstallFolderButton")!;
                var openLog = window.FindControl<Button>("OpenLogButton")!;
                var hidden = (folder.IsVisible, openLog.IsVisible);
                model.Finish(RunState.Succeeded);
                Relayout(viewModel, window);
                var shown = (folder.IsVisible, openLog.IsVisible);
                return (hidden, shown);
            });

            Assert.Equal((false, false), before);
            Assert.Equal((true, true), after);
        }
    }
}

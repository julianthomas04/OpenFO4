using Avalonia;
using Avalonia.Data.Converters;

namespace AssetConverter.InstallerUi
{
    public static class LayoutConverters
    {
        public const double RowSpacing = 14;

        /// <summary>The space between a step row's edge and its content, so text stays clear of the row's rounded fill edge; the header keeps the same inset to stay aligned.</summary>
        public const double StepRowInset = 6;

        public static Thickness StepRowMargin { get; } = new(StepRowInset, 2);

        public static Thickness StepHeaderMargin { get; } = new(StepRowInset, 0);

        /// <summary>Every log line is exactly this tall, so the log can keep whole lines at its top edge.</summary>
        public const double LogLineHeight = 18;

        /// <summary>The blank space to leave below the last log line so that, scrolled to the end, the first visible line starts exactly at the viewport's top edge.</summary>
        public static double LogEndPaddingFor(double viewportHeight) => viewportHeight % LogLineHeight;

        /// <summary>The least height the log keeps, whatever the step list needs.</summary>
        public const double LogMinHeight = 80;

        /// <summary>
        /// The height the step list may take given the height of the area it shares with the log: what is left after the log's minimum and
        /// the spacing between the two, so the list scrolls instead of pushing the log (and the buttons below) out of the window.
        /// </summary>
        public static double StepListMaxHeightFor(double sharedAreaHeight) => Math.Max(0, sharedAreaHeight - LogMinHeight - RowSpacing);

        public static FuncValueConverter<double, double> StepListMaxHeight { get; } = new(StepListMaxHeightFor);
    }
}

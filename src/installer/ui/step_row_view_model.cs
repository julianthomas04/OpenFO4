using AssetConverter.Progress;

namespace AssetConverter.InstallerUi
{
    /// <summary>One row of the step list.</summary>
    public sealed class StepRowViewModel : ObservableObject
    {
        private StepStatus _status;
        private string _name = "";
        private string _countText = "";
        private string _elapsedText = "";
        private double _fraction;

        public string Name
        {
            get => _name;
            private set => SetProperty(ref _name, value);
        }

        public string CountText
        {
            get => _countText;
            private set => SetProperty(ref _countText, value);
        }

        public string ElapsedText
        {
            get => _elapsedText;
            private set => SetProperty(ref _elapsedText, value);
        }

        /// <summary>How much of the step is done, 0..1; the fill of the row's background bar. Zero while the total is unknown.</summary>
        public double Fraction
        {
            get => _fraction;
            private set => SetProperty(ref _fraction, value);
        }

        public StepStatus Status
        {
            get => _status;
            private set
            {
                if (_status == value) return;
                _status = value;
                OnStatusChanged();
            }
        }

        public string StatusGlyph => Status switch
        {
            StepStatus.Pending => "○",
            StepStatus.Running => "●",
            StepStatus.Done => "✓",
            StepStatus.Failed => "✕",
            StepStatus.Cancelled => "–",
            _ => throw new InvalidOperationException($"Unknown step status {Status}."),
        };

        public bool IsRunning => Status == StepStatus.Running;

        public bool IsDone => Status == StepStatus.Done;

        public bool IsFailed => Status == StepStatus.Failed;

        public bool IsCancelled => Status == StepStatus.Cancelled;

        public void Update(StepSnapshot step, ProgressFormatter format)
        {
            Name = step.Name;
            Status = step.Status;
            CountText = format.Compact(step);
            ElapsedText = format.Elapsed(step);
            Fraction = step.Fraction;
        }

        private void OnStatusChanged()
        {
            RaiseChanged(nameof(Status));
            RaiseChanged(nameof(StatusGlyph));
            RaiseChanged(nameof(IsRunning));
            RaiseChanged(nameof(IsDone));
            RaiseChanged(nameof(IsFailed));
            RaiseChanged(nameof(IsCancelled));
        }
    }
}

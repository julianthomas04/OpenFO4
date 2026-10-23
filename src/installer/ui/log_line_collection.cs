using System.Collections.ObjectModel;
using System.Collections.Specialized;
using System.ComponentModel;

namespace AssetConverter.InstallerUi
{
    public sealed record LogLineViewModel(string Text, bool IsError);

    /// <summary>A bounded list of log lines that notifies once per batch instead of once per line.</summary>
    public sealed class LogLineCollection : ObservableCollection<LogLineViewModel>
    {
        private readonly int _capacity;

        public LogLineCollection(int capacity)
        {
            ArgumentOutOfRangeException.ThrowIfLessThanOrEqual(capacity, 0);
            _capacity = capacity;
        }

        public void Append(IReadOnlyList<LogLineViewModel> lines)
        {
            if (lines.Count == 0) return;
            var kept = lines.Count > _capacity ? lines.Skip(lines.Count - _capacity).ToList() : lines.ToList();

            int overflow = Count + kept.Count - _capacity;
            if (overflow > 0) RemoveOldest(overflow);

            int start = Count;
            foreach (var line in kept) Items.Add(line);
            RaiseChanged(new NotifyCollectionChangedEventArgs(NotifyCollectionChangedAction.Add, kept, start));
        }

        private void RemoveOldest(int count)
        {
            var removed = Items.Take(count).ToList();
            ((List<LogLineViewModel>)Items).RemoveRange(0, count);
            RaiseChanged(new NotifyCollectionChangedEventArgs(NotifyCollectionChangedAction.Remove, removed, 0));
        }

        private void RaiseChanged(NotifyCollectionChangedEventArgs change)
        {
            OnPropertyChanged(new PropertyChangedEventArgs(nameof(Count)));
            OnPropertyChanged(new PropertyChangedEventArgs("Item[]"));
            OnCollectionChanged(change);
        }
    }
}

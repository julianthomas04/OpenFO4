using System.Collections.Specialized;
using AssetConverter.InstallerUi;
using Xunit;

namespace InstallerTests.Ui
{
    public class LogLineCollectionTests
    {
        private static LogLineViewModel[] Lines(int from, int count) =>
            [.. Enumerable.Range(from, count).Select(i => new LogLineViewModel($"line {i}", false))];

        private static List<NotifyCollectionChangedEventArgs> Record(LogLineCollection collection)
        {
            var changes = new List<NotifyCollectionChangedEventArgs>();
            collection.CollectionChanged += (_, e) => changes.Add(e);
            return changes;
        }

        [Fact]
        public void ABatchIsOneAddNotificationAtTheEnd()
        {
            var collection = new LogLineCollection(100);
            collection.Append(Lines(0, 3));
            var changes = Record(collection);

            collection.Append(Lines(3, 4));

            var change = Assert.Single(changes);
            Assert.Equal(NotifyCollectionChangedAction.Add, change.Action);
            Assert.Equal(3, change.NewStartingIndex);
            Assert.Equal(4, change.NewItems!.Count);
            Assert.Equal(7, collection.Count);
        }

        [Fact]
        public void OverflowDropsTheOldestLinesWithOneRemoveNotificationBeforeTheAdd()
        {
            var collection = new LogLineCollection(5);
            collection.Append(Lines(0, 4));
            var changes = Record(collection);

            collection.Append(Lines(4, 3));

            Assert.Equal([NotifyCollectionChangedAction.Remove, NotifyCollectionChangedAction.Add], changes.Select(c => c.Action));
            Assert.Equal(2, changes[0].OldItems!.Count);
            Assert.Equal(0, changes[0].OldStartingIndex);
            Assert.Equal(["line 2", "line 3", "line 4", "line 5", "line 6"], collection.Select(l => l.Text));
        }

        [Fact]
        public void ABatchLargerThanTheCapacityKeepsItsNewestLines()
        {
            var collection = new LogLineCollection(3);
            collection.Append(Lines(0, 2));

            collection.Append(Lines(10, 10));

            Assert.Equal(["line 17", "line 18", "line 19"], collection.Select(l => l.Text));
        }

        [Fact]
        public void AnEmptyBatchRaisesNothing()
        {
            var collection = new LogLineCollection(3);
            var changes = Record(collection);

            collection.Append([]);

            Assert.Empty(changes);
        }

        [Fact]
        public void ACapacityOfZeroIsRejected()
        {
            Assert.Throws<ArgumentOutOfRangeException>(() => new LogLineCollection(0));
        }
    }
}

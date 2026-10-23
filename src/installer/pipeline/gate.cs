namespace AssetConverter.Pipeline
{
    /// <summary>
    /// A one-shot signal that the lane producing something (extracted meshes, the texture index, ...) has finished that part, so the
    /// lanes reading it can start. <see cref="LaneRunner"/> faults every gate a lane is responsible for with the lane's own exception
    /// when the lane ends without opening it, so a waiting lane fails with the producer's exception instead of hanging.
    /// </summary>
    internal sealed class Gate
    {
        private readonly TaskCompletionSource _opened = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public Gate(string name)
        {
            Name = name;
        }

        public string Name { get; }

        /// <summary>Completes when the gate opens and fails with the producer's exception when it is aborted.</summary>
        public Task Opened => _opened.Task;

        public bool IsOpen => _opened.Task.IsCompletedSuccessfully;

        public void Open() => _opened.TrySetResult();

        /// <summary>Faults the gate with the exception that ended its producer; a gate that is already open keeps its state.</summary>
        public void Abort(Exception cause) => _opened.TrySetException(cause);

        /// <summary>Lets a gate stand in a list of tasks to wait for, next to the lanes a lane depends on.</summary>
        public static implicit operator Task(Gate gate) => gate.Opened;
    }
}

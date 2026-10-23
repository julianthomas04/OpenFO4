# Installer

The installer is the C# program in `src/installer/` that converts the user's
Fallout 4 install into the [install layout](../install-contract.md) the engine
reads. `install.cs` unpacks the payload and runs the pipeline, and
`InstallSession` owns one install's log, progress, window and exit code.

## Window

A setup view leads into a progress view, which offers Play once the install
finishes. The window runs on its own UI thread.

## Pipeline

The install runs as lanes in parallel (`pipeline/`). Each lane waits only on
gates, one-time "this part is done" signals, for what it reads. A lane that
fails also fails the lanes waiting on it, so nothing is left hanging.

The first real failure cancels the other lanes. Cancelling is not a failure.
The exit code is 0 when the install succeeded, 1 when it failed and 2 when it
was cancelled.

## Memory

The install holds one memory budget, 80% of the effective memory limit (the
smaller of the physical memory and any Job Object or container limit), logged
at the start. Every memory-heavy unit (the ESM import, a video, an ffdec
process, an audio entry, a NIF, a Havok clip pack, a BA2 entry payload, the
collision worker pool) reserves its estimate from it before it runs and
releases it when it ends. `pipeline/memory_estimates.cs` holds the estimates.
A unit that fits while nobody waits is granted at once; otherwise it waits and
waiters are granted first-fit in arrival order. A waiter that has waited 30
seconds stops later waiters from being granted ahead of it, so a large unit
(the ESM import, a big Havok pack) cannot starve behind a stream of small ones.
A request above the budget is clamped, so it runs alone among the other work.
A gauge of the memory actually in use backs the estimates up, since they cannot
see the installer's own baseline, the runtime's free heap, child processes or
other programs. It reads the committed memory of the Job Object when the job
limit is the effective one (summing the private bytes of the job's processes if
the system does not report that figure), and the used physical memory otherwise
(which counts other programs). A unit is granted when the reserved bytes plus
the measured bytes plus its request fit the budget. A unit that has already
materialized is counted in both terms, which is deliberately conservative.
While a unit waits only because of the measurement, the budget re-reads the
gauge every 250 ms, logs it (at most once per ten seconds), and compacts the GC
heap at most once per second, and only when the runtime holds at least 64 MB
of unused memory. With no transient reservation held, the heap is compacted when
due and the gauge read again; only a unit that still does not fit is granted
anyway, one at a time, so memory the install does not control cannot stall it.
A gauge that fails after it has worked keeps its last reading (logged once).
On a machine with enough memory nothing waits.

The collision worker pool is a long-lived reservation: it is held while the
pool waits for streamed input and does not count as held work for the progress
guarantee. Two rules keep this free of deadlocks: a flow never waits for a
reservation while holding a transient one, and a long-lived reservation never
waits for work that itself needs one and stays a small share of the budget (the
pool takes at most 25%).

## Skipping and resuming

A conversion stamp lets an output be reused only if its sources and its
converter version are unchanged. Outputs are built next to their final place
and then renamed into it, so the next run sees either the complete new output
or the previous one. Each extracted archive gets a done-marker.

## Packaging

`build-release.ps1` builds a single executable that embeds a payload of the
runtime tools. On first run the payload is unpacked once per version. A
repository checkout uses the repository's files and the system's tools instead.

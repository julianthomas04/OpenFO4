// Public API for the Papyrus .pex -> Lua transpiler.
//
// Design: single static class per the project's converter convention
// (see NifConverter). All heavy work is stateless
// so the outer Parallel.For batching is safe.

using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Serialization;
using AssetConverter.PapyrusLua.Lua;
using AssetConverter.PapyrusLua.Pex;

namespace AssetConverter.PapyrusLua;

public static class PapyrusLuaTranspiler
{
    static readonly JsonSerializerOptions DumpOptions = new()
    {
        WriteIndented = true,
        IncludeFields = false,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never,
    };

    // Phase 1 entry point: read a single .pex and dump the parsed model as
    // indented JSON to stdout. Used from install.cs when invoked as:
    //   installer.exe pex-dump <path>
    public static void DumpPexAsJson(string pexPath)
    {
        var pex = PexReader.Read(pexPath);
        string json = JsonSerializer.Serialize(pex, DumpOptions);
        Console.WriteLine(json);
    }

    // Phase 2 entry: read every .pex in a directory, report parse stats.
    // No Lua output yet.
    public static void ParseAllPex(string sourceDir, string logPath)
    {
        string[] files = Directory.GetFiles(sourceDir, "*.pex", SearchOption.AllDirectories);
        Console.WriteLine($"Parsing {files.Length} .pex files.");

        int successes = 0, failures = 0;
        var errors = new System.Collections.Concurrent.ConcurrentBag<(string path, string message)>();
        object consoleLock = new();
        int sharedIndex = 0;

        Parallel.For(0, Environment.ProcessorCount, workerId =>
        {
            while (true)
            {
                const int batch = 128;
                int start = Interlocked.Add(ref sharedIndex, batch) - batch;
                if (start >= files.Length) break;
                int end = Math.Min(start + batch, files.Length);
                int localOk = 0, localFail = 0;
                for (int i = start; i < end; i++)
                {
                    try
                    {
                        PexReader.Read(files[i]);
                        localOk++;
                    }
                    catch (Exception ex)
                    {
                        localFail++;
                        errors.Add((files[i], ex.Message));
                    }
                }
                Interlocked.Add(ref successes, localOk);
                Interlocked.Add(ref failures, localFail);
                lock (consoleLock)
                {
                    int done = successes + failures;
                    Console.WriteLine($"  {done}/{files.Length} parsed ({failures} failures so far)");
                }
            }
        });

        Console.WriteLine($"Done. {successes} succeeded, {failures} failed.");

        // Always write a summary log for triage.
        using var log = new StreamWriter(logPath, append: false);
        log.WriteLine($"PEX parse summary @ {DateTime.Now:yyyy-MM-dd HH:mm:ss}");
        log.WriteLine($"source: {sourceDir}");
        log.WriteLine($"files:  {files.Length}");
        log.WriteLine($"ok:     {successes}");
        log.WriteLine($"fail:   {failures}");
        if (failures > 0)
        {
            log.WriteLine();
            log.WriteLine("Failures:");
            foreach (var (p, m) in errors.OrderBy(e => e.path))
                log.WriteLine($"  {Path.GetFileName(p)}\t{m}");
        }
    }

    // Phase 3: emit a single .pex as skeletal Lua to stdout.
    public static void EmitLua(string pexPath)
    {
        var pex = PexReader.Read(pexPath);
        Console.Write(LuaEmitter.Emit(pex));
    }

    // Phase 3+: transpile every .pex under sourceDir to .lua under destDir,
    // mirroring the relative path. Emits a summary log alongside.
    public static void ConvertToLuaBatched(string sourceDir, string destDir, string logPath)
    {
        string[] files = Directory.GetFiles(sourceDir, "*.pex", SearchOption.AllDirectories);
        Console.WriteLine($"Transpiling {files.Length} .pex files to Lua...");
        Directory.CreateDirectory(destDir);

        int successes = 0, failures = 0, done = 0;
        var errors = new System.Collections.Concurrent.ConcurrentBag<(string path, string message)>();
        int sharedIndex = 0;
        var sw = Stopwatch.StartNew();

        Parallel.For(0, Environment.ProcessorCount, _ =>
        {
            while (true)
            {
                const int batch = 128;
                int start = Interlocked.Add(ref sharedIndex, batch) - batch;
                if (start >= files.Length) break;
                int end = Math.Min(start + batch, files.Length);
                for (int i = start; i < end; i++)
                {
                    bool ok = true;
                    try
                    {
                        var pex = PexReader.Read(files[i]);
                        string lua = LuaEmitter.Emit(pex);
                        string rel = Path.GetRelativePath(sourceDir, files[i]);
                        string outPath = Path.Combine(destDir, Path.ChangeExtension(rel, ".lua"));
                        Directory.CreateDirectory(Path.GetDirectoryName(outPath)!);
                        File.WriteAllText(outPath, lua);
                    }
                    catch (Exception ex)
                    {
                        ok = false;
                        errors.Add((files[i], ex.Message));
                    }

                    if (ok) Interlocked.Increment(ref successes);
                    else Interlocked.Increment(ref failures);

                    int newDone = Interlocked.Increment(ref done);
                    if (newDone % 100 == 0 || newDone == files.Length)
                    {
                        double pct = 100.0 * newDone / files.Length;
                        Console.WriteLine($"  {newDone}/{files.Length} ({pct:F1}%) — {sw.Elapsed:mm\\:ss} elapsed");
                    }
                }
            }
        });

        sw.Stop();
        Console.WriteLine($"Done. {successes} succeeded, {failures} failed in {sw.Elapsed:mm\\:ss}.");

        using var log = new StreamWriter(logPath, append: false);
        log.WriteLine($"PEX -> Lua transpile summary @ {DateTime.Now:yyyy-MM-dd HH:mm:ss}");
        log.WriteLine($"source:  {sourceDir}");
        log.WriteLine($"dest:    {destDir}");
        log.WriteLine($"files:   {files.Length}");
        log.WriteLine($"ok:      {successes}");
        log.WriteLine($"fail:    {failures}");
        log.WriteLine($"elapsed: {sw.Elapsed:mm\\:ss}");
        if (failures > 0)
        {
            log.WriteLine();
            log.WriteLine("Failures:");
            foreach (var (p, m) in errors.OrderBy(e => e.path))
                log.WriteLine($"  {Path.GetFileName(p)}\t{m}");
        }
    }
}

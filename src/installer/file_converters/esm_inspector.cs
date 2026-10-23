using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;

namespace AssetConverter
{
    public class EsmInspector
    {
        // Debug probe: a getter that throws is shown inline as "<err ...>" (with the real
        // exception behind the reflection wrapper) instead of aborting the dump.
        private static object? ReadProperty(PropertyInfo property, object target)
        {
            try
            {
                return property.GetValue(target);
            }
            catch (TargetInvocationException ex)
            {
                var cause = ex.InnerException ?? ex;
                return $"<err {cause.GetType().Name}: {cause.Message}>";
            }
        }

        static string Truncate(string? text, int maxLength) =>
            text == null ? string.Empty : text[..Math.Min(maxLength, text.Length)];

        public static void Inspect(string path)
        {
            Console.WriteLine($"Inspecting ESM: {path}");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(path, Fallout4Release.Fallout4);

            InspectTopLevelCells(file);
            InspectWorldspaces(file);
        }

        private static void InspectTopLevelCells(IFallout4ModDisposableGetter file)
        {
            Console.WriteLine("\n--- Inspecting Top-Level Cells (file.Cells) ---");
            var cells = file.Cells;
            Console.WriteLine($"Type: {cells.GetType().Name}");
            PrintInterfaces(cells);

            if (cells is IEnumerable collection)
            {
                int count = 0;
                foreach (var item in collection)
                {
                    count++;
                    Console.WriteLine($"Item 1 Type: {item.GetType().Name}");
                    PrintInterfaces(item);
                    break; // Only check first
                }
                Console.WriteLine($"Total Count: {cells.Count}");
            }
        }

        private static void InspectWorldspaces(IFallout4ModDisposableGetter file)
        {
            Console.WriteLine("\n--- Inspecting Worldspaces (file.Worldspaces) ---");
            var wrlds = file.Worldspaces;
            Console.WriteLine($"Count: {wrlds.Count}");

            var wrld = wrlds.FirstOrDefault();
            if (wrld == null) return;

            Console.WriteLine($"First Worldspace: {wrld.EditorID} ({wrld.FormKey})");

            var subCells = wrld.SubCells;
            if (subCells == null)
            {
                Console.WriteLine("SubCells is null.");
                return;
            }

            Console.WriteLine($"SubCells Type: {subCells.GetType().Name}");
            PrintInterfaces(subCells);

            if (subCells is IEnumerable collection)
            {
                int count = 0;
                foreach (var block in collection)
                {
                    count++;
                    Console.WriteLine($"\nBlock 1 Type: {block.GetType().Name}");
                    PrintInterfaces(block);
                    PrintProperties(block);

                    // Check Enumerable
                    if (block is IEnumerable subCollection)
                    {
                        Console.WriteLine("Block IS IEnumerable.");
                        int subCount = 0;
                        foreach (var subBlock in subCollection)
                        {
                            subCount++;
                            Console.WriteLine($"  SubBlock 1 Type: {subBlock.GetType().Name}");
                            PrintInterfaces(subBlock);
                            PrintProperties(subBlock);

                            if (subBlock is IEnumerable cellCollection)
                            {
                                Console.WriteLine("  SubBlock IS IEnumerable.");
                                int cellCount = 0;
                                foreach (var cell in cellCollection)
                                {
                                    cellCount++;
                                    Console.WriteLine($"    Cell 1 Type: {cell.GetType().Name}");
                                    break;
                                }
                            }
                            else
                            {
                                Console.WriteLine("  SubBlock IS NOT IEnumerable.");
                            }
                            break; // First subblock only
                        }
                    }
                    else
                    {
                        Console.WriteLine("Block IS NOT IEnumerable.");
                    }

                    break; // First block only
                }
            }
        }

        public static void ReflectPhase3A(string esmPath)
        {
            Console.WriteLine($"Reflecting Phase 3A (placeable) getters from: {esmPath}");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(esmPath, Fallout4Release.Fallout4);

            void Dump(string label, object record)
            {
                Console.WriteLine($"\n=== {label} ===");
                foreach (var p in record.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
                {
                    object? v = null; v = ReadProperty(p, record);
                    string? vs = v == null ? "null" : (v is IEnumerable en && !(v is string) ? $"IEnumerable({en.Cast<object>().Count()})" : v.ToString()?.Substring(0, Math.Min(80, v.ToString()?.Length ?? 0)));
                    Console.WriteLine($"  {p.Name} ({p.PropertyType.Name}) = {vs}");
                }
            }

            var act = file.Activators.FirstOrDefault();
            if (act != null) Dump("IActivatorGetter", act);

            var cont = file.Containers.FirstOrDefault();
            if (cont != null) Dump("IContainerGetter", cont);

            var door = file.Doors.FirstOrDefault();
            if (door != null) Dump("IDoorGetter", door);

            var furn = file.Furniture.FirstOrDefault();
            if (furn != null) Dump("IFurnitureGetter", furn);

            var ligh = file.Lights.FirstOrDefault();
            if (ligh != null) Dump("ILightGetter", ligh);

            var movs = file.MovableStatics.FirstOrDefault();
            if (movs != null) Dump("IMovableStaticGetter", movs);

            var talk = file.TalkingActivators.FirstOrDefault();
            if (talk != null) Dump("ITalkingActivatorGetter", talk);

            var scol = file.StaticCollections.FirstOrDefault();
            if (scol != null) Dump("IStaticCollectionGetter", scol);
        }

        public static void ReflectThinTables(string esmPath)
        {
            Console.WriteLine($"Reflecting THIN-table getters from: {esmPath}");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(esmPath, Fallout4Release.Fallout4);

            void DumpGetterProps(string label, object record)
            {
                Console.WriteLine($"\n=== {label} runtime={record.GetType().Name} ===");
                foreach (var p in record.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
                {
                    object? v = null; v = ReadProperty(p, record);
                    string? vs = v == null ? "null" : (v is IEnumerable en && !(v is string) ? $"IEnumerable({en.Cast<object>().Count()})" : v.ToString()?.Substring(0, Math.Min(60, v.ToString()?.Length ?? 0)));
                    Console.WriteLine($"  {p.Name} ({p.PropertyType.Name}) = {vs}");
                }
            }

            // Races
            var race = file.Races.FirstOrDefault();
            if (race != null) DumpGetterProps("IRaceGetter", race);

            // NPCs
            var npc = file.Npcs.FirstOrDefault();
            if (npc != null) DumpGetterProps("INpcGetter", npc);

            // Weapons
            var weap = file.Weapons.FirstOrDefault();
            if (weap != null) DumpGetterProps("IWeaponGetter", weap);

            // Worldspaces
            var ws = file.Worldspaces.FirstOrDefault();
            if (ws != null) DumpGetterProps("IWorldspaceGetter", ws);

            // Cells (find one exterior)
            ICellGetter? cell = null;
            foreach (var w in file.Worldspaces)
                foreach (var b in w.SubCells ?? Enumerable.Empty<IWorldspaceBlockGetter>())
                    foreach (var sb in b.Items ?? Enumerable.Empty<IWorldspaceSubBlockGetter>())
                    { cell = sb.Items is { Count: > 0 } subCells ? subCells[0] : null; if (cell != null) goto FoundCell; }
                FoundCell:
            if (cell != null) DumpGetterProps("ICellGetter", cell);

            // Statics
            var stat = file.Statics.FirstOrDefault();
            if (stat != null) DumpGetterProps("IStaticGetter", stat);
        }

        public static void ReflectLandscape(string esmPath)
        {
            Console.WriteLine($"Reflecting ILandscapeGetter from: {esmPath}");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(esmPath, Fallout4Release.Fallout4);

            // Find first landscape via a worldspace cell
            ILandscapeGetter? land = null;
            foreach (var ws in file.Worldspaces)
            {
                foreach (var block in ws.SubCells ?? Enumerable.Empty<IWorldspaceBlockGetter>())
                    foreach (var sub in block.Items ?? Enumerable.Empty<IWorldspaceSubBlockGetter>())
                        foreach (var cell in sub.Items ?? Enumerable.Empty<ICellGetter>())
                        {
                            land = cell.Landscape;
                            if (land != null) goto Found;
                        }
            }
        Found:
            if (land == null) { Console.WriteLine("No landscape found."); return; }

            Console.WriteLine($"\nRuntime type: {land.GetType().FullName}");
            Console.WriteLine("\n=== ILandscapeGetter interface properties ===");
            foreach (var iface in land.GetType().GetInterfaces()
                .Where(i => i.Name.Contains("Landscape") || i.Name.Contains("Getter")))
            {
                foreach (var p in iface.GetProperties())
                    Console.WriteLine($"  [{iface.Name}] {p.Name} : {p.PropertyType.FullName ?? p.PropertyType.Name}");
            }

            Console.WriteLine("\n=== Concrete type properties ===");
            foreach (var p in land.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
            {
                object? val = null;
                val = ReadProperty(p, land);
                string valStr = val == null ? "null" : (val is IEnumerable en && !(val is string)
                    ? $"IEnumerable count={en.Cast<object>().Count()}"
                    : val.GetType().Name + " = " + Truncate(val.ToString(), 80));
                Console.WriteLine($"  {p.Name} : {p.PropertyType.Name}  => {valStr}");
            }

            // Specifically probe AlphaLayers / Textures / Layers
            foreach (var name in new[] { "AlphaLayers", "Textures", "Layers", "VertexColors", "VertexNormals" })
            {
                var prop = land.GetType().GetProperty(name, BindingFlags.Public | BindingFlags.Instance);
                if (prop == null) { Console.WriteLine($"\n  {name}: NOT FOUND"); continue; }
                try
                {
                    var val = prop.GetValue(land);
                    Console.WriteLine($"\n  {name} ({prop.PropertyType.Name}): {(val == null ? "null" : val.ToString())}");
                    if (val is IEnumerable coll)
                    {
                        int i = 0;
                        foreach (var item in coll)
                        {
                            Console.WriteLine($"    [{i}] type={item.GetType().Name}");
                            foreach (var sp in item.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
                            {
                                object? sv = null;
                                sv = ReadProperty(sp, item);
                                Console.WriteLine($"      .{sp.Name} ({sp.PropertyType.Name}) = {sv}");
                                // Go one level deeper for Header
                                if (sp.Name == "Header" && sv != null)
                                {
                                    foreach (var hp in sv.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
                                    {
                                        object? hv = null;
                                        hv = ReadProperty(hp, sv);
                                        Console.WriteLine($"        .{hp.Name} ({hp.PropertyType.Name}) = {hv}");
                                    }
                                }
                            }
                            if (++i >= 3) break;
                        }
                    }
                }
                catch (Exception ex) { Console.WriteLine($"\n  {name}: EXCEPTION {ex.Message}"); }
            }
        }

        private static void PrintInterfaces(object obj)
        {
            var ifaces = obj.GetType().GetInterfaces();
            Console.WriteLine("Interfaces:");
            foreach (var i in ifaces)
            {
                if (i.Name.Contains("Enumerable") || i.Name.Contains("List") || i.Name.Contains("Getter"))
                    Console.WriteLine($" - {i.Name}");
            }
        }

        private static void PrintProperties(object obj)
        {
            Console.WriteLine("Properties:");
            foreach (var p in obj.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
            {
                Console.WriteLine($" - {p.Name} ({p.PropertyType.Name})");
            }
        }
    }
}

using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;

namespace AssetConverter
{
    /// <summary>A navigation_meshes row: the encoded mesh plus the owning cell the cell walk found.</summary>
    internal sealed record NavMeshRow(
        EncodedNavMesh Mesh,
        string Cell,
        string? Worldspace,
        int? GridX,
        int? GridY);

    /// <summary>A cell reached by the cell walk and the worldspace it was found under (null for interior cells).</summary>
    internal readonly record struct NavMeshCellScope(ICellGetter Cell, FormKey? Worldspace);

    /// <summary>More than the tolerated share of meshes failed validation, or none survived: the extraction itself is broken.</summary>
    internal sealed class NavMeshExtractionException(string message) : Exception(message);

    /// <summary>
    /// NAVM records live in their cell's temporary group, which Mutagen exposes as Cell.NavigationMeshes. Each mesh is
    /// encoded and checked against its owning cell here; a mesh that fails is reported with its form key and left out,
    /// so neither its row nor its records row is written. Isolated failures are tolerated; a failure rate above
    /// <see cref="DefaultMaxSkippedFraction"/> by default means the extraction is systemically wrong and fails the install.
    /// </summary>
    internal sealed class PreparedNavMeshes
    {
        public const double DefaultMaxSkippedFraction = 0.01;
        const int DeletedRecordFlag = 0x20;

        readonly TextWriter _errors;
        readonly double _maxSkippedFraction;

        public List<(INavigationMeshGetter Record, NavMeshRow Row)> Entries { get; } = [];
        public int Skipped { get; private set; }

        PreparedNavMeshes(TextWriter errors, double maxSkippedFraction)
        {
            _errors = errors;
            _maxSkippedFraction = maxSkippedFraction;
        }

        public static PreparedNavMeshes From(
            IEnumerable<NavMeshCellScope> cells, TextWriter errors, double maxSkippedFraction = DefaultMaxSkippedFraction)
        {
            var prepared = new PreparedNavMeshes(errors, maxSkippedFraction);
            foreach (var (cell, worldspace) in cells)
                foreach (var mesh in cell.NavigationMeshes)
                    prepared.Add(cell, worldspace, mesh);
            prepared.ThrowIfSystemic();
            return prepared;
        }

        void Add(ICellGetter cell, FormKey? worldspace, INavigationMeshGetter mesh)
        {
            // Deleted records carry no navmesh data.
            if ((mesh.MajorRecordFlagsRaw & DeletedRecordFlag) != 0) return;
            try
            {
                Entries.Add((mesh, ToRow(cell, worldspace, NavMeshEncoder.Encode(mesh))));
            }
            catch (NavMeshValidationException ex)
            {
                Skipped++;
                _errors.WriteLine($"  NAVM ERROR {mesh.FormKey} in cell {cell.FormKey}: {ex.Message}; mesh skipped");
            }
        }

        void ThrowIfSystemic()
        {
            int total = Entries.Count + Skipped;
            if (Skipped > total * _maxSkippedFraction)
                throw new NavMeshExtractionException(
                    $"{Skipped} of {total} navigation meshes failed validation (more than {_maxSkippedFraction:P0}); see the NAVM ERROR lines");
        }

        static NavMeshRow ToRow(ICellGetter cell, FormKey? owningWorldspace, EncodedNavMesh mesh)
        {
            string cellKey = cell.FormKey.ToString();
            switch (mesh.Parent)
            {
                case NavMeshParent.Exterior exterior:
                    if (cell.Grid is not { } grid)
                        throw new NavMeshValidationException("exterior mesh sits in a cell without a grid position");
                    if (grid.Point.X != exterior.GridX || grid.Point.Y != exterior.GridY)
                        throw new NavMeshValidationException(
                            $"mesh grid ({exterior.GridX}, {exterior.GridY}) differs from its cell's grid ({grid.Point.X}, {grid.Point.Y})");
                    if (exterior.Worldspace.IsNull || exterior.Worldspace != owningWorldspace)
                        throw new NavMeshValidationException(
                            $"mesh worldspace {(exterior.Worldspace.IsNull ? "(none)" : exterior.Worldspace.ToString())} differs from its cell's worldspace {owningWorldspace?.ToString() ?? "(none)"}");
                    return new NavMeshRow(mesh, cellKey, exterior.Worldspace.ToString(), grid.Point.X, grid.Point.Y);
                case NavMeshParent.Interior interior:
                    if (interior.Cell != cell.FormKey)
                        throw new NavMeshValidationException($"mesh parent cell {interior.Cell} differs from its owning cell");
                    return new NavMeshRow(mesh, cellKey, null, null, null);
                default:
                    throw new NavMeshValidationException($"unknown parent {mesh.Parent}");
            }
        }
    }
}

using AssetConverter;
using AssetConverter.Pipeline;
using Mutagen.Bethesda.Plugins;

namespace InstallerTests
{
    /// <summary>
    /// The NAVMs of the fixture plugin: two valid exterior meshes (one linking to the other and to a door), a valid
    /// interior mesh, and two exterior meshes the importer must reject.
    /// </summary>
    internal static class NavMeshFixture
    {
        public const string Neighbour = "NeighbourNav";
        public const string Linked = "LinkedNav";
        public const string Interior = "InteriorNav";
        public const string BadVertexIndex = "BadVertexIndexNav";
        public const string WrongGrid = "WrongGridNav";

        public const string DanglingLink = "DanglingLinkNav";
        public const uint DanglingTargetId = 0xABCDE;
        public const int ImportedCount = 4;

        // The two deliberately invalid meshes are 40% of the fixture, far above the production tolerance.
        public const double SkipToleranceForTheRejectedMeshes = 0.5;

        public const int InteriorRecordFlags = unchecked((int)0x84000000);

        public static ESMLoader Loader(string databasePath) =>
            new(databasePath) { Memory = MemoryBudget.Unlimited(), NavMeshMaxSkippedFraction = SkipToleranceForTheRejectedMeshes };

        public static void Describe(TinyPlugin plugin)
        {
            var neighbour = plugin.AddExteriorNavMesh(Neighbour)
                .Vertex(4100f, -8000f, 1f).Vertex(4150f, -8000f, 2f).Vertex(4100f, -7950f, 3f)
                .Triangle(0, 1, 2, -1, -1, -1, NavMeshBuilder.FoundFlag);

            plugin.AddExteriorNavMesh(Linked)
                .Vertex(4096f, -8192f, 10f).Vertex(4200f, -8192f, 12f).Vertex(4096f, -8100f, 14f).Vertex(4200f, -8100f, 16.5f)
                .Triangle(0, 1, 2, -1, 1, 0, NavMeshBuilder.FoundFlag | NavMeshBuilder.PreferredFlag | NavMeshBuilder.ExternalEdge2)
                .Triangle(1, 3, 2, 1, -1, 0, NavMeshBuilder.FoundFlag | NavMeshBuilder.DoorFlag | NavMeshBuilder.ExternalEdge0)
                .EdgeLink(2, neighbour.Mesh.FormKey, 5, 1)
                .EdgeLink(0, neighbour.Mesh.FormKey, 0, 2)
                .DoorLink(1, plugin.FormKeyOf("ExteriorDoor"));

            plugin.AddInteriorNavMesh(Interior)
                .RecordFlags(InteriorRecordFlags)
                .Vertex(-100f, -200f, 50f).Vertex(100f, -200f, 50f).Vertex(0f, 300f, 60f)
                .Triangle(0, 1, 2, -1, -1, -1, NavMeshBuilder.FoundFlag | NavMeshBuilder.DoorFlag)
                .DoorLink(0, plugin.FormKeyOf("InteriorDoor"));

            plugin.AddInteriorNavMesh(DanglingLink)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f)
                .Triangle(0, 1, 2, -1, -1, 0, NavMeshBuilder.FoundFlag | NavMeshBuilder.ExternalEdge2)
                .EdgeLink(0, new FormKey(neighbour.Mesh.FormKey.ModKey, DanglingTargetId), 0, 0);

            plugin.AddExteriorNavMesh(BadVertexIndex)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f)
                .Triangle(0, 1, 7, -1, -1, -1, NavMeshBuilder.FoundFlag);

            plugin.AddExteriorNavMesh(WrongGrid, gridX: 5, gridY: 5)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f)
                .Triangle(0, 1, 2, -1, -1, -1, NavMeshBuilder.FoundFlag);
        }
    }
}

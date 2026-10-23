using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace InstallerTests
{
    /// <summary>Builds NAVM records in memory. The mesh grid is passed as the game numbers it; the stored pair order that Mutagen reads swapped is handled here.</summary>
    internal sealed class NavMeshBuilder
    {
        public const ushort FoundFlag = 0x0800;
        public const ushort PreferredFlag = 0x0040;
        public const ushort DoorFlag = 0x0400;
        public const ushort ExternalEdge0 = 0x1;
        public const ushort ExternalEdge1 = 0x2;
        public const ushort ExternalEdge2 = 0x4;
        public const int AutogenRecordFlag = 0x04000000;

        readonly NavigationMesh _mesh;

        NavMeshBuilder(NavigationMesh mesh) => _mesh = mesh;

        public static NavMeshBuilder Exterior(IFallout4Mod mod, FormKey worldspace, int gridX, int gridY) =>
            new(new NavigationMesh(mod)
            {
                NavmeshGeometry = new NavmeshGeometry
                {
                    NavmeshVersion = 15,
                    Parent = new WorldspaceNavmeshParent
                    {
                        Parent = new FormLink<IWorldspaceGetter>(worldspace),
                        Coordinates = new P2Int16((short)gridY, (short)gridX),
                    },
                },
            });

        public static NavMeshBuilder Interior(IFallout4Mod mod, FormKey cell) =>
            new(new NavigationMesh(mod)
            {
                NavmeshGeometry = new NavmeshGeometry
                {
                    NavmeshVersion = 15,
                    Parent = new CellNavmeshParent { Parent = new FormLink<ICellGetter>(cell) },
                },
            });

        public NavigationMesh Mesh => _mesh;

        NavmeshGeometry Geometry => _mesh.NavmeshGeometry!;

        public NavMeshBuilder WithoutGeometry()
        {
            _mesh.NavmeshGeometry = null;
            return this;
        }

        public NavMeshBuilder RecordFlags(int flags)
        {
            _mesh.MajorRecordFlagsRaw = flags;
            return this;
        }

        public NavMeshBuilder Vertex(float x, float y, float z)
        {
            Geometry.Vertices.Add(new P3Float(x, y, z));
            return this;
        }

        public NavMeshBuilder Triangle(short v0, short v1, short v2, short n0, short n1, short n2, ushort flags)
        {
            Geometry.Triangles.Add(new NavmeshTriangle
            {
                Vertices = new P3Int16(v0, v1, v2),
                EdgeLink_0_1 = n0,
                EdgeLink_1_2 = n1,
                EdgeLink_2_0 = n2,
                Height = float.MaxValue,
                Flags = (NavmeshTriangle.Flag)flags,
            });
            return this;
        }

        public NavMeshBuilder EdgeLink(int type, FormKey? target, short targetTriangle, sbyte targetEdge)
        {
            var link = new EdgeLink { Unknown = type, TriangleIndex = targetTriangle, Unknown2 = targetEdge };
            if (target is { } key) link.Mesh.FormKey = key;
            Geometry.EdgeLinks.Add(link);
            return this;
        }

        public NavMeshBuilder DoorLink(short triangle, FormKey? door)
        {
            var link = new DoorTriangle { TriangleBeforeDoor = triangle, Unknown = unchecked((int)0xE48B73F3) };
            if (door is { } key) link.Door.FormKey = key;
            Geometry.DoorTriangles.Add(link);
            return this;
        }
    }
}

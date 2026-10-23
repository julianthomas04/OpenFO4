using System.Buffers.Binary;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace AssetConverter
{
    internal sealed class NavMeshValidationException(string message) : Exception(message);

    /// <summary>Where the mesh declares it belongs: an exterior grid cell of a worldspace, or an interior cell.</summary>
    internal abstract record NavMeshParent
    {
        /// <summary>Grid coordinates are already corrected for Mutagen's swapped reading of the stored pair.</summary>
        internal sealed record Exterior(FormKey Worldspace, int GridX, int GridY) : NavMeshParent;

        internal sealed record Interior(FormKey Cell) : NavMeshParent;
    }

    /// <summary>One NAVM ready for the navigation_meshes row.</summary>
    internal sealed record EncodedNavMesh(
        uint FormId,
        uint RecordFlags,
        uint FormatVersion,
        NavMeshParent Parent,
        int VertexCount,
        int TriangleCount,
        int EdgeLinkCount,
        int DoorLinkCount,
        byte[] Vertices,
        byte[] Triangles,
        byte[] EdgeLinks,
        byte[] DoorLinks);

    internal static class NavMeshEncoder
    {
        public const int VertexStride = 12;
        public const int TriangleStride = 14;
        public const int EdgeLinkStride = 8;
        public const int DoorLinkStride = 6;

        public static EncodedNavMesh Encode(INavigationMeshGetter mesh)
        {
            var geometry = mesh.NavmeshGeometry
                ?? throw new NavMeshValidationException("record has no navmesh geometry");
            var vertices = geometry.Vertices;
            var triangles = geometry.Triangles;
            var edgeLinks = geometry.EdgeLinks;
            var doorLinks = geometry.DoorTriangles;
            var plugin = mesh.FormKey.ModKey;

            return new EncodedNavMesh(
                mesh.FormKey.ID,
                unchecked((uint)mesh.MajorRecordFlagsRaw),
                geometry.NavmeshVersion,
                ReadParent(geometry.Parent),
                vertices.Count,
                triangles.Count,
                edgeLinks.Count,
                doorLinks.Count,
                EncodeVertices(vertices),
                EncodeTriangles(triangles, vertices.Count, edgeLinks.Count),
                EncodeEdgeLinks(edgeLinks, plugin),
                EncodeDoorLinks(doorLinks, triangles.Count, plugin));
        }

        static NavMeshParent ReadParent(IANavmeshParentGetter parent) => parent switch
        {
            // Mutagen reads the stored (grid Y, grid X) pair in the opposite order: Coordinates.X holds grid Y.
            IWorldspaceNavmeshParentGetter exterior => new NavMeshParent.Exterior(
                exterior.Parent.FormKey, exterior.Coordinates.Y, exterior.Coordinates.X),
            ICellNavmeshParentGetter interior => new NavMeshParent.Interior(interior.Parent.FormKey),
            _ => throw new NavMeshValidationException($"unknown parent kind {parent.GetType().Name}"),
        };

        static byte[] EncodeVertices(IReadOnlyList<P3Float> vertices)
        {
            var bytes = new byte[vertices.Count * VertexStride];
            for (int i = 0; i < vertices.Count; i++)
            {
                var span = bytes.AsSpan(i * VertexStride);
                BinaryPrimitives.WriteSingleLittleEndian(span, vertices[i].X);
                BinaryPrimitives.WriteSingleLittleEndian(span[4..], vertices[i].Y);
                BinaryPrimitives.WriteSingleLittleEndian(span[8..], vertices[i].Z);
            }
            return bytes;
        }

        static byte[] EncodeTriangles(IReadOnlyList<INavmeshTriangleGetter> triangles, int vertexCount, int edgeLinkCount)
        {
            for (int i = 0; i < triangles.Count; i++)
                ValidateTriangle(i, triangles[i], vertexCount, edgeLinkCount, triangles.Count);

            var bytes = new byte[triangles.Count * TriangleStride];
            for (int i = 0; i < triangles.Count; i++)
            {
                var triangle = triangles[i];
                var span = bytes.AsSpan(i * TriangleStride);
                for (int corner = 0; corner < 3; corner++)
                    BinaryPrimitives.WriteUInt16LittleEndian(span[(corner * 2)..], (ushort)Corner(triangle, corner));
                for (int edge = 0; edge < 3; edge++)
                    BinaryPrimitives.WriteInt16LittleEndian(span[(6 + edge * 2)..], Neighbour(triangle, edge));
                BinaryPrimitives.WriteUInt16LittleEndian(span[12..], Flags(triangle));
            }
            return bytes;
        }

        static void ValidateTriangle(int index, INavmeshTriangleGetter triangle, int vertexCount, int edgeLinkCount, int triangleCount)
        {
            for (int corner = 0; corner < 3; corner++)
            {
                short vertex = Corner(triangle, corner);
                if (vertex < 0 || vertex >= vertexCount)
                    throw new NavMeshValidationException(
                        $"triangle {index} vertex {corner} index {vertex} outside {vertexCount} vertices");
            }

            ushort flags = Flags(triangle);
            for (int edge = 0; edge < 3; edge++)
            {
                bool external = (flags & (1 << edge)) != 0; // flag bit e set: neighbour e indexes the mesh's edge links
                int limit = external ? edgeLinkCount : triangleCount;
                int lowest = external ? 0 : -1;
                short neighbour = Neighbour(triangle, edge);
                if (neighbour < lowest || neighbour >= limit)
                    throw new NavMeshValidationException(
                        $"triangle {index} edge {edge} {(external ? "edge-link" : "neighbour")} index {neighbour} outside {limit}");
            }
        }

        static short Corner(INavmeshTriangleGetter triangle, int corner) => corner switch
        {
            0 => triangle.Vertices.X,
            1 => triangle.Vertices.Y,
            _ => triangle.Vertices.Z,
        };

        static short Neighbour(INavmeshTriangleGetter triangle, int edge) => edge switch
        {
            0 => triangle.EdgeLink_0_1,
            1 => triangle.EdgeLink_1_2,
            _ => triangle.EdgeLink_2_0,
        };

        static ushort Flags(INavmeshTriangleGetter triangle) => (ushort)(int)triangle.Flags;

        static byte[] EncodeEdgeLinks(IReadOnlyList<IEdgeLinkGetter> links, ModKey plugin)
        {
            var bytes = new byte[links.Count * EdgeLinkStride];
            for (int i = 0; i < links.Count; i++)
            {
                var link = links[i];
                if (link.Unknown is < 0 or > byte.MaxValue)
                    throw new NavMeshValidationException($"edge link {i} type {link.Unknown} outside 0..255");
                if (link.Unknown2 < 0)
                    throw new NavMeshValidationException($"edge link {i} target edge {link.Unknown2} is negative");
                if (link.TriangleIndex < 0)
                    throw new NavMeshValidationException($"edge link {i} target triangle {link.TriangleIndex} is negative");

                var span = bytes.AsSpan(i * EdgeLinkStride);
                span[0] = (byte)link.Unknown;
                span[1] = (byte)link.Unknown2;
                BinaryPrimitives.WriteUInt16LittleEndian(span[2..], (ushort)link.TriangleIndex);
                BinaryPrimitives.WriteUInt32LittleEndian(span[4..], LinkTargetId(link.Mesh, plugin, $"edge link {i} target"));
            }
            return bytes;
        }

        static byte[] EncodeDoorLinks(IReadOnlyList<IDoorTriangleGetter> links, int triangleCount, ModKey plugin)
        {
            var bytes = new byte[links.Count * DoorLinkStride];
            for (int i = 0; i < links.Count; i++)
            {
                var link = links[i];
                if (link.TriangleBeforeDoor < 0 || link.TriangleBeforeDoor >= triangleCount)
                    throw new NavMeshValidationException($"door link {i} triangle {link.TriangleBeforeDoor} outside {triangleCount} triangles");

                var span = bytes.AsSpan(i * DoorLinkStride);
                BinaryPrimitives.WriteUInt16LittleEndian(span, (ushort)link.TriangleBeforeDoor);
                BinaryPrimitives.WriteUInt32LittleEndian(span[2..], LinkTargetId(link.Door, plugin, $"door link {i} reference"));
            }
            return bytes;
        }

        // Links carry only the 24-bit id, so a target outside the mesh's own plugin could not be told apart from a local one.
        static uint LinkTargetId(IFormLinkGetter target, ModKey plugin, string what)
        {
            if (target.IsNull) return 0;
            var key = target.FormKey;
            if (key.ModKey != plugin)
                throw new NavMeshValidationException($"{what} {key} belongs to another plugin than {plugin}");
            return key.ID;
        }
    }
}

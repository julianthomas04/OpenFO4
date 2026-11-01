using System.Diagnostics.CodeAnalysis;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    internal readonly record struct PlacedCounts(long Placed, long Teleports)
    {
        public static PlacedCounts operator +(PlacedCounts left, PlacedCounts right) =>
            new(left.Placed + right.Placed, left.Teleports + right.Teleports);
    }

    /// <summary>
    /// Which placed references of a cell become placed_objects rows and which of those carry a teleport row. The load and the
    /// progress discovery both go through here, so the counted units cannot drift from the ticked ones.
    /// </summary>
    internal static class PlacedReferences
    {
        private const int DeletedRecordFlag = 0x20;

        /// <summary>
        /// False for deleted refs (record flag 0x20: they never spawn in-game; vanilla Fallout4.esm contains none, DLC/mod plugins
        /// can) and for refs without a base object.
        /// </summary>
        public static bool TryGetBase(IPlacedGetter placed, out string baseObject)
        {
            baseObject = "";
            if ((placed.MajorRecordFlagsRaw & DeletedRecordFlag) != 0) return false;

            if (placed is IPlacedObjectGetter placedObject && !placedObject.Base.IsNull) baseObject = placedObject.Base.FormKey.ToString();
            else if (placed is IPlacedNpcGetter placedNpc && !placedNpc.Base.IsNull) baseObject = placedNpc.Base.FormKey.ToString();
            return baseObject.Length > 0;
        }

        /// <summary>
        /// XTEL: load-door teleport. Only IPlacedObjectGetter carries it (ACHR/PlacedNpc cannot). The filter is "has a non-null
        /// destination door link", not base == DOOR, so modded teleport activators are kept. Resolution of the destination cell is
        /// left to the runtime (via destination_ref -> placed_objects.cell_form_key).
        /// </summary>
        public static bool TryGetTeleport(IPlacedGetter placed, [NotNullWhen(true)] out ITeleportDestinationGetter? destination)
        {
            destination = null;
            if (placed is IPlacedObjectGetter { TeleportDestination: { } candidate } && !candidate.Door.IsNull)
                destination = candidate;
            return destination != null;
        }

        public static PlacedCounts Count(IEnumerable<IPlacedGetter> placedReferences)
        {
            long placed = 0, teleports = 0;
            foreach (var reference in placedReferences)
            {
                if (!TryGetBase(reference, out _)) continue;
                placed++;
                if (TryGetTeleport(reference, out _)) teleports++;
            }
            return new PlacedCounts(placed, teleports);
        }

        /// <summary>The refs of both of the cell's groups, which is what the load flattens.</summary>
        public static PlacedCounts Count(ICellGetter cell) =>
            (cell.Persistent is { } persistent ? Count(persistent) : default)
            + (cell.Temporary is { } temporary ? Count(temporary) : default);
    }
}

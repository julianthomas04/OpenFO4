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
        /// <summary>
        /// False for deleted refs (record flag 0x20: they never spawn in-game; vanilla Fallout4.esm contains none, DLC/mod plugins
        /// can) and for refs without a base object.
        /// </summary>
        public static bool TryGetBase(IPlacedGetter placed, out string baseObject)
        {
            baseObject = "";
            if (!IsKept(placed)) return false;

            baseObject = placed switch
            {
                IPlacedObjectGetter placedObject => placedObject.Base.FormKey.ToString(),
                IPlacedNpcGetter placedNpc => placedNpc.Base.FormKey.ToString(),
                _ => throw new InvalidOperationException($"Placed reference {placed.FormKey} of type {placed.GetType().Name} has no base."),
            };
            return true;
        }

        /// <summary>The filter of <see cref="TryGetBase"/> without building the base object's key.</summary>
        public static bool IsKept(IPlacedGetter placed) =>
            (placed.MajorRecordFlagsRaw & RecordFlags.Deleted) == 0 && placed switch
            {
                IPlacedObjectGetter placedObject => !placedObject.Base.IsNull,
                IPlacedNpcGetter placedNpc => !placedNpc.Base.IsNull,
                _ => false,
            };

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
                if (!IsKept(reference)) continue;
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

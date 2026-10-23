using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;

namespace AssetConverter
{
    /// <summary>
    /// The rank chains of the PERK records. <see cref="FirstRankOf"/> maps every perk record to the first rank of its chain,
    /// so no consumer has to detect chain heads itself.
    /// </summary>
    internal sealed record PerkRankChains(IReadOnlyDictionary<FormKey, FormKey> FirstRankOf, IReadOnlyList<string> Warnings)
    {
        /// <summary>
        /// Rank records link to the next rank through <c>NextPerk</c>. A chain is either open (it ends at a rank with no
        /// next) or closed (the last rank links back to the first). The head of an open chain is the rank nothing links to,
        /// the head of a closed one the rank with the lowest required level, ties going to the lowest form id. A rank
        /// that two records link to would merge two chains, which is reported.
        /// </summary>
        public static PerkRankChains Build(IReadOnlyCollection<IPerkGetter> perks)
        {
            var known = perks.Select(perk => perk.FormKey).ToHashSet();
            var links = perks
                .Where(perk => !perk.NextPerk.IsNull && known.Contains(perk.NextPerk.FormKey))
                .ToDictionary(perk => perk.FormKey, perk => perk.NextPerk.FormKey);
            var linkedBy = links.ToLookup(link => link.Value, link => link.Key);

            var warnings = linkedBy
                .Where(group => group.Count() > 1)
                .Select(group => $"Perk {group.Key} is the next rank of {group.Count()} records ({string.Join(", ", group.OrderBy(key => key.ID))}); their rank chains are treated as one.")
                .ToList();

            var component = new Dictionary<FormKey, FormKey>();
            FormKey Root(FormKey key)
            {
                while (component.TryGetValue(key, out var parent) && parent != key) key = parent;
                return key;
            }

            foreach (var perk in perks)
                component.TryAdd(perk.FormKey, perk.FormKey);
            foreach (var (from, to) in links)
                component[Root(from)] = Root(to);

            var firstRankOf = new Dictionary<FormKey, FormKey>();
            foreach (var chain in perks.GroupBy(perk => Root(perk.FormKey)))
            {
                var unlinked = chain.Where(perk => !linkedBy.Contains(perk.FormKey)).ToList();
                var candidates = unlinked.Count > 0 ? unlinked : chain.ToList();
                var head = candidates.OrderBy(perk => perk.Level).ThenBy(perk => perk.FormKey.ID).First().FormKey;
                foreach (var perk in chain)
                    firstRankOf[perk.FormKey] = head;
            }
            return new PerkRankChains(firstRankOf, warnings);
        }
    }
}

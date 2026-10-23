using System.Text.Json.Nodes;

namespace AssetConverter
{
    internal sealed record ParticleSystemResult(JsonObject Extras, IReadOnlyList<string> SkippedModifiers);
}

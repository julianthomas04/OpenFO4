using System.Text.Json.Nodes;

namespace AssetConverter
{
    // Node extras are written by several independent passes; each pass adds its own keys to the
    // object already on the node instead of replacing it.
    internal static class GltfNodeExtras
    {
        public static JsonObject Existing(object? current) => current switch
        {
            null => new JsonObject(),
            JsonObject extras => extras,
            _ => throw new InvalidOperationException($"node extras are a {current.GetType().Name}, expected a JSON object"),
        };
    }
}

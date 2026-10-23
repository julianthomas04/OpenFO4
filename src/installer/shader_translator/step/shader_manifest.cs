using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;

namespace AssetConverter.ShaderTranslation.Installation
{
    public enum RegisterKind { ConstantBuffer, Resource, Sampler, UnorderedAccess }

    public sealed record ManifestBinding(
        RegisterKind Kind,
        int Register,
        string Name,
        int Group,
        int Binding,
        bool External,
        TextureOverride TextureOverride,
        string? UavFormat);

    public sealed record ManifestShader(
        string Name,
        int Blob,
        ShaderStage Stage,
        NanPolicy NanPolicy,
        IReadOnlyList<ManifestBinding> Bindings);

    /// <summary>
    /// A family of shaders that is expanded from the shader archive: every blob of FXP group <see cref="FxpGroup"/> becomes one shader named
    /// by <see cref="NameTemplate"/> (placeholders <c>{stage}</c> = vs|ps and <c>{descriptor:x8}</c>). The header counts are asserted so another game build fails loudly.
    /// </summary>
    public sealed record ManifestFamily(
        string Family,
        int FxpGroup,
        string NameTemplate,
        int VertexCount,
        int PixelCount,
        NanPolicy NanPolicy,
        IReadOnlyList<ManifestBinding> VertexBindings,
        IReadOnlyList<ManifestBinding> PixelBindings)
    {
        public static readonly string[] RequiredPlaceholders = ["{stage}", "{descriptor:x8}"];

        public static string ExpandName(string template, string stage, uint descriptor) =>
            template.Replace("{stage}", stage, StringComparison.Ordinal)
                .Replace("{descriptor:x8}", descriptor.ToString("x8", System.Globalization.CultureInfo.InvariantCulture), StringComparison.Ordinal);
    }

    public sealed record ShaderManifest(IReadOnlyList<ManifestShader> Shaders, IReadOnlyList<ManifestFamily>? Families = null)
    {
        public IReadOnlyList<ManifestFamily> Families { get; init; } = Families ?? [];
    }

    public sealed class ShaderManifestException : ShaderTranslationException
    {
        public ShaderManifestException(string message) : base(message) { }
        public ShaderManifestException(string message, Exception inner) : base(message, inner) { }
    }

    public sealed class ShaderTranslateStepException : ShaderTranslationException
    {
        public ShaderTranslateStepException(string message) : base(message) { }
        public ShaderTranslateStepException(string message, Exception inner) : base(message, inner) { }
    }
}

using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    public enum NanPolicy
    {
        /// <summary>Plain WGSL min/max/clamp; behaviour for NaN operands is unspecified (WGSL does not guarantee NaN handling).</summary>
        Unspecified,
        /// <summary>D3D semantics (a NaN operand yields the other min/max operand, saturate yields 0) built from x != x tests; larger code.</summary>
        D3dStrict,
    }

    public sealed record WgslEmitOptions(NanPolicy Nan = NanPolicy.Unspecified, string? ProvenanceComment = null);

    /// <summary>One signature element living in an I/O register: <see cref="Mask"/> is the part of the register this element occupies.</summary>
    public sealed record IoElement(string SemanticName, uint SemanticIndex, byte Mask);

    public sealed record IoRegister(string Field, uint Register, byte Mask, string Semantic, uint SystemValue, InterpolationMode Interpolation, IReadOnlyList<IoElement>? Elements = null)
    {
        public IReadOnlyList<IoElement> Elements { get; init; } = Elements ?? [];
    }

    public sealed record EmittedBinding(string Kind, int Register, string Name, int Group, int Binding, string WgslType, bool Declared);

    /// <summary>Result of translating one blob. <see cref="Wgsl"/> is a module fragment (no entry points).</summary>
    public sealed record EmittedShader(
        string Name,
        ShaderStage Stage,
        string Wgsl,
        string InputStruct,
        string OutputStruct,
        IReadOnlyList<IoRegister> Inputs,
        IReadOnlyList<IoRegister> Outputs,
        IReadOnlyList<EmittedBinding> Bindings,
        (uint X, uint Y, uint Z)? ThreadGroupSize = null);
}

namespace AssetConverter.ShaderTranslation.Dxbc
{
    public enum ShaderStage : ushort { Pixel = 0, Vertex = 1, Geometry = 2, Hull = 3, Domain = 4, Compute = 5 }

    public enum OperandType : byte
    {
        Temp = 0, Input = 1, Output = 2, IndexableTemp = 3, Immediate32 = 4, Immediate64 = 5, Sampler = 6, Resource = 7,
        ConstantBuffer = 8, ImmediateConstantBuffer = 9, Label = 10, InputPrimitiveId = 11, OutputDepth = 12, Null = 13,
        Rasterizer = 14, OutputCoverageMask = 15, Stream = 16, FunctionBody = 17, FunctionTable = 18, Interface = 19,
        FunctionInput = 20, FunctionOutput = 21, OutputControlPointId = 22, InputForkInstanceId = 23, InputJoinInstanceId = 24,
        InputControlPoint = 25, OutputControlPoint = 26, InputPatchConstant = 27, InputDomainPoint = 28, ThisPointer = 29,
        UnorderedAccessView = 30, ThreadGroupSharedMemory = 31, InputThreadId = 32, InputThreadGroupId = 33,
        InputThreadIdInGroup = 34, InputCoverageMask = 35, InputThreadIdInGroupFlattened = 36, InputGsInstanceId = 37,
        OutputDepthGreaterEqual = 38, OutputDepthLessEqual = 39, CycleCounter = 40,
    }

    public enum SelectionMode : byte { Mask = 0, Swizzle = 1, Select1 = 2 }

    public enum OperandModifier : byte { None = 0, Negate = 1, Absolute = 2, AbsoluteNegate = 3 }

    public enum IndexRepresentation : byte
    {
        Immediate32 = 0, Immediate64 = 1, Relative = 2, Immediate32PlusRelative = 3, Immediate64PlusRelative = 4,
    }

    public sealed class OperandIndex
    {
        public IndexRepresentation Representation { get; init; }
        public ulong Immediate { get; init; }
        public Operand? Relative { get; init; }
    }

    public sealed class Operand
    {
        public OperandType Type { get; init; }
        public int ComponentCount { get; init; }
        public SelectionMode Mode { get; init; }
        public byte Mask { get; init; }
        public byte[] Swizzle { get; init; } = [0, 1, 2, 3];
        public byte Select1 { get; init; }
        public IReadOnlyList<OperandIndex> Indices { get; init; } = [];
        public OperandModifier Modifier { get; init; }
        public int MinPrecision { get; init; }
        public bool NonUniform { get; init; }
        public IReadOnlyList<uint> Immediate { get; init; } = [];

        public bool IsImmediate => Type is OperandType.Immediate32 or OperandType.Immediate64;

        /// <summary>Source component selected for destination slot <paramref name="slot"/> (0-3).</summary>
        public int SourceComponent(int slot) => Mode switch
        {
            SelectionMode.Select1 => Select1,
            SelectionMode.Swizzle => Swizzle[slot],
            _ => slot,
        };

        public int ImmediateIndex0 => Indices.Count > 0 ? (int)Indices[0].Immediate : 0;
    }

    public enum ResourceDimension : byte
    {
        Unknown = 0, Buffer, Texture1D, Texture2D, Texture2DMs, Texture3D, TextureCube, Texture1DArray,
        Texture2DArray, Texture2DMsArray, TextureCubeArray, RawBuffer, StructuredBuffer,
    }

    public enum ResourceReturnType : byte
    {
        None = 0, Unorm, Snorm, SignedInt, UnsignedInt, Real32, Mixed, Real64, Continued, Unused,
    }

    public enum InterpolationMode : byte
    {
        Undefined = 0, Constant, Linear, LinearCentroid, LinearNoperspective, LinearNoperspectiveCentroid,
        LinearSample, LinearNoperspectiveSample,
    }

    public enum SamplerMode : byte { Default = 0, Comparison = 1, Mono = 2 }

    public enum ResinfoReturnType : byte { Real32 = 0, ReciprocalFloat = 1, UnsignedInt = 2 }

    /// <summary>Class of a <c>customdata</c> block (the instruction's control field).</summary>
    public static class CustomDataClass
    {
        public const uint ImmediateConstantBuffer = 3;
    }

    public sealed class Instruction
    {
        private const uint SaturateBit = 0x4;
        private const uint TestNonZeroBit = 0x80;
        private const int PreciseShift = 8;
        private const uint PreciseMask = 0xF;
        private const uint DimensionMask = 0x1F;
        private const uint InterpolationMask = 0xF;
        private const uint DynamicIndexedBit = 1;
        private const uint ResinfoReturnMask = 0x3;
        private const uint SyncFlagMask = 0xF;
        private const uint SamplerModeMask = 0xF;

        public Opcode Opcode { get; init; }
        public uint Control { get; init; }
        public IReadOnlyList<Operand> Operands { get; init; } = [];
        public IReadOnlyList<uint> Data { get; init; } = [];

        public bool HasSampleOffset { get; init; }
        public sbyte OffsetU { get; init; }
        public sbyte OffsetV { get; init; }
        public sbyte OffsetW { get; init; }
        public ResourceDimension ExtendedDimension { get; init; }
        public uint ExtendedStride { get; init; }
        public IReadOnlyList<ResourceReturnType> ExtendedReturnType { get; init; } = [];
        public int ByteOffset { get; init; }

        public bool HasNonZeroSampleOffset => HasSampleOffset && (OffsetU != 0 || OffsetV != 0 || OffsetW != 0);
        public bool Saturate => (Control & SaturateBit) != 0 && !OpcodeInfo.IsDeclaration(Opcode);
        public bool TestNonZero => (Control & TestNonZeroBit) != 0;
        public bool Precise => (Control >> PreciseShift & PreciseMask) != 0;
        public ResourceDimension Dimension => (ResourceDimension)(Control & DimensionMask);
        public InterpolationMode Interpolation => (InterpolationMode)(Control & InterpolationMask);
        public bool DynamicIndexed => (Control & DynamicIndexedBit) != 0;
        public ResinfoReturnType ResinfoReturn => (ResinfoReturnType)(Control & ResinfoReturnMask);
        public uint SyncFlags => Control & SyncFlagMask;
        public SamplerMode SamplerKind => (SamplerMode)(Control & SamplerModeMask);
    }

    public sealed record SignatureElement(
        string SemanticName, uint SemanticIndex, uint SystemValue, uint ComponentType, uint Register, byte Mask, byte ReadWriteMask, uint Stream);

    public sealed class Sm5Program
    {
        public required ShaderStage Stage { get; init; }
        public required int Major { get; init; }
        public required int Minor { get; init; }
        public required IReadOnlyList<Instruction> Instructions { get; init; }
        public IReadOnlyList<SignatureElement> Inputs { get; init; } = [];
        public IReadOnlyList<SignatureElement> Outputs { get; init; } = [];
        public IReadOnlyList<SignatureElement> PatchConstants { get; init; } = [];
    }
}

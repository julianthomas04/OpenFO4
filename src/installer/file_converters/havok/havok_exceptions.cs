namespace AssetConverter.Havok
{
    public class HavokException : Exception
    {
        public HavokException(string message) : base(message) { }
    }

    public sealed class HavokFormatException : HavokException
    {
        public HavokFormatException(string message) : base(message) { }
    }

    public sealed class HavokUnsupportedClassException : HavokException
    {
        public string ClassName { get; }

        public HavokUnsupportedClassException(string context, string className)
            : base($"Unsupported Havok class '{className}' ({context})")
        {
            ClassName = className;
        }
    }

    public sealed class HavokUnsupportedQuantizationException : HavokException
    {
        public HavokUnsupportedQuantizationException(string message) : base(message) { }
    }

    public sealed class HavokDuplicateBoneNameException : HavokException
    {
        public HavokDuplicateBoneNameException(string boneName)
            : base($"Skeleton contains more than one bone named '{boneName}'; glTF nodes would be ambiguous")
        {
        }
    }

    public sealed class HavokAmbiguousSkeletonException : HavokException
    {
        public HavokAmbiguousSkeletonException(string file, string reason)
            : base($"Cannot choose the animation skeleton in '{file}': {reason}")
        {
        }
    }

    public sealed class HavokSkeletonMismatchException : HavokException
    {
        public HavokSkeletonMismatchException(string clipName, string? clipSkeleton, string? skeletonName)
            : base($"Clip '{clipName}' binds skeleton '{clipSkeleton}' but the creature skeleton is '{skeletonName}'")
        {
        }
    }

    public sealed class HavokPackNameCollisionException : HavokException
    {
        public HavokPackNameCollisionException(string packName)
            : base($"An Animations subfolder named '{packName}' collides with the pack for clips directly under Animations")
        {
        }
    }

    public sealed class HavokRenderSkeletonMissingException : HavokException
    {
        public HavokRenderSkeletonMissingException(string unit, string path)
            : base($"Unit '{unit}' has no NIF-converted render skeleton at '{path}'; clips cannot bind to it")
        {
        }
    }
}

namespace AssetConverter
{
    /// <summary>SWF lengths are twips (1/20 px); readers keep them as twips and convert at the point they write pixels.</summary>
    internal static class Twips
    {
        public const double PerPixel = 20.0;

        public static double ToPixels(double twips) => twips / PerPixel;
    }

    /// <summary>A 2x3 affine in SWF convention (x' = A·x + C·y + Tx, y' = B·x + D·y + Ty) with a translation in twips.</summary>
    internal readonly record struct SwfMatrix(double A, double B, double C, double D, double Tx, double Ty)
    {
        /// <summary>The slack the 16.16 fixed-point scale factors of a SWF matrix leave on a matrix that nominally is a pure translation.</summary>
        private const double IdentityTolerance = 1e-6;

        public static readonly SwfMatrix Identity = new(1, 0, 0, 1, 0, 0);

        /// <summary>The matrix that applies <paramref name="child"/> first and this one second.</summary>
        public SwfMatrix Compose(SwfMatrix child) => new(
            A * child.A + C * child.B,
            B * child.A + D * child.B,
            A * child.C + C * child.D,
            B * child.C + D * child.D,
            A * child.Tx + C * child.Ty + Tx,
            B * child.Tx + D * child.Ty + Ty);

        public (double X, double Y) Apply(double x, double y) => (A * x + C * y + Tx, B * x + D * y + Ty);

        public bool IsTranslationOnly =>
            Math.Abs(A - 1) <= IdentityTolerance && Math.Abs(D - 1) <= IdentityTolerance
            && Math.Abs(B) <= IdentityTolerance && Math.Abs(C) <= IdentityTolerance;
    }

    /// <summary>An axis-aligned box in twips.</summary>
    internal readonly record struct TwipRect(double Xmin, double Ymin, double Xmax, double Ymax)
    {
        public double Width => Xmax - Xmin;

        public double Height => Ymax - Ymin;

        public TwipRect Union(TwipRect other) => new(
            Math.Min(Xmin, other.Xmin), Math.Min(Ymin, other.Ymin),
            Math.Max(Xmax, other.Xmax), Math.Max(Ymax, other.Ymax));

        /// <summary>The bounding box of the four corners under <paramref name="matrix"/>, so a rotation is not lost.</summary>
        public TwipRect Transform(SwfMatrix matrix)
        {
            var corners = new[]
            {
                matrix.Apply(Xmin, Ymin), matrix.Apply(Xmax, Ymin),
                matrix.Apply(Xmax, Ymax), matrix.Apply(Xmin, Ymax),
            };
            return new TwipRect(
                corners.Min(c => c.X), corners.Min(c => c.Y),
                corners.Max(c => c.X), corners.Max(c => c.Y));
        }
    }
}

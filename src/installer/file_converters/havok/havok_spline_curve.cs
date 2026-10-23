namespace AssetConverter.Havok
{
    /// <summary>Non-uniform B-spline over N-dimensional control points, evaluated with De Boor's algorithm.</summary>
    internal sealed class HavokSplineCurve
    {
        private readonly int _degree;
        private readonly byte[] _knots;
        private readonly float[] _points;
        private readonly int _pointCount;
        private readonly int _dimensions;

        public HavokSplineCurve(int degree, byte[] knots, float[] points, int dimensions)
        {
            _pointCount = points.Length / dimensions;
            if (degree < 0 || _pointCount < degree + 1)
                throw new HavokFormatException($"Spline of degree {degree} needs at least {degree + 1} control points, has {_pointCount}");
            if (knots.Length != _pointCount + degree + 1)
                throw new HavokFormatException($"Spline has {knots.Length} knots, expected {_pointCount + degree + 1}");
            for (int i = 1; i < knots.Length; i++)
                if (knots[i] < knots[i - 1])
                    throw new HavokFormatException("Spline knots are not non-decreasing");

            _degree = degree;
            _knots = knots;
            _points = points;
            _dimensions = dimensions;
        }

        public void Evaluate(float u, Span<float> result)
        {
            int last = _pointCount - 1;
            u = Math.Clamp(u, _knots[_degree], _knots[last + 1]);
            int span = FindSpan(u, last);

            Span<float> work = stackalloc float[(_degree + 1) * _dimensions];
            for (int j = 0; j <= _degree; j++)
                _points.AsSpan((span - _degree + j) * _dimensions, _dimensions).CopyTo(work.Slice(j * _dimensions, _dimensions));

            for (int r = 1; r <= _degree; r++)
            {
                for (int j = _degree; j >= r; j--)
                {
                    int i = j + span - _degree;
                    float denominator = _knots[i + _degree - r + 1] - _knots[i];
                    float alpha = denominator > 0f ? (u - _knots[i]) / denominator : 0f;
                    for (int d = 0; d < _dimensions; d++)
                        work[j * _dimensions + d] = (1f - alpha) * work[(j - 1) * _dimensions + d] + alpha * work[j * _dimensions + d];
                }
            }
            work.Slice(_degree * _dimensions, _dimensions).CopyTo(result);
        }

        private int FindSpan(float u, int last)
        {
            if (u >= _knots[last + 1])
                return last;
            for (int k = _degree; k < last; k++)
                if (u < _knots[k + 1])
                    return k;
            return last;
        }
    }
}

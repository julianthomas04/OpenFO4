using System.Text;

namespace AssetConverter
{
    internal sealed class KeyValuesFormatException(string message) : Exception(message);

    /// <summary>One node of Valve's KeyValues text format: either a string value or a list of named children.</summary>
    internal sealed class KeyValuesNode
    {
        public string? Value { get; init; }
        public IReadOnlyList<KeyValuePair<string, KeyValuesNode>> Children { get; init; } = [];

        public KeyValuesNode? Child(string key) =>
            Children.FirstOrDefault(pair => pair.Key.Equals(key, StringComparison.OrdinalIgnoreCase)).Value;

        public string? ValueOf(string key) => Child(key)?.Value;
    }

    internal static class KeyValuesText
    {
        /// <summary>Parses a document of the form <c>"root" { "key" "value" "group" { ... } }</c>; the result is the root's value.</summary>
        public static KeyValuesNode Parse(string text)
        {
            var tokens = new Tokenizer(text);
            if (tokens.Next().Kind != TokenKind.Text) throw new KeyValuesFormatException("The document does not start with a key.");
            KeyValuesNode root = ParseValue(tokens);
            if (tokens.Next().Kind != TokenKind.End) throw new KeyValuesFormatException("Unexpected content after the root object.");
            return root;
        }

        private static KeyValuesNode ParseValue(Tokenizer tokens)
        {
            Token token = tokens.Next();
            return token.Kind switch
            {
                TokenKind.Text => new KeyValuesNode { Value = token.Text },
                TokenKind.Open => ParseBody(tokens),
                _ => throw new KeyValuesFormatException("A key has no value."),
            };
        }

        private static KeyValuesNode ParseBody(Tokenizer tokens)
        {
            var children = new List<KeyValuePair<string, KeyValuesNode>>();
            while (true)
            {
                Token key = tokens.Next();
                if (key.Kind == TokenKind.Close) return new KeyValuesNode { Children = children };
                if (key.Kind != TokenKind.Text) throw new KeyValuesFormatException("An object is not closed or has a value without a key.");
                children.Add(new(key.Text, ParseValue(tokens)));
            }
        }

        private enum TokenKind { Text, Open, Close, End }

        private readonly record struct Token(TokenKind Kind, string Text = "");

        private sealed class Tokenizer(string text)
        {
            private int _position;

            public Token Next()
            {
                SkipBlanksAndComments();
                if (_position >= text.Length) return new Token(TokenKind.End);
                char c = text[_position];
                if (c == '{')
                {
                    _position++;
                    return new Token(TokenKind.Open);
                }
                if (c == '}')
                {
                    _position++;
                    return new Token(TokenKind.Close);
                }
                return new Token(TokenKind.Text, c == '"' ? ReadQuoted() : ReadBare());
            }

            private void SkipBlanksAndComments()
            {
                while (_position < text.Length)
                {
                    if (char.IsWhiteSpace(text[_position])) _position++;
                    else if (text.AsSpan(_position).StartsWith("//")) SkipLine();
                    else return;
                }
            }

            private void SkipLine()
            {
                while (_position < text.Length && text[_position] != '\n') _position++;
            }

            private string ReadBare()
            {
                int start = _position;
                while (_position < text.Length && !char.IsWhiteSpace(text[_position]) && text[_position] is not ('{' or '}' or '"')) _position++;
                return text[start.._position];
            }

            private string ReadQuoted()
            {
                _position++;
                var value = new StringBuilder();
                while (_position < text.Length)
                {
                    char c = text[_position++];
                    if (c == '"') return value.ToString();
                    value.Append(c == '\\' ? ReadEscape() : c);
                }
                throw new KeyValuesFormatException("A quoted string is not closed.");
            }

            private char ReadEscape()
            {
                if (_position >= text.Length) throw new KeyValuesFormatException("A quoted string ends inside an escape sequence.");
                char escaped = text[_position++];
                return escaped switch { 'n' => '\n', 't' => '\t', _ => escaped };
            }
        }
    }
}

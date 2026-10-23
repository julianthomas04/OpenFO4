using System.Text.RegularExpressions;

namespace InstallerTests.Support
{
    /// <summary>
    /// Offline structural check of emitted WGSL for the mistakes the emitter can make without a WGSL compiler at hand: unbalanced
    /// delimiters, locals used before (or outside the scope of) their declaration, and discards outside the deferred form.
    /// The engine validates the same text with naga at startup; this catches the problem at install-test time.
    /// </summary>
    internal static partial class WgslLint
    {
        [GeneratedRegex(@"\b(tmp\d+|bits_[0-9a-f]{8}|discard_pending|r\d+)\b")]
        private static partial Regex GeneratedLocal();

        [GeneratedRegex(@"\bx(\d+)\[")]
        private static partial Regex IndexableLocal();

        [GeneratedRegex(@"^\s*(?:let|var)\s+(\w+)")]
        private static partial Regex LocalDeclaration();

        [GeneratedRegex(@"\bdiscard\s*;")]
        private static partial Regex DiscardStatement();

        public static IReadOnlyList<string> Problems(string wgsl)
        {
            var problems = new List<string>();
            var lines = StripComments(wgsl).Split('\n');
            CheckDelimiters(lines, problems);
            CheckFunctionLocals(lines, problems);
            CheckDiscards(lines, problems);
            return problems;
        }

        private static string StripComments(string wgsl) =>
            string.Join('\n', wgsl.Replace("\r\n", "\n").Split('\n').Select(l => l.Contains("//") ? l[..l.IndexOf("//", StringComparison.Ordinal)] : l));

        private static void CheckDelimiters(string[] lines, List<string> problems)
        {
            var open = new Stack<(char Char, int Line)>();
            const string Opening = "({[";
            const string Closing = ")}]";
            for (int n = 0; n < lines.Length; n++)
            {
                foreach (char c in lines[n])
                {
                    int kind = Opening.IndexOf(c);
                    if (kind >= 0)
                    {
                        open.Push((c, n + 1));
                        continue;
                    }
                    kind = Closing.IndexOf(c);
                    if (kind < 0) continue;
                    if (open.Count == 0 || open.Peek().Char != Opening[kind])
                        problems.Add($"line {n + 1}: unmatched '{c}'");
                    else
                        open.Pop();
                }
            }
            foreach (var (c, line) in open)
                problems.Add($"line {line}: '{c}' is never closed");
        }

        private static void CheckFunctionLocals(string[] lines, List<string> problems)
        {
            var scopes = new Stack<HashSet<string>>();
            scopes.Push([]);
            bool insideFunction = false;
            for (int n = 0; n < lines.Length; n++)
            {
                string line = lines[n];
                if (line.StartsWith("fn ", StringComparison.Ordinal)) insideFunction = true;
                if (!insideFunction) continue;

                var declaration = LocalDeclaration().Match(line);
                string? declared = declaration.Success ? declaration.Groups[1].Value : null;
                int declarationEnd = declaration.Success ? declaration.Length : 0;

                var segment = new System.Text.StringBuilder();
                void Flush()
                {
                    string text = segment.ToString();
                    segment.Clear();
                    foreach (var name in UsedGeneratedLocals(text))
                        if (!scopes.Any(s => s.Contains(name)))
                            problems.Add($"line {n + 1}: '{name}' is used before its declaration or outside its scope");
                }

                for (int i = 0; i < line.Length; i++)
                {
                    char c = line[i];
                    if (c is not ('{' or '}'))
                    {
                        if (i >= declarationEnd) segment.Append(c);
                        continue;
                    }
                    Flush();
                    if (c == '{')
                    {
                        scopes.Push([]);
                    }
                    else if (scopes.Count > 1)
                    {
                        scopes.Pop();
                    }
                }
                Flush();
                if (declared != null)
                {
                    if (scopes.Peek().Contains(declared)) problems.Add($"line {n + 1}: '{declared}' is declared twice in one scope");
                    scopes.Peek().Add(declared);
                }
            }
        }

        private static IEnumerable<string> UsedGeneratedLocals(string text)
        {
            foreach (Match m in GeneratedLocal().Matches(text)) yield return m.Groups[1].Value;
            foreach (Match m in IndexableLocal().Matches(text)) yield return "x" + m.Groups[1].Value;
        }

        private static void CheckDiscards(string[] lines, List<string> problems)
        {
            for (int n = 0; n < lines.Length; n++)
            {
                if (DiscardStatement().IsMatch(lines[n]) && lines[n].Trim() != "if (discard_pending) { discard; }")
                    problems.Add($"line {n + 1}: discard outside the deferred 'if (discard_pending)' form");
            }
        }
    }
}

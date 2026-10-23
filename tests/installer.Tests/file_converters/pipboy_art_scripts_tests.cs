using AssetConverter;
using AssetConverter.Abc;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    // A clip's script is what its timeline class's frame scripts do to its own playback.
    public sealed class PipboyArtScriptsTests
    {
        const int Card = 5;

        /// <summary>A four-frame sprite of class <c>Card</c> labelled <c>loop</c> on frame 2, whose constructor registers <paramref name="frames"/>.</summary>
        static SwfMovie Movie(params (int FrameIndex, Func<AbcBuilder, AbcCode, AbcCode> Body)[] frames) => Movie(conditional: false, frames);

        static SwfMovie Movie(bool conditional, params (int FrameIndex, Func<AbcBuilder, AbcCode, AbcCode> Body)[] frames)
        {
            var abc = new AbcBuilder();
            var traits = new List<AbcTraitSpec>();
            var register = new AbcCode().GetLocal0().PushScope().GetLocal0().ConstructSuper(0);
            int skip = conditional ? register.GetLocal(1).IfFalse() : -1;
            register.GetLocal0();
            foreach ((int index, var body) in frames)
            {
                string name = $"frame{index + 1}";
                int method = abc.Function(body(abc, new AbcCode().GetLocal0().PushScope()).ReturnVoid().ToArray());
                traits.Add(new AbcMethodTraitSpec(abc.Name(name), method));
                register.PushByte((sbyte)index).GetLocal0().GetProperty(abc.Name(name));
            }
            register.CallPropVoid(abc.Name("addFrameScript"), frames.Length * 2);
            if (skip >= 0)
                register.Here(skip);
            int init = abc.Function(register.ReturnVoid().ToArray(), parameters: 1);
            abc.Class("", "Card", 0, init, abc.Method(), traits);

            return new SwfMovieBuilder()
                .Sprite(Card, t => t.ShowFrame().Label("loop").ShowFrame().ShowFrame().ShowFrame())
                .SymbolClass((Card, "Card"))
                .Abc("frames", abc.Build())
                .Read();
        }

        static Func<AbcBuilder, AbcCode, AbcCode> Call(string method) => (abc, code) => code.GetLocal0().CallPropVoid(abc.Name(method), 0);

        static Func<AbcBuilder, AbcCode, AbcCode> Goto(string method, sbyte frame) =>
            (abc, code) => code.GetLocal0().PushByte(frame).CallPropVoid(abc.Name(method), 1);

        static Func<AbcBuilder, AbcCode, AbcCode> GotoLabel(string method, string label) =>
            (abc, code) => code.GetLocal0().PushString(abc.String(label)).CallPropVoid(abc.Name(method), 1);

        static IReadOnlyList<ClipScriptStep> Script(SwfMovie movie, int first = 1, int last = 4) =>
            new PipboyArtScripts(movie, "Menu.swf").Of(Card, Enumerable.Range(first, last - first + 1).ToList());

        [Fact]
        public void Of_StopFirstAndLoopBackFromTheLast_AreTheClipsSteps()
        {
            var movie = Movie((0, Call("stop")), (3, Goto("gotoAndPlay", 2)));

            Assert.Equal([new(1, "stop", null), new(4, "gotoAndPlay", 2)], Script(movie));
        }

        [Fact]
        public void Of_GotoALabel_GoesToItsFrame()
        {
            Assert.Equal([new(4, "gotoAndStop", 2)], Script(Movie((3, GotoLabel("gotoAndStop", "loop")))));
        }

        [Fact]
        public void Of_ClipOfLaterFrames_NumbersStepsFromItsFirstFrame()
        {
            var movie = Movie((0, Call("stop")), (3, Goto("gotoAndPlay", 2)));

            Assert.Equal([new(3, "gotoAndPlay", 1)], Script(movie, first: 2));
        }

        [Fact]
        public void Of_GotoOutsideTheClip_Throws()
        {
            var movie = Movie((3, Goto("gotoAndPlay", 1)));

            Assert.Throws<FrameScriptException>(() => Script(movie, first: 2));
        }

        [Fact]
        public void Of_ConditionalRegistration_Throws()
        {
            Assert.Throws<FrameScriptException>(() => Script(Movie(conditional: true, (0, Call("stop")))));
        }

        [Fact]
        public void Of_TimelineWithoutAClass_HasNoSteps()
        {
            var movie = new SwfMovieBuilder().Sprite(7, t => t.ShowFrame().ShowFrame()).Read();

            Assert.Empty(new PipboyArtScripts(movie, "Menu.swf").Of(7, [1, 2]));
        }

        [Fact]
        public void Of_OtherCallsOfTheFrameMethod_AreNotSteps()
        {
            Assert.Equal([new(1, "stop", null)], Script(Movie((0, (abc, code) => Call("stop")(abc, Call("redraw")(abc, code))))));
        }
    }
}

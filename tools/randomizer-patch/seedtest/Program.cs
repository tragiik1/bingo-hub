using System; using System.Linq; using System.Reflection; using System.Threading; using System.Collections.Generic;
var t = typeof(Project.Tasks.Randomizer);
var F = BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static;
string game = args[0];
foreach (var seed in args.Skip(1)) {
  if (seed.StartsWith("rate:")) {
    var rp = seed[5..].Split((char)58); int start = rp.Length > 1 ? int.Parse(rp[0]) : 0; int n = int.Parse(rp[^1]), bad = 0; var keys = new Dictionary<string,int>();
    for (int i = start; i < start + n; i++) {
      var r = Project.Tasks.Randomizer.BuildRandomizerAsync(game, "s" + i, CancellationToken.None).Result;
      try { r.RandomizeRegulation().Wait(); }
      catch (Exception e0) { var e = e0 is AggregateException a ? a.InnerException : e0; bad++; var k = e.Message; keys[k] = keys.GetValueOrDefault(k) + 1; }
    }
    Console.WriteLine($"{bad}/{n} seeds crash");
    foreach (var kv in keys.OrderByDescending(k => k.Value)) Console.WriteLine($"  {kv.Value}x {kv.Key}");
    continue;
  }
  var rr = Project.Tasks.Randomizer.BuildRandomizerAsync(game, seed, CancellationToken.None).Result;
  try { rr.RandomizeRegulation().Wait(); Console.WriteLine($"{seed}: ok"); }
  catch (Exception e0) { var e = e0 is AggregateException a ? a.InnerException : e0; Console.WriteLine($"{seed}: CRASH {e.Message}"); }
}

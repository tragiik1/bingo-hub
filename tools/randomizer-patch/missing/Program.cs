using System; using System.Linq; using System.Reflection; using System.Threading; using System.Collections;
using System.Collections.Generic;
var F = BindingFlags.NonPublic | BindingFlags.Instance;
var r = Project.Tasks.Randomizer.BuildRandomizerAsync(args[0], "x", CancellationToken.None).Result;
var t = typeof(Project.Tasks.Randomizer);
var wd = (IDictionary)t.GetField("_weaponDictionary", F).GetValue(r);
var names = (Dictionary<int,string>)t.GetField("_weaponNameDictionary", F).GetValue(r);
var table = Project.Settings.Equipment.EquipmentNameList;
foreach (int b in wd.Keys.Cast<int>().Select(k => k / 10000 * 10000).Distinct().Where(b => !table.ContainsKey(b)).OrderBy(b => b))
  Console.WriteLine($"{b}\t{(names.TryGetValue(b, out var n) ? n : "?")}");

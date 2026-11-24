using System; using System.Collections.Generic; using System.IO; using System.Linq; using System.Reflection; using System.Text; using System.Text.RegularExpressions; using System.Threading;
using SoulsFormats; using FSParam; using BCnEncoder.Decoder; using BCnEncoder.Shared.ImageFiles;
using SixLabors.ImageSharp; using SixLabors.ImageSharp.PixelFormats; using SixLabors.ImageSharp.Processing;
// Builds the Classes page's item data from the player's own game files (vanilla params and icons).
// args: gameDir outDir iconPx [pal]
//   items.tsv  name<TAB>kind<TAB>smithing|somber|<empty><TAB>iconId  (weapons, then spells)
//   weapons.tsv  per weapon: attack, scaling, requirements, status, skill; scaling curves; spell costs
//   icons.bin  "BHIC", u32 count, (u32 iconId, u32 length) x count, then the PNGs
// Upgrade type: the weapon's EquipMtrlSetParam rows (Smithing Stones 10100-10109, Somber 10160-10169);
// ammo, Unarmed and weapons with no +1 (Meteorite Staff, Serpent-Hunter) count as not upgradable.
var F = BindingFlags.NonPublic | BindingFlags.Instance;
string game = args[0], outDir = args[1]; int px = int.Parse(args[2]);
Directory.CreateDirectory(outDir);
var r = Project.Tasks.Randomizer.BuildRandomizerAsync(game + "/eldenring.exe", "x", CancellationToken.None).Result;
T G<T>(string n) => (T)typeof(Project.Tasks.Randomizer).GetField(n, F).GetValue(r);
var bhd = G<Project.FileHandler.BHD5Reader>("_bhd5Reader");
Project.Utility.Kernel32.LoadLibrary(game + "/oo2core_6_win64.dll");
var wep = G<Param>("_equipParamWeapon"); var goods = G<Param>("_equipParamGoods"); var mtrl = G<Param>("_equipMtrlSetParam");
var wfmg = G<FMG>("_weaponFmg"); var gfmg = G<FMG>("_goodsFmg");
int I(Param.Row row, string f) => Convert.ToInt32(row[f].Value.Value);
var mtrlById = mtrl.Rows.ToDictionary(x => x.ID);
var kinds = new Dictionary<int,string>{{1,"Dagger"},{3,"Straight Sword"},{5,"Greatsword"},{7,"Colossal Sword"},{9,"Curved Sword"},{11,"Curved Greatsword"},{13,"Katana"},{14,"Twinblade"},{15,"Thrusting Sword"},{16,"Heavy Thrusting Sword"},{17,"Axe"},{19,"Greataxe"},{21,"Hammer"},{23,"Great Hammer"},{24,"Flail"},{25,"Spear"},{28,"Great Spear"},{29,"Halberd"},{31,"Reaper"},{35,"Fist"},{37,"Claw"},{39,"Whip"},{41,"Colossal Weapon"},{50,"Light Bow"},{51,"Bow"},{53,"Greatbow"},{55,"Crossbow"},{56,"Ballista"},{57,"Glintstone Staff"},{61,"Sacred Seal"},{65,"Small Shield"},{67,"Medium Shield"},{69,"Greatshield"},{81,"Arrow"},{83,"Greatarrow"},{85,"Bolt"},{86,"Ballista Bolt"},{87,"Torch"},{88,"Hand-to-Hand Art"},{89,"Perfume Bottle"},{90,"Thrusting Shield"},{91,"Throwing Blade"},{92,"Backhand Blade"},{93,"Light Greatsword"},{94,"Great Katana"},{95,"Beast Claw"}};
var rows = new List<(string name,string kind,string up,int icon)>(); var unknownKinds = new SortedSet<int>();
foreach (var row in wep.Rows) {
  if (row.ID % 10000 != 0) continue; var name = wfmg[row.ID];
  if (string.IsNullOrWhiteSpace(name) || name.Contains("[ERROR]")) continue;
  int t = I(row,"wepType"); if (!kinds.TryGetValue(t, out var kind)) { unknownKinds.Add(t); kind = ""; }
  string up = "";
  int ms = I(row,"materialSetId");
  for (int id = ms; id < ms + 4 && up == ""; id++) if (mtrlById.TryGetValue(id, out var m) && I(m,"materialCate01")==4) {
    int mat = I(m,"materialId01"); up = mat>=10100&&mat<10110 ? "smithing" : mat>=10160&&mat<10170 ? "somber" : ""; }
  if (t is 81 or 83 or 85 or 86 or 33 || I(row,"originEquipWep1") < 0 || I(row,"reinforceTypeId") == 3000) up = "";
  rows.Add((name.Trim(), kind, up, I(row,"iconId")));
}
foreach (var row in goods.Rows) {
  int gt = I(row,"goodsType"); if (gt!=5 && (gt<16||gt>18)) continue;
  var name = gfmg[row.ID]; if (string.IsNullOrWhiteSpace(name) || name.Contains("[ERROR]")) continue;
  rows.Add((name.Trim(), gt==5||gt==17 ? "Sorcery" : "Incantation", "", I(row,"iconId")));
}
Console.WriteLine($"items {rows.Count}; unknown wepTypes: {string.Join(",", unknownKinds)}");
Console.WriteLine("upgrade kinds: " + string.Join(", ", rows.GroupBy(x=>x.up).Select(g=>$"{(g.Key==""?"none":g.Key)}={g.Count()}")));
// dedupe by name (first wins: weapons before spells)
var seen = new HashSet<string>(); var outRows = rows.Where(x => seen.Add(x.name)).ToList();
File.WriteAllText(Path.Combine(outDir,"items.tsv"), string.Join("\n", outRows.Select(x=>$"{x.name}\t{x.kind}\t{x.up}\t{x.icon}"))+"\n", new UTF8Encoding(false));
// weapons.tsv: what the Classes page needs to work out attack rating, plus skills and spell costs.
// Classes get plain +0 weapons with their own skills (the Randomizer never changes these params).
var reg = G<BND4>("_regulationBnd");
// The Randomizer's own layouts, plus four it doesn't ship (from Paramdex, in .\defs).
var defs = Directory.GetFiles("Resources/Params/Defs", "*.xml").Concat(Directory.GetFiles(Path.Combine(AppContext.BaseDirectory, "defs"), "*.xml"))
  .Select(f => { var x = new System.Xml.XmlDocument(); x.Load(f); return PARAMDEF.XmlSerializer.Deserialize(x, true); }).ToList();
Param P(string file) {
  var p = Param.Read(reg.Files.First(f => Path.GetFileName(f.Name) == file).Bytes);
  if (!Project.Utility.ExtensionMethods.ApplyParamDefsCarefully(p, defs)) throw new Exception("No paramdef fits " + file);
  return p;
}
var reinforce = P("ReinforceParamWeapon.param").Rows.ToDictionary(x => x.ID);
var graphs = P("CalcCorrectGraph.param").Rows.ToDictionary(x => x.ID);
var aecs = P("AttackElementCorrectParam.param").Rows.ToDictionary(x => x.ID);
var spfx = P("SpEffectParam.param").Rows.ToDictionary(x => x.ID);
var magic = P("Magic.param").Rows.ToDictionary(x => x.ID);
var arts = new Dictionary<int,string>();
foreach (var f in BND4.Read(bhd.GetFile("/msg/engus/item_dlc02.msgbnd.dcx")).Files.Where(f => Path.GetFileName(f.Name).StartsWith("ArtsName")))
  foreach (var e in FMG.Read(f.Bytes).Entries) if (!string.IsNullOrWhiteSpace(e.Text) && !e.Text.Contains("[ERROR]") && !e.Text.Contains("dummy", StringComparison.OrdinalIgnoreCase)) arts[e.ID] = e.Text.Trim();
var swordArts = P("SwordArtsParam.param").Rows.ToDictionary(x => x.ID);
double D(Param.Row row, string f) => Convert.ToDouble(row[f].Value.Value);
string N(double v) => Math.Round(v, 4).ToString(System.Globalization.CultureInfo.InvariantCulture);
var els = new[]{"Physics","Magic","Fire","Thunder","Dark"};
var attrs = new[]{("Strength","Strength"),("Agility","Dexterity"),("Magic","Magic"),("Faith","Faith"),("Luck","Luck")};
// First line: the game version this was built from, so the hub can tell when a patch has made it stale.
var fv = System.Diagnostics.FileVersionInfo.GetVersionInfo(Path.Combine(game, "eldenring.exe"));
var gameVer = fv.FilePrivatePart == 0 ? $"{fv.FileMajorPart}.{fv.FileMinorPart}.{fv.FileBuildPart}" : $"{fv.FileMajorPart}.{fv.FileMinorPart}.{fv.FileBuildPart}.{fv.FilePrivatePart}";
var lines = new List<string>{ "V	" + gameVer }; var usedGraphs = new SortedSet<int>{0, 6}; var usedAec = new SortedSet<int>(); var wseen = new HashSet<string>(); var usedReinforce = new SortedSet<int>(); var usedSpfx = new SortedSet<int>();
foreach (var row in wep.Rows) {
  // Plain weapons and their +0 infusions ("Quality Longsword"): classes can start with either.
  if (row.ID % 100 != 0) continue; var name = wfmg[row.ID]?.Trim();
  if (string.IsNullOrWhiteSpace(name) || name.Contains("[ERROR]") || !wseen.Add(name)) continue;
  if (row.ID % 10000 != 0 && wep[row.ID / 10000 * 10000] == null) continue;
  int t = I(row,"wepType"); if (t is 81 or 83 or 85 or 86) continue;
  if (!reinforce.TryGetValue(I(row,"reinforceTypeId"), out var rf)) { Console.WriteLine("no reinforce row for " + name); continue; }
  int rfType = I(row,"reinforceTypeId"), maxLv = 0;
  // Ammo, Unarmed and weapons with no +1 (Meteorite Staff) don't upgrade.
  if (I(row,"originEquipWep1") >= 0 && rfType != 3000) while (maxLv < 25 && reinforce.ContainsKey(rfType + maxLv + 1)) maxLv++;
  var atk = new[]{"attackBasePhysics","attackBaseMagic","attackBaseFire","attackBaseThunder","attackBaseDark"}
    .Zip(new[]{"physicsAtkRate","magicAtkRate","fireAtkRate","thunderAtkRate","darkAtkRate"}, (a, r) => D(row,a) * D(rf,r));
  var scale = new[]{"correctStrength","correctAgility","correctMagic","correctFaith","correctLuck"}
    .Zip(new[]{"correctStrengthRate","correctAgilityRate","correctMagicRate","correctFaithRate","correctLuckRate"}, (a, r) => D(row,a) * D(rf,r));
  var req = new[]{"properStrength","properAgility","properMagic","properFaith","properLuck"}.Select(f => I(row,f));
  var g = new[]{"correctType_Physics","correctType_Magic","correctType_Fire","correctType_Thunder","correctType_Dark","correctType_Poison","correctType_Blood","correctType_Sleep","correctType_Madness"}.Select(f => I(row,f)).ToArray();
  foreach (var id in g) usedGraphs.Add(id);
  int aec = I(row,"attackElementCorrectId"); usedAec.Add(aec);
  // Status buildup from the weapon's SpEffects, at +0 (ReinforceParamWeapon.spEffectId1..3 offsets).
  var status = new double[7]; var stf = new[]{"poizonAttackPower","diseaseAttackPower","bloodAttackPower","freezeAttackPower","sleepAttackPower","madnessAttackPower","curseAttackPower"};
  for (int k = 0; k < 3; k++) {
    int id = I(row,$"spEffectBehaviorId{k}"); if (id <= 0) continue;
    if (!spfx.TryGetValue(id + I(rf,$"spEffectId{k+1}"), out var se)) continue;
    for (int s = 0; s < 7; s++) status[s] += D(se, stf[s]);
  }
  var types = new List<string>();
  if (I(row,"isNormalAttackType") == 1) types.Add("Standard"); if (I(row,"isSlashAttackType") == 1) types.Add("Slash");
  if (I(row,"isBlowAttackType") == 1) types.Add("Strike"); if (I(row,"isThrustAttackType") == 1) types.Add("Pierce");
  // Skill: SwordArtsParam.textId names it in ArtsName. FP as the game shows it: the skill's own cost,
  // or for stances (Unsheathe 0, then 10/15) the light/heavy follow-up costs.
  string skill = "", fp = "";
  if (swordArts.TryGetValue(I(row,"swordArtsParamId"), out var sa)) {
    skill = arts.GetValueOrDefault(I(sa,"textId"), "");
    if (skill == "No Skill") skill = "";
    int l2 = I(sa,"useMagicPoint_L2"), r1 = I(sa,"useMagicPoint_R1"), r2 = I(sa,"useMagicPoint_R2");
    fp = skill == "" ? "" : l2 > 0 ? l2.ToString() : string.Join("/", new[]{r1, r2}.Where(x => x > 0).Distinct());
  }
  lines.Add(string.Join("\t", new[]{"W", name, skill, fp, string.Join(" / ", types)}.Concat(atk.Select(N)).Concat(scale.Select(N)).Concat(req.Select(x => x.ToString()))
    .Concat(g.Select(x => x.ToString())).Append(aec.ToString()).Concat(status.Select(N))
    .Append(I(row,"isDualBlade").ToString()).Append(I(row,"enableMagic").ToString()).Append(I(row,"enableMiracle").ToString())
    // Upgrades: ReinforceParamWeapon rows reinforceTypeId+level multiply the raw values below.
    .Append(rfType.ToString()).Append(maxLv.ToString())
    .Concat(new[]{"attackBasePhysics","attackBaseMagic","attackBaseFire","attackBaseThunder","attackBaseDark","correctStrength","correctAgility","correctMagic","correctFaith","correctLuck"}.Select(f => N(D(row,f))))
    .Concat(Enumerable.Range(0,3).Select(k => Math.Max(0, I(row,$"spEffectBehaviorId{k}")).ToString()))));
  usedReinforce.Add(rfType);
  for (int lv = 0; lv <= maxLv; lv++) for (int k = 0; k < 3; k++) {
    int id = I(row,$"spEffectBehaviorId{k}"); if (id > 0 && reinforce.TryGetValue(rfType + lv, out var rl)) usedSpfx.Add(id + I(rl,$"spEffectId{k+1}"));
  }
}
foreach (var id in usedReinforce) for (int lv = 0; reinforce.TryGetValue(id + lv, out var rl) && lv <= 25; lv++)
  lines.Add(string.Join("	", new[]{"R", id.ToString(), lv.ToString()}
    .Concat(new[]{"physicsAtkRate","magicAtkRate","fireAtkRate","thunderAtkRate","darkAtkRate","correctStrengthRate","correctAgilityRate","correctMagicRate","correctFaithRate","correctLuckRate"}.Select(f => N(D(rl,f))))
    .Concat(Enumerable.Range(1,3).Select(k => I(rl,$"spEffectId{k}").ToString()))));
foreach (var id in usedSpfx) {
  if (!spfx.TryGetValue(id, out var se)) continue;
  var st = new[]{"poizonAttackPower","diseaseAttackPower","bloodAttackPower","freezeAttackPower","sleepAttackPower","madnessAttackPower","curseAttackPower"}.Select(f => D(se,f)).ToArray();
  if (st.Any(v => v != 0)) lines.Add(string.Join("	", new[]{"X", id.ToString()}.Concat(st.Select(N))));
}
foreach (var id in usedGraphs) {
  if (!graphs.TryGetValue(id, out var gr)) { Console.WriteLine("no graph " + id); continue; }
  lines.Add(string.Join("\t", new[]{"G", id.ToString()}.Concat(Enumerable.Range(0,5).SelectMany(k => new[]{N(D(gr,$"stageMaxVal{k}")), N(D(gr,$"stageMaxGrowVal{k}")), N(D(gr,$"adjPt_maxGrowVal{k}"))}))));
}
foreach (var id in usedAec) {
  if (!aecs.TryGetValue(id, out var ae)) { Console.WriteLine("no attack element row " + id); continue; }
  // Per element, per attribute: -2 = doesn't scale, -1 = scales with the weapon's own value, else an override rate.
  lines.Add(string.Join("\t", new[]{"E", id.ToString()}.Concat(els.SelectMany(el => attrs.Select(a =>
    I(ae,$"is{a.Item2}Correct_by{el}") == 0 ? "-2" : N(D(ae,$"overwrite{a.Item2}CorrectRate_by{el}")))))));
}
foreach (var row in goods.Rows) {
  int gt = I(row,"goodsType"); if (gt!=5 && (gt<16||gt>18)) continue;
  var name = gfmg[row.ID]?.Trim(); if (string.IsNullOrWhiteSpace(name) || name.Contains("[ERROR]") || !magic.TryGetValue(row.ID, out var m)) continue;
  lines.Add(string.Join("\t", "S", name, I(m,"mp"), I(m,"slotLength"), I(m,"requirementIntellect"), I(m,"requirementFaith"), I(m,"requirementLuck")));
}
File.WriteAllText(Path.Combine(outDir,"weapons.tsv"), string.Join("\n", lines)+"\n", new UTF8Encoding(false));
Console.WriteLine($"weapons.tsv: {lines.Count(l => l[0]=='W')} weapons, {usedReinforce.Count} upgrade paths, {lines.Count(l => l[0]=='X')} status effects, {usedGraphs.Count} graphs, {usedAec.Count} element rows, {lines.Count(l => l[0]=='S')} spells, {arts.Count} skill names");
// icons from SB_Icon atlases
var need = outRows.Select(x=>x.icon).ToHashSet();
var lay = BND4.Read(bhd.GetFile("/menu/hi/01_common.sblytbnd.dcx"));
var tpf = TPF.Read(bhd.GetFile("/menu/hi/01_common.tpf.dcx"));
var re = new Regex(@"name=""MENU_ItemIcon_(\d+)\.png""\s+x=""(\d+)""\s+y=""(\d+)""\s+width=""(\d+)""\s+height=""(\d+)""");
var icons = new SortedDictionary<int, byte[]>(); var dec = new BcDecoder(); var sizes = new SortedDictionary<string,int>();
foreach (var f in lay.Files) {
  var xml = Encoding.UTF8.GetString(f.Bytes); var ms = re.Matches(xml).Where(m => need.Contains(int.Parse(m.Groups[1].Value)) && !icons.ContainsKey(int.Parse(m.Groups[1].Value))).ToList();
  if (ms.Count == 0) continue;
  var texName = Path.GetFileNameWithoutExtension(f.Name);
  var tex = tpf.Textures.FirstOrDefault(t => t.Name == texName); if (tex == null) { Console.WriteLine("no texture " + texName); continue; }
  var dds = DdsFile.Load(new MemoryStream(tex.Bytes)); var pix = dec.Decode(dds);
  int W = (int)dds.header.dwWidth, H = (int)dds.header.dwHeight;
  using var atlas = Image.LoadPixelData<Rgba32>(MemoryMarshal(pix), W, H);
  foreach (var m in ms) {
    int id = int.Parse(m.Groups[1].Value), x = int.Parse(m.Groups[2].Value), y = int.Parse(m.Groups[3].Value), w = int.Parse(m.Groups[4].Value), h = int.Parse(m.Groups[5].Value);
    sizes[$"{w}x{h}"] = sizes.GetValueOrDefault($"{w}x{h}") + 1;
    using var ic = atlas.Clone(c => c.Crop(new Rectangle(x, y, w, h)).Resize(px, px, KnownResamplers.Lanczos3));
    var ms2 = new MemoryStream(); ic.SaveAsPng(ms2, new SixLabors.ImageSharp.Formats.Png.PngEncoder{ CompressionLevel = SixLabors.ImageSharp.Formats.Png.PngCompressionLevel.BestCompression, ColorType = args.Length > 3 ? SixLabors.ImageSharp.Formats.Png.PngColorType.Palette : SixLabors.ImageSharp.Formats.Png.PngColorType.RgbWithAlpha, Quantizer = new SixLabors.ImageSharp.Processing.Processors.Quantization.WuQuantizer() }); icons[id] = ms2.ToArray();
  }
  Console.WriteLine($"{texName}: {ms.Count}");
}
Console.WriteLine("icon sizes: " + string.Join(", ", sizes.Select(k=>$"{k.Key}={k.Value}")));
Console.WriteLine($"icons {icons.Count}/{need.Count}; missing: {string.Join(", ", outRows.Where(x=>!icons.ContainsKey(x.icon)).Select(x=>x.name).Take(30))}");
// pack: "BHIC" u32 count, then (u32 id, u32 len) x count, then data
using (var o = new BinaryWriter(File.Create(Path.Combine(outDir,"icons.bin")))) {
  o.Write(Encoding.ASCII.GetBytes("BHIC")); o.Write(icons.Count);
  foreach (var kv in icons) { o.Write(kv.Key); o.Write(kv.Value.Length); }
  foreach (var kv in icons) o.Write(kv.Value);
}
Console.WriteLine($"icons.bin {new FileInfo(Path.Combine(outDir,"icons.bin")).Length/1024} KB");
static Rgba32[] MemoryMarshal(BCnEncoder.Shared.ColorRgba32[] p) => p.Select(c => new Rgba32(c.r, c.g, c.b, c.a)).ToArray();

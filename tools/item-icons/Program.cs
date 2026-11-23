using System; using System.Collections.Generic; using System.IO; using System.Linq; using System.Reflection; using System.Text; using System.Text.RegularExpressions; using System.Threading;
using SoulsFormats; using FSParam; using BCnEncoder.Decoder; using BCnEncoder.Shared.ImageFiles;
using SixLabors.ImageSharp; using SixLabors.ImageSharp.PixelFormats; using SixLabors.ImageSharp.Processing;
// Builds the Classes page's item data from the player's own game files (vanilla params and icons).
// args: gameDir outDir iconPx [pal]
//   items.tsv  name<TAB>kind<TAB>smithing|somber|<empty><TAB>iconId  (weapons, then spells)
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

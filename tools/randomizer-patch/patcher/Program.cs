using Mono.Cecil; using Mono.Cecil.Cil; using System; using System.IO; using System.Linq;
// args: in.dll out.dll [names.tsv (id<TAB>name) | -] [--stones N]
//   names.tsv   adds missing weapon names to Project.Settings.Equipment.EquipmentNameList
//   --stones N  sets how many smithing stones each upgrade level costs (S2 ships with 2)
var rp = new DefaultAssemblyResolver(); rp.AddSearchDirectory(Path.GetDirectoryName(Path.GetFullPath(args[0])));
var asm = AssemblyDefinition.ReadAssembly(args[0], new ReaderParameters { AssemblyResolver = rp });
string names = args.Length > 2 && args[2] != "-" && !args[2].StartsWith("--") ? args[2] : null;
int si = Array.IndexOf(args, "--stones");
int? stones = si > 0 ? int.Parse(args[si + 1]) : null;

if (names != null) {
  var t = asm.MainModule.GetType("Project.Settings.Equipment");
  var cctor = t.Methods.Single(m => m.Name == ".cctor");
  var il = cctor.Body.GetILProcessor();
  var store = cctor.Body.Instructions.Single(i => i.OpCode == OpCodes.Stsfld && ((FieldReference)i.Operand).Name == "EquipmentNameList");
  var add = (MethodReference)store.Previous.Operand;
  if (store.Previous.OpCode != OpCodes.Callvirt || add.Name != "Add") throw new Exception("unexpected IL before stsfld: " + store.Previous);
  // Existing keys, read from the IL (ldc.i4 id; ldstr name; callvirt Add)
  var existing = cctor.Body.Instructions.Where(i => i.OpCode == OpCodes.Callvirt && i.Operand is MethodReference m && m.FullName == add.FullName && i.Previous.Previous.OpCode == OpCodes.Ldc_I4).Select(i => (int)i.Previous.Previous.Operand).ToHashSet();
  int n = 0;
  foreach (var line in File.ReadAllLines(names)) {
    var p = line.Split('\t'); int id = int.Parse(p[0]);
    if (existing.Contains(id)) continue;
    il.InsertBefore(store, il.Create(OpCodes.Dup));
    il.InsertBefore(store, il.Create(OpCodes.Ldc_I4, id));
    il.InsertBefore(store, il.Create(OpCodes.Ldstr, p[1]));
    il.InsertBefore(store, il.Create(OpCodes.Callvirt, add));
    n++;
  }
  Console.WriteLine($"names: existing {existing.Count}, added {n}");
}

if (stones is int k) {
  // patchSmithingStones: `sbyte b = 2; foreach row: if (itemNum01 > 1 && materialCate01 == 4 &&
  // 10100 <= materialId01 < 10110) itemNum01 = b;` (regular Smithing Stones only; somber ones
  // already cost 1). The constant is the only small int stored straight into a local.
  if (k < 1 || k > 8) throw new Exception("--stones must be 1..8");
  var m = asm.MainModule.GetType("Project.Tasks.Randomizer").Methods.Single(x => x.Name == "patchSmithingStones");
  var body = m.Body.Instructions;
  if (!body.Any(i => i.OpCode == OpCodes.Ldc_I4 && (int)i.Operand == 10100)) throw new Exception("patchSmithingStones doesn't look like the S2 one (no 10100)");
  var consts = body.Where(i => IsSmallConst(i, out _) && IsStloc(i.Next)).ToList();
  if (consts.Count != 1) throw new Exception($"expected one stone-count constant, found {consts.Count}");
  IsSmallConst(consts[0], out int old);
  var il = m.Body.GetILProcessor();
  il.Replace(consts[0], il.Create(OpCodes.Ldc_I4_S, (sbyte)k));
  Console.WriteLine($"stones per level: {old} -> {k}");
}

asm.Write(args[1]);

static bool IsSmallConst(Instruction i, out int v) {
  v = 0;
  if (i.OpCode == OpCodes.Ldc_I4_1) v = 1; else if (i.OpCode == OpCodes.Ldc_I4_2) v = 2; else if (i.OpCode == OpCodes.Ldc_I4_3) v = 3;
  else if (i.OpCode == OpCodes.Ldc_I4_4) v = 4; else if (i.OpCode == OpCodes.Ldc_I4_5) v = 5; else if (i.OpCode == OpCodes.Ldc_I4_6) v = 6;
  else if (i.OpCode == OpCodes.Ldc_I4_7) v = 7; else if (i.OpCode == OpCodes.Ldc_I4_8) v = 8;
  else if (i.OpCode == OpCodes.Ldc_I4_S) v = (sbyte)i.Operand; else return false;
  return true;
}
static bool IsStloc(Instruction i) => i != null && (i.OpCode == OpCodes.Stloc || i.OpCode == OpCodes.Stloc_S || i.OpCode == OpCodes.Stloc_0 || i.OpCode == OpCodes.Stloc_1 || i.OpCode == OpCodes.Stloc_2 || i.OpCode == OpCodes.Stloc_3);

using Mono.Cecil; using Mono.Cecil.Cil; using System; using System.IO; using System.Linq;
// args: in.dll out.dll names.tsv (id<TAB>name)
var rp = new DefaultAssemblyResolver(); rp.AddSearchDirectory(Path.GetDirectoryName(Path.GetFullPath(args[0])));
var asm = AssemblyDefinition.ReadAssembly(args[0], new ReaderParameters { AssemblyResolver = rp });
var t = asm.MainModule.GetType("Project.Settings.Equipment");
var cctor = t.Methods.Single(m => m.Name == ".cctor");
var il = cctor.Body.GetILProcessor();
var store = cctor.Body.Instructions.Single(i => i.OpCode == OpCodes.Stsfld && ((FieldReference)i.Operand).Name == "EquipmentNameList");
var add = (MethodReference)store.Previous.Operand;
if (store.Previous.OpCode != OpCodes.Callvirt || add.Name != "Add") throw new Exception("unexpected IL before stsfld: " + store.Previous);
// Existing keys, read from the IL (ldc.i4 id; ldstr name; callvirt Add)
var existing = cctor.Body.Instructions.Where(i => i.OpCode == OpCodes.Callvirt && i.Operand is MethodReference m && m.FullName == add.FullName && i.Previous.Previous.OpCode == OpCodes.Ldc_I4).Select(i => (int)i.Previous.Previous.Operand).ToHashSet();
int n = 0;
foreach (var line in File.ReadAllLines(args[2])) {
  var p = line.Split('\t'); int id = int.Parse(p[0]);
  if (existing.Contains(id)) continue;
  il.InsertBefore(store, il.Create(OpCodes.Dup));
  il.InsertBefore(store, il.Create(OpCodes.Ldc_I4, id));
  il.InsertBefore(store, il.Create(OpCodes.Ldstr, p[1]));
  il.InsertBefore(store, il.Create(OpCodes.Callvirt, add));
  n++;
}
asm.Write(args[1]);
Console.WriteLine($"existing {existing.Count}, added {n}");

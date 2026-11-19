# Randomizer name-table patch

`S2_Style_1.0.exe` names each starting class's weapons from a table built into its code
(`Project.Settings.Equipment.EquipmentNameList`) instead of the game's text. When a game update
adds weapons, seeds that hand one out crash with `KeyNotFoundException` in `addDescriptionString`
("This seed doesn't work" in the hub). The table is only used for that description text, so adding
names never changes how a seed randomizes.

## When Bingo Hub warns about it

Setup shows "The Randomizer doesn't know some new weapons" (`src/randocheck.rs` compares the
table with the Randomizer's copy of the game text) and writes the list to
`Data\randomizer-missing-names.tsv`. Then:

```
.\patch.ps1
```

It patches the installed exe, keeps the previous one in `Data\Randomizer backup\<date>`, and
installs the new one. Release a hub version afterwards so everyone has the same build (the race
code includes the Randomizer's fingerprint). Needs Python 3 and the .NET SDK.

## History

- 2026-11-19, game regulation 2.7.2: all 115 named weapons missing from the table added. Seeds
  crashing went from 46/100 to 0/100; seed 1712's spoiler log is byte-identical to the unpatched
  build. The unpatched exe is `Data\Randomizer backup\S2_Style_1.0.exe` (SHA-256 785FD8DC...309E).

## The pieces

- `unbundle.py` pulls `Randomizer.dll` out of the .NET single-file exe.
- `patcher/` (Mono.Cecil) adds `dup; ldc.i4 id; ldstr name; callvirt Add` entries to the table's
  static constructor, skipping IDs it already has.
- `repack.py` appends the new dll to the exe and points the bundle manifest at it; everything else
  in the bundle is untouched.
- `missing/` and `seedtest/` run the Randomizer's own code against the game for testing:
  `seedtest` with `1712` compares against a known spoiler log, `rate:0:10` counts crashing seeds
  (one process per batch of 10; the param library's finalizer crashes after many runs in one
  process). Build them, then copy `oo2core_6_win64.dll` from the game folder and the Randomizer's
  `Resources\Params` and `Cache\msg` folders next to the built dll, and run them from that folder.

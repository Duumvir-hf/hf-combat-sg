# hf_combat_sg — strike-group battles get their own level

A HighFleet mod (Steam **1.163**) that makes a **fleet-vs-fleet battle against a strike group** load
`Data/Levels/Combat_SG.seria` instead of `Data/Levels/Combat.seria`. Every other battle keeps the
stock level.

`Combat_SG.seria` does not exist in vanilla: the mod ships a copy of `Combat.seria` to start from, and
the player edits it (arena size, background, extra nodes) to make strike-group fights feel different.

## How the game decides that you are fighting a strike group

HighFleet already makes the distinction, for its music. `selectMusic_` (`0x140253a50`) walks the
**enemy side list** of the engagement and, when any fleet there has the behaviour word `+0x3a8 == 5`,
it plays `Tanc_a_lelek`; otherwise it picks one of the garrison tracks (`Sahara_test`, `Mogott_short`,
`Folyo_merfold`). That is exactly why strike-group fights sound different.

This mod reads the same list at the same moment, so its idea of "strike group" is the game's own and
cannot drift from the music.

Which list is which: `UpdateStrategicFleet` always fills **A (`0x143a158e0`) with the player side**
and **B (`0x143a159c0`) with the AI side** (both branches of the encounter code at `0x1401142cb` /
`0x140114363` end that way, and the nearby-fleet loop that follows only appends fleets with a negative
side value to B). So scanning B asks "is the *enemy* a strike group", exactly like the music.

```
140253a9f  mov r9,  [143a159c8]      ; enemy side vector end
140253ab0  mov r8,  [143a159c0]      ; enemy side vector begin
140253ad5  mov rax, [r8+8]           ; Link -> the fleet object
140253ade  cmp dword [rax+3a8], 5    ; strike group?
140253ae5  jz  -> "Tanc_a_lelek"
```

## Where the level name is set

There is exactly one `Combat.seria` string in the whole executable (`0x14035fa00`). The
fleet-vs-fleet branch of `UpdateStrategicFleet` (`0x140111ff0`) loads it here:

```
140114668  mov rdx, [143a159c0]      ; enemy side, first object
140114673  mov rcx, [143a158e0]      ; our side, first object
14011467e  call 0x1401bbeb0          ; approach angle / camera setup
140114683  xor r8d, r8d
140114686  xor edx, edx
140114688  lea rcx, [14035fa00]      ; <-- "Combat.seria"  (patched by this mod)
14011468f  call 0x14025afd0          ; SetLevelSeriaToLoad_(name, false, false)
```

The hook replaces the seven bytes of that `lea` with a jump to a trampoline that:

1. saves the volatile registers and calls this mod's `level_name()`,
2. writes the returned pointer into RCX — **always**, because the `lea` it replaced never runs and
   the caller's RCX is dead garbage at that point,
3. jumps back to `0x14011468f`, so the original `call SetLevelSeriaToLoad_` runs unchanged.

`level_name()` therefore never returns 0: for an ordinary battle it returns the stock
`"Combat.seria"` literal, decoded from the hooked instruction's own rel32 at install time
(`0x14035fa00`), so the vanilla path is byte-for-byte what the `lea` would have produced.

> **1.0.1 crashed on ordinary battles** exactly here: it returned 0 and left RCX alone, so
> `SetLevelSeriaToLoad_` strlen'd the leftover from the previous call (`rcx=0xe080faa84f270000`,
> fault at `0x14025b008`, frame `0x140114694`). The trampoline no longer has a conditional path.

Nothing else is modified: the scene itself is chosen from the `LVL_SCRIPT` key **inside** the seria
file (`SCR_COMBAT`), not from the file name, so a copy of `Combat.seria` runs the identical battle,
pre-combat screen included. `SetSkirmishFlagsFromLevelName` only knows the `*Skirmish*` names, so an
unknown name leaves the campaign flags at "not a skirmish".

## Install

1. `Modloader/mods/hf_combat_sg.dll` (this build).
2. `Modloader/config/combat_sg.json` (optional; the defaults are written on first run).
3. `Data/Levels/Combat_SG.seria` — copy the shipped file, or copy vanilla
   `Data/Levels/Combat.seria` and edit it.

Requires [Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader) (bass.dll proxy) and the
Steam 1.163 build. GOG 1.163 has different addresses and is refused (`version` handshake).

## The level file

`Combat_SG.seria` is an ordinary level shell. Vanilla `Combat.seria` (401 bytes) is:

```
{
m_classname=LEVEL
m_code=3
LVL_SCRIPT=SCR_COMBAT
LVL_CITY_RES_HYDROGEN=0
LVL_WIND_FORCE=3
LVL_BACKGROUND=background_mid_01
LVL_HIGH.x=3000
LVL_HIGH.y=2000
LVL_LOW.x=-3000
LVL_LOW.y=-2000
m_world=7
{
m_classname=Node
m_code=7
m_id=1
m_state=4
m_name=LEVEL_ROOT
physical_root=true
}
m_world_trade=7
{
m_classname=Node
m_code=7
m_id=2
m_state=4
m_name=LEVEL_TRADE_ROOT
}
}
```

Keep `LVL_SCRIPT=SCR_COMBAT` — that is what starts the tactical battle, the pre-combat screen and the
return-to-map logic. Safe things to change for strike-group fights: `LVL_HIGH`/`LVL_LOW` (the arena
box), `LVL_BACKGROUND`, `LVL_WIND_FORCE`, and the extra `Node` entries. Editing the level's landscape
keys is the same as any other level; the battle terrain generator only honours
`LVL_LANDSCAPE_*` keys in other levels (see the notes' `battle-terrain.md`).

If the file is missing the mod does **not** switch: `App_ReloadSeria` (`0x140240b90`) treats a failed
`Data/Levels/<name>` load as fatal (`ERR: reload_sg failed`, then it asks the game to exit). A warning
is logged instead and `Combat.seria` is used.

## Config — `Modloader/config/combat_sg.json`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Master switch. `false` = no patch installed. |
| `level` | `"Combat_SG.seria"` | Level to load for strike-group battles. A bare `*.seria` name in `Data/Levels/`. |
| `verbose_logging` | `false` | Also log the enemy fleets' states for ordinary battles and the link count. |

## What the log shows

`Modloader/logs/<timestamp>.log`, filtered per DLL in `logging.json`:

```
hf_combat_sg v1.0.2: level "Combat_SG.seria", image base 0x140000000, version Steam 1.163
hf_combat_sg: stock level name literal at 0x14035fa00
hf_combat_sg v1.0.2: hooked at 0x140114688; strike-group battles load Combat_SG.seria
hf_combat_sg: strike group engaged, loading Combat_SG.seria instead of Combat.seria
```

With `verbose_logging` on, every fleet-vs-fleet load also logs the enemy side's behaviour words, e.g.
`3 enemy fleet(s), states [5, 2, -1] -> strike group` (`-1` = a link whose object was not readable),
and an unusable/empty side list logs `enemy side list unusable (...)`. That line is the first thing
to look at if a fight you expected to switch did not.

## Build

```
cargo build --release          # Windows MSVC, x86_64-pc-windows-msvc
```

Output: `target/release/hf_combat_sg.dll`. The crate depends on `highfleet-mod-api` (Modloader v4.0.0
logging SDK) and `patchy-rs` v0.1.0 (trampolines).

## Test plan

1. Install, start a campaign, check the log for `hooked at 0x140114688`.
2. Let a strike group catch you (or engage one): the loading should end in the level from
   `Combat_SG.seria`. If you only changed the background, you see it immediately.
3. Engage a garrison/convoy: `Combat.seria` as before.
4. Delete/rename `Combat_SG.seria` and repeat step 2: the log warns and the vanilla level loads.
5. Shipworks test fight (`EDC_TEST_FIGHT`) is not affected; it loads `Combat.seria` from its own call
   site, which this mod does not touch.

## Changelog

* **1.0.2** — fixed the crash on ordinary (non-strike-group) battles: the trampoline returned 0 and
  left RCX as the caller left it, so `SetLevelSeriaToLoad_` strlen'd a wild pointer. It now always
  writes RCX, and the stock `"Combat.seria"` pointer is decoded from the hooked instruction instead
  of being hard-coded. Unit test asserts the trampoline has no conditional path and that the decoded
  pointer is `0x14035fa00`.
* **1.0.1** — fixed the image-base constant (`0x140_0000_000` was one hex digit too long, so the mod
  refused to start with "unexpected image base"); the constant now has a compile-time assert. The
  startup line prints the version, so the log shows which build is running.
* **1.0.0** — first build.

## Evidence

All addresses are Steam 1.163, image base `0x140000000`; the hook bytes were re-checked against the
unpacked executable on disk (`0x114688`: `48 8d 0d 71 b3 24 00 e8 3c 69 14 00`).

## Licence

MIT © 2026 Duumvir-hf. Not affiliated with Konstantin Koshutin or MicroProse.

# Changelog

Every build, newest first. Addresses are Steam 1.163, image base `0x140000000`.

## 1.0.2 — 2026-11-14

Fixed the crash on ordinary (non-strike-group) battles: the trampoline returned 0 and left RCX as
the caller left it, so `SetLevelSeriaToLoad_` strlen'd a wild pointer. It now always writes RCX, and
the stock `"Combat.seria"` pointer is decoded from the hooked instruction instead of being
hard-coded. Unit test asserts the trampoline has no conditional path and that the decoded pointer is
`0x14035fa00`.

This is the released build.

```
hf_combat_sg.dll               EDC141CE15DE98E67232773644EC1704EBDD299E2E2116D2B4466705EF39BC2E
combat_sg.json                 70727626D4BDC31B282069FD6254BDEBE1372D343B747F3E5BC29D0135CF2362
Combat_SG.seria                25CD72AC390B183A8F48326F0481DD7CD4457821919AAB1A0DABB277062273DF
hf_combat_sg-v1.0.2.zip        0461427C2632A94CE59A4B37AB9DBA4A3AB326464C91DD03C50198D519779A38
```

## 1.0.1

Fixed the image-base constant (`0x140_0000_000` was one hex digit too long, so the mod refused to
start with "unexpected image base"); the constant now has a compile-time assert. The startup line
prints the version, so the log shows which build is running.

## 1.0.0

First build. Fleet-vs-fleet battles whose enemy side is a strike group load `Data/Levels/Combat_SG.seria`
instead of `Data/Levels/Combat.seria`; every other battle keeps the stock level.

# License Analysis: Recoil-Rust, BAR, and Recoil Engine

## Our Project (recoil-rust)

Our engine code is original work. We reference BAR data files during development.

---

## Recoil/Spring Engine

| | |
|---|---|
| **License** | GPL v2 or later |
| **Source** | https://github.com/beyond-all-reason/spring |
| **Allows** | Commercial use, modification, redistribution |
| **Requires** | Source disclosure, same license (GPL) for derivatives, license notice |
| **Prohibits** | Proprietary distribution of derivative engine code |

**Impact on us:** Our engine is a clean-room reimplementation, not a fork. We do NOT use any Spring/Pierce engine source code. **No GPL obligation applies** unless we copy engine code.

---

## BAR Game Code (Lua scripts, unit definitions)

| | |
|---|---|
| **License** | GPL v2 |
| **Source** | https://github.com/beyond-all-reason/Beyond-All-Reason |
| **Allows** | Commercial use, modification, redistribution |
| **Requires** | Source disclosure, same license (GPL) for derivatives |

**Impact on us:** We parse BAR Lua files to extract numeric stats (health, speed, cost, buildoptions). Reading data values from GPL code and using them in our own code is generally permissible — we are not copying or distributing the Lua source. However, shipping the Lua files themselves would require GPL compliance.

---

## BAR Art Assets — MIXED LICENSES (HIGH RISK)

### Models & Textures

| Asset | Author(s) | License | Commercial | Derivatives | Redistribution |
|-------|-----------|---------|------------|-------------|----------------|
| Cremuss models | Cremuss | **CC-BY-SA 4.0** | Yes | Yes (share-alike) | Yes with attribution |
| Arm textures (`Arm_*.dds`) | Cremuss | **CC-BY-SA 4.0** | Yes | Yes (share-alike) | Yes with attribution |
| All other models | FireStorm, Beherith, Mr Bob, KaiserJ, PtaQ, Flaka, Floris | **CC-BY-NC-ND 4.0** | **NO** | **NO** | Only with attribution, non-commercial, no derivatives |
| Cor textures (`Cor_*.dds`) | Beherith | **CC-BY-NC-ND 4.0** | **NO** | **NO** | Only with attribution, non-commercial |
| Decal textures (`*_aoplane*`) | Beherith | **CC-BY-NC-ND 4.0** | **NO** | **NO** | Non-commercial only |
| Animations (all) | Beherith | **CC-BY-NC-ND 4.0** | **NO** | **NO** | Non-commercial only |

### Icons & UI

| Asset | Author(s) | License |
|-------|-----------|---------|
| Unit icons (`unitpics/`) | IceXuick, Floris | **CC-BY-NC-ND 4.0** |
| Map icons | IceXuick, Floris, PtaQ | **CC-BY-NC-ND 4.0** (all rights reserved) |
| Cursors | IceXuick | **All rights reserved** |
| UI bitmaps (`bitmaps/ui/`) | IceXuick | **All rights reserved** |
| Load pictures | Rubus | **All rights reserved** |

### Audio

| Asset | Author(s) | License |
|-------|-----------|---------|
| Sound effects (post-2019) | IceXuick | **All rights reserved** |
| Music (original soundtrack) | Multiple artists (see license_music.txt) | **CC-BY-NC-ND 4.0** (all rights reserved) |
| One sound (AllyRequest.wav) | Inimitible_Wolf | **CC0 (public domain)** |

### Projectile/Effect Textures

| Asset | Author(s) | License |
|-------|-----------|---------|
| All projectile textures | IceXuick | **All rights reserved** |
| Smoke textures | IceXuick | **All rights reserved** |
| Atmospheric textures | IceXuick | **All rights reserved** |

---

## What We CAN Do

### Clearly Safe
- **Write our own engine code** — no GPL obligation (clean-room implementation)
- **Read BAR Lua files to extract numeric values** (stats, costs, buildoptions) — facts/data are not copyrightable
- **Use Cremuss models and Arm textures** (CC-BY-SA 4.0) — must attribute Cremuss, and derivative works must be CC-BY-SA
- **Use BAR Lua file structure as reference** for our own parser — parsing a format is not copying
- **Create our own original assets** — models, textures, sounds, icons

### With Conditions
- **Use Cremuss CC-BY-SA models commercially** — must attribute, must share derivatives under same license
- **Distribute BAR Lua code** — must comply with GPL v2 (include source, license notice)

---

## What We CANNOT Do

### Without Permission
- **Use CC-BY-NC-ND models/textures** in any commercial product
- **Modify CC-BY-NC-ND models** and redistribute (no derivatives clause)
- **Use CC-BY-NC-ND models** in a different game (license explicitly states: "not permitted... including it... in any other game")
- **Use "all rights reserved" assets** (sounds, icons, UI, cursors, bitmaps) in any way without explicit permission
- **Ship BAR's unit icons** (`unitpics/`) — CC-BY-NC-ND, explicitly for BAR only
- **Ship BAR's sound effects** — all rights reserved
- **Ship BAR's music** — all rights reserved / CC-BY-NC-ND

---

## Potential Violations by Severity

### CRITICAL (must fix before any release)
| # | Violation | Current Status | Fix |
|---|-----------|---------------|-----|
| 1 | **Shipping CC-BY-NC-ND models** (.s3o files from `objects3d/`) | Currently loading `armpw.s3o` in main.rs | Must create original models or license from authors |
| 2 | **Shipping CC-BY-NC-ND textures** (`Cor_*.dds`, decals) | Referenced in Jira RR-100 | Must create original textures |
| 3 | **Shipping BAR unit icons** (`unitpics/*.dds`) | Referenced in Jira RR-101 | Must create original icons or use CC-BY-SA only |
| 4 | **Shipping BAR sounds** | Not yet integrated but planned (RR-97) | Must create original sounds |

### HIGH (legal risk if distributed)
| # | Violation | Current Status | Fix |
|---|-----------|---------------|-----|
| 5 | **Using CC-BY-NC-ND assets in a commercial game** | Dev-only currently | Ensure NC-ND assets never ship in releases |
| 6 | **Distributing BAR Lua files without GPL notice** | We parse but don't ship them | Keep parsing at runtime; don't bundle |
| 7 | **Modifying CC-BY-NC-ND models** (scaling, converting) | Scaling in main.rs line 501-512 | Even transformation may violate ND clause |

### MEDIUM (attribution issues)
| # | Violation | Current Status | Fix |
|---|-----------|---------------|-----|
| 8 | **Using Cremuss CC-BY-SA models without attribution** | No attribution in game or docs | Add CREDITS.md with Cremuss attribution |
| 9 | **Using Cremuss models without SA compliance** | Not sharing derivatives under CC-BY-SA | Ensure any modified Cremuss assets stay CC-BY-SA |

### LOW (development-only, not shipped)
| # | Violation | Current Status | Fix |
|---|-----------|---------------|-----|
| 10 | **Loading BAR assets during development** | Active — models, Lua files | Acceptable for dev; don't distribute |
| 11 | **Extracting stats from GPL Lua files** | Active — parser reads numeric values | Generally safe (data/facts not copyrightable) |
| 12 | **Referencing BAR file formats** | S3O loader, Lua parser | Format implementations are clean-room; safe |

---

## Recommended Approach

1. **Development phase**: Use BAR assets freely for testing (don't distribute builds)
2. **Pre-release**: Replace ALL BAR assets with originals (Epic RR-55 covers this)
3. **Arm faction models by Cremuss**: May use if attributed and shared under CC-BY-SA
4. **Everything else from BAR**: Must be replaced with original assets before release
5. **Numeric data** (unit stats, costs, build times): Safe to use as gameplay reference — balance your own values
6. **File formats** (S3O, Lua table structure): Safe — format parsing is not copying

---

## Key License Texts

- **CC-BY-SA 4.0**: https://creativecommons.org/licenses/by-sa/4.0/
- **CC-BY-NC-ND 4.0**: https://creativecommons.org/licenses/by-nc-nd/4.0/
- **GPL v2**: https://www.gnu.org/licenses/old-licenses/gpl-2.0.html

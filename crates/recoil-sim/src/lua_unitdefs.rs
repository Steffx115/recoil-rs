//! BAR (Beyond All Reason) Lua unit definition parser.
//!
//! Extracts unit stats from BAR's Lua table format using simple text parsing —
//! no full Lua interpreter required.  The Lua files are essentially nested
//! key-value tables with numeric and string literals.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::unit_defs::{UnitDef, UnitDefRegistry, WeaponDefData};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Spring engine runs at 30 sim frames per second.
const SPRING_FPS: f64 = 30.0;

/// Spring turnrate is in centidegrees per frame.  To convert to radians:
/// `turnrate * PI / 18000`.
const CENTIDEG_TO_RAD: f64 = std::f64::consts::PI / 18000.0;

/// Spatial scale factor: Spring maps are ~8192 elmos, our maps are ~1024 world
/// units.  All spatial quantities (speed, range, sight, collision radius) from
/// BAR Lua need division by this factor.
const SPRING_ELMO_SCALE: f64 = 8.0;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a single BAR Lua unit definition string into a [`UnitDef`].
pub fn parse_bar_unitdef(lua_source: &str) -> Result<UnitDef> {
    let table = parse_lua_table(lua_source)?;

    // The outer table has one key: the unit name.
    let (unit_name, unit_table) = table
        .sub_tables
        .iter()
        .next()
        .with_context(|| "No unit table found in Lua source")?;

    build_unitdef(unit_name, unit_table)
}

/// Load a single BAR Lua unit definition file.
pub fn load_bar_unitdef(path: &Path) -> Result<UnitDef> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    parse_bar_unitdef(&source).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Load all `.lua` files from a directory and all subdirectories.
pub fn load_bar_unitdefs_recursive(path: &Path) -> Result<UnitDefRegistry> {
    let mut registry = UnitDefRegistry::new();
    walk_directory_recursive(path, &mut registry)?;
    Ok(registry)
}

fn walk_directory_recursive(path: &Path, registry: &mut UnitDefRegistry) -> Result<()> {
    let entries = std::fs::read_dir(path)
        .with_context(|| format!("Failed to read directory: {}", path.display()))?;
    for entry in entries.flatten() {
        let file_path = entry.path();
        if file_path.is_dir() {
            let _ = walk_directory_recursive(&file_path, registry);
        } else if file_path.extension().and_then(|e| e.to_str()) == Some("lua") {
            match load_bar_unitdef(&file_path) {
                Ok(def) => registry.register(def),
                Err(e) => eprintln!("Skipping {}: {e:#}", file_path.display()),
            }
        }
    }
    Ok(())
}

/// Load all `.lua` files from a directory into a [`UnitDefRegistry`].
pub fn load_bar_unitdefs_directory(path: &Path) -> Result<UnitDefRegistry> {
    let mut registry = UnitDefRegistry::new();
    let entries = std::fs::read_dir(path)
        .with_context(|| format!("Failed to read directory: {}", path.display()))?;

    for entry in entries {
        let entry = entry?;
        let file_path = entry.path();
        if file_path.extension().and_then(|e| e.to_str()) == Some("lua") {
            match load_bar_unitdef(&file_path) {
                Ok(def) => {
                    registry.register(def);
                }
                Err(e) => {
                    // Log but continue — some files may not be unit defs.
                    eprintln!("Skipping {}: {e:#}", file_path.display());
                }
            }
        }
    }

    Ok(registry)
}

// ---------------------------------------------------------------------------
// Parsed Lua table representation
// ---------------------------------------------------------------------------

/// A simplified representation of a Lua table extracted via text parsing.
#[derive(Debug, Default)]
struct LuaTable {
    /// Flat key-value pairs (string or numeric values stored as strings).
    values: BTreeMap<String, String>,
    /// Nested sub-tables keyed by name.
    sub_tables: BTreeMap<String, LuaTable>,
    /// Array entries keyed by integer index (from `[N] = { ... }` syntax).
    array_entries: BTreeMap<u32, LuaTable>,
    /// Array entries with string/ident values (from `[N] = "string"` syntax).
    array_string_entries: BTreeMap<u32, String>,
}

// ---------------------------------------------------------------------------
// Lua table parser
// ---------------------------------------------------------------------------

/// Parse a BAR Lua unit definition source into a [`LuaTable`].
fn parse_lua_table(source: &str) -> Result<LuaTable> {
    let lines = preprocess(source);
    let tokens = tokenize(&lines);
    let (table, _) = parse_table_contents(&tokens, 0)?;
    Ok(table)
}

/// Strip comments and normalize whitespace.
fn preprocess(source: &str) -> Vec<String> {
    source
        .lines()
        .map(|line| {
            // Remove block comments (simplified: just line-level).
            // Remove `--` line comments, but not inside strings.
            let mut result = String::new();
            let mut in_string = false;
            let mut string_char = '"';
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if in_string {
                    result.push(chars[i]);
                    if chars[i] == string_char {
                        in_string = false;
                    }
                    i += 1;
                } else if chars[i] == '"' || chars[i] == '\'' {
                    in_string = true;
                    string_char = chars[i];
                    result.push(chars[i]);
                    i += 1;
                } else if i + 1 < chars.len() && chars[i] == '-' && chars[i + 1] == '-' {
                    break; // Rest is comment.
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            result
        })
        .collect()
}

/// Simple token types for our parser.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    OpenBrace,
    CloseBrace,
    Equals,
    Comma,
    OpenBracket,
    CloseBracket,
    Ident(String),
    StringLit(String),
    Number(String),
}

/// Tokenize preprocessed lines into a flat token stream.
fn tokenize(lines: &[String]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let joined = lines.join(" ");
    let chars: Vec<char> = joined.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\r' | '\n' => {
                i += 1;
            }
            '{' => {
                tokens.push(Token::OpenBrace);
                i += 1;
            }
            '}' => {
                tokens.push(Token::CloseBrace);
                i += 1;
            }
            '=' => {
                tokens.push(Token::Equals);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '[' => {
                tokens.push(Token::OpenBracket);
                i += 1;
            }
            ']' => {
                tokens.push(Token::CloseBracket);
                i += 1;
            }
            '"' | '\'' => {
                let quote = chars[i];
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != quote {
                    s.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing quote
                }
                tokens.push(Token::StringLit(s));
            }
            c if c.is_ascii_digit() || c == '.' || c == '-' => {
                // Check if this '-' is actually a negative number (not a comment).
                // A '-' followed by a digit or '.' is a number.
                if c == '-' {
                    if i + 1 < chars.len() && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '.')
                    {
                        let mut num = String::new();
                        num.push(chars[i]);
                        i += 1;
                        while i < chars.len()
                            && (chars[i].is_ascii_digit()
                                || chars[i] == '.'
                                || chars[i] == 'e'
                                || chars[i] == 'E'
                                || chars[i] == '-'
                                || chars[i] == '+')
                        {
                            num.push(chars[i]);
                            i += 1;
                        }
                        tokens.push(Token::Number(num));
                    } else {
                        // Stray '-', skip it.
                        i += 1;
                    }
                } else {
                    let mut num = String::new();
                    while i < chars.len()
                        && (chars[i].is_ascii_digit()
                            || chars[i] == '.'
                            || chars[i] == 'e'
                            || chars[i] == 'E'
                            || chars[i] == '-'
                            || chars[i] == '+')
                    {
                        num.push(chars[i]);
                        i += 1;
                    }
                    tokens.push(Token::Number(num));
                }
            }
            c if c.is_ascii_alphanumeric() || c == '_' => {
                let mut ident = String::new();
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    ident.push(chars[i]);
                    i += 1;
                }
                // Lua keywords we treat as values.
                match ident.as_str() {
                    "true" | "false" | "nil" | "return" => {
                        tokens.push(Token::Ident(ident));
                    }
                    _ => {
                        tokens.push(Token::Ident(ident));
                    }
                }
            }
            _ => {
                i += 1; // skip unknown chars
            }
        }
    }
    tokens
}

/// Parse table contents between braces.  Returns `(LuaTable, next_position)`.
fn parse_table_contents(tokens: &[Token], start: usize) -> Result<(LuaTable, usize)> {
    let mut table = LuaTable::default();
    let mut i = start;

    // Skip `return` keyword if present.
    if i < tokens.len() && tokens[i] == Token::Ident("return".into()) {
        i += 1;
    }

    // Expect opening brace.
    if i < tokens.len() && tokens[i] == Token::OpenBrace {
        i += 1;
    }

    while i < tokens.len() {
        // End of this table.
        if tokens[i] == Token::CloseBrace {
            i += 1;
            // Skip trailing comma.
            if i < tokens.len() && tokens[i] == Token::Comma {
                i += 1;
            }
            return Ok((table, i));
        }

        // Skip commas.
        if tokens[i] == Token::Comma {
            i += 1;
            continue;
        }

        // `[N] = { ... }` — array entry.
        if tokens[i] == Token::OpenBracket {
            i += 1; // skip [
            if let Some(Token::Number(ref n)) = tokens.get(i) {
                let idx: u32 = n.parse().unwrap_or(0);
                i += 1; // skip number
                if i < tokens.len() && tokens[i] == Token::CloseBracket {
                    i += 1; // skip ]
                }
                if i < tokens.len() && tokens[i] == Token::Equals {
                    i += 1; // skip =
                }
                if i < tokens.len() && tokens[i] == Token::OpenBrace {
                    let (sub, next) = parse_table_contents(tokens, i)?;
                    table.array_entries.insert(idx, sub);
                    i = next;
                } else if let Some(tok) = tokens.get(i) {
                    let val = match tok {
                        Token::StringLit(s) => s.clone(),
                        Token::Ident(id) => id.clone(),
                        Token::Number(n) => n.clone(),
                        _ => String::new(),
                    };
                    if !val.is_empty() {
                        table.array_string_entries.insert(idx, val);
                    }
                    i += 1;
                    if i < tokens.len() && tokens[i] == Token::Comma {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            } else {
                // Skip unknown bracket expression.
                i += 1;
            }
            continue;
        }

        // `key = value` or `key = { ... }`
        if let Some(Token::Ident(ref key)) = tokens.get(i).cloned() {
            if i + 1 < tokens.len() && tokens[i + 1] == Token::Equals {
                let key = key.clone();
                i += 2; // skip key and =

                if i < tokens.len() && tokens[i] == Token::OpenBrace {
                    // Nested table.
                    let (sub, next) = parse_table_contents(tokens, i)?;
                    table.sub_tables.insert(key, sub);
                    i = next;
                } else if let Some(tok) = tokens.get(i) {
                    // Flat value.
                    let val = match tok {
                        Token::Number(n) => n.clone(),
                        Token::StringLit(s) => s.clone(),
                        Token::Ident(id) => id.clone(),
                        _ => String::new(),
                    };
                    table.values.insert(key, val);
                    i += 1;
                    // Skip trailing comma.
                    if i < tokens.len() && tokens[i] == Token::Comma {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            } else {
                // Bare identifier (e.g. in a list), skip.
                i += 1;
            }
        } else {
            // Skip unexpected tokens.
            i += 1;
        }
    }

    Ok((table, i))
}

// ---------------------------------------------------------------------------
// UnitDef construction from parsed table
// ---------------------------------------------------------------------------

fn build_unitdef(unit_name: &str, table: &LuaTable) -> Result<UnitDef> {
    let get_f64 = |key: &str| -> f64 {
        table
            .values
            .get(key)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };

    let get_string =
        |key: &str| -> Option<String> { table.values.get(key).cloned().filter(|s| !s.is_empty()) };

    // Generate a stable type id from the unit name.
    let unit_type_id = hash_unit_name(unit_name);

    // Parse weapon defs.
    let weapons = parse_weapons(table);

    // Determine armor class from categories or heuristics.
    let armor_class = guess_armor_class(table);

    // Extract categories if present.
    let categories = Vec::new(); // BAR stores these differently; leave empty for now.

    let max_speed_spring = get_f64("speed");
    let acceleration_spring = get_f64("maxacc");
    let turnrate_spring = get_f64("turnrate");
    let footprint_x = get_f64("footprintx");

    // --- Build options ---
    let can_build_names: Vec<String> = table
        .sub_tables
        .get("buildoptions")
        .map(|bo| bo.array_string_entries.values().cloned().collect())
        .unwrap_or_default();

    // --- Economy fields (BAR-specific names) ---
    let energy_make = get_f64("energymake");
    let energy_upkeep = get_f64("energyupkeep");
    let energy_production_legacy = get_f64("energyproduction");
    let energy_production = if energy_make > 0.0 {
        Some(energy_make)
    } else if energy_upkeep < 0.0 {
        Some(-energy_upkeep)
    } else if energy_production_legacy > 0.0 {
        Some(energy_production_legacy)
    } else {
        None
    };

    let metal_make = get_f64("metalmake");
    let extracts_metal = get_f64("extractsmetal");
    let metal_production_legacy = get_f64("metalproduction");
    let metal_production = if metal_make > 0.0 {
        Some(metal_make)
    } else if extracts_metal > 0.0 {
        Some(extracts_metal * 1000.0)
    } else if metal_production_legacy > 0.0 {
        Some(metal_production_legacy)
    } else {
        None
    };

    let is_builder_flag = get_string("builder")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);
    let workertime = get_f64("workertime");
    let buildpower_legacy = get_f64("buildpower");
    let build_power = if is_builder_flag && workertime > 0.0 {
        Some(workertime)
    } else if buildpower_legacy > 0.0 {
        Some(buildpower_legacy)
    } else if is_builder_flag {
        Some(100.0)
    } else {
        None
    };

    let is_building = max_speed_spring <= 0.0;
    let is_builder = build_power.is_some();

    Ok(UnitDef {
        name: unit_name.to_string(),
        unit_type_id,
        max_health: get_f64("health"),
        armor_class,
        sight_range: get_f64("sightdistance") / SPRING_ELMO_SCALE,
        collision_radius: footprint_x * 8.0 / 2.0 / SPRING_ELMO_SCALE,
        max_speed: max_speed_spring / SPRING_FPS / SPRING_ELMO_SCALE,
        acceleration: acceleration_spring / SPRING_FPS / SPRING_ELMO_SCALE,
        turn_rate: turnrate_spring * CENTIDEG_TO_RAD,
        metal_cost: get_f64("metalcost"),
        energy_cost: get_f64("energycost"),
        build_time: (get_f64("buildtime") / SPRING_FPS) as u32,
        weapons,
        model_path: get_string("objectname"),
        icon_path: None,
        categories,
        can_build: Vec::new(),
        can_build_names,
        build_power,
        metal_production,
        energy_production,
        is_building,
        is_builder,
    })
}

/// Parse weapon definitions from the `weapondefs` and `weapons` sub-tables.
fn parse_weapons(table: &LuaTable) -> Vec<WeaponDefData> {
    let weapondefs = match table.sub_tables.get("weapondefs") {
        Some(wd) => wd,
        None => return Vec::new(),
    };

    // Build a map of weapon name → WeaponDefData.
    let mut weapon_map: BTreeMap<String, WeaponDefData> = BTreeMap::new();
    for (name, wt) in &weapondefs.sub_tables {
        let get_f64 = |key: &str| -> f64 {
            wt.values
                .get(key)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
        };

        let get_string = |key: &str| -> String { wt.values.get(key).cloned().unwrap_or_default() };

        // damage.default is in a nested `damage` subtable.
        let damage = wt
            .sub_tables
            .get("damage")
            .and_then(|dt| dt.values.get("default"))
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);

        let weapon_type = get_string("weapontype");
        let damage_type = map_weapon_type_to_damage_type(&weapon_type);

        weapon_map.insert(
            name.to_lowercase(),
            WeaponDefData {
                name: name.clone(),
                damage,
                damage_type,
                range: get_f64("range") / SPRING_ELMO_SCALE,
                reload_time: (get_f64("reloadtime") * SPRING_FPS) as u32,
                projectile_speed: get_f64("weaponvelocity") / SPRING_FPS / SPRING_ELMO_SCALE,
                area_of_effect: get_f64("areaofeffect") / SPRING_ELMO_SCALE,
            },
        );
    }

    // Order weapons by the `weapons` array table.
    let weapons_order = table.sub_tables.get("weapons");
    if let Some(wo) = weapons_order {
        let mut ordered = Vec::new();
        // Collect array entries in order.
        for entry in wo.array_entries.values() {
            if let Some(def_name) = entry.values.get("def") {
                let key = def_name.to_lowercase();
                if let Some(wd) = weapon_map.remove(&key) {
                    ordered.push(wd);
                }
            }
        }
        // Append any remaining weapons not in the order table.
        for (_, wd) in weapon_map {
            ordered.push(wd);
        }
        ordered
    } else {
        // No ordering — return all weapondefs in alphabetical order.
        weapon_map.into_values().collect()
    }
}

/// Map BAR weapon type strings to our damage type strings.
fn map_weapon_type_to_damage_type(weapon_type: &str) -> String {
    match weapon_type {
        "Cannon" | "LaserCannon" => "Normal".to_string(),
        "MissileLauncher" => "Explosive".to_string(),
        "BeamLaser" | "LightningCannon" => "Laser".to_string(),
        _ => "Normal".to_string(),
    }
}

/// Guess armor class from the unit table.
fn guess_armor_class(table: &LuaTable) -> String {
    // If speed is 0, it's probably a building.
    let speed = table
        .values
        .get("speed")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    if speed <= 0.0 {
        return "Building".to_string();
    }

    // Check footprint — large footprint suggests vehicle.
    let footprint = table
        .values
        .get("footprintx")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0);

    if footprint >= 4.0 {
        "Medium".to_string()
    } else {
        "Light".to_string()
    }
}

/// Generate a stable u32 id from a unit name using FNV-1a hash.
pub fn hash_unit_name(name: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in name.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ARMPW_LUA: &str = r#"
return {
    armpw = {
        health = 370,
        metalcost = 54,
        energycost = 900,
        buildtime = 1650,
        speed = 87,
        maxacc = 0.414,
        turnrate = 1214.40002,
        sightdistance = 429,
        objectname = "Units/ARMPW.s3o",
        footprintx = 2,
        footprintz = 2,
        weapondefs = {
            emg = {
                damage = { default = 9 },
                range = 180,
                reloadtime = 0.3,
                weaponvelocity = 600,
                areaofeffect = 8,
                weapontype = "Cannon",
                burst = 3,
                burstrate = 0.1,
            },
        },
        weapons = {
            [1] = { def = "EMG" },
        },
    },
}
"#;

    #[test]
    fn parse_armpw() {
        let def = parse_bar_unitdef(ARMPW_LUA).unwrap();
        assert_eq!(def.name, "armpw");
        assert_eq!(def.max_health, 370.0);
        assert_eq!(def.metal_cost, 54.0);
        assert_eq!(def.energy_cost, 900.0);
        assert_eq!(def.build_time, (1650.0 / 30.0) as u32); // 55
        assert!((def.max_speed - 87.0 / 30.0 / 8.0).abs() < 0.001);
        assert!((def.acceleration - 0.414 / 30.0 / 8.0).abs() < 0.001);
        assert!((def.turn_rate - 1214.40002 * std::f64::consts::PI / 18000.0).abs() < 0.001);
        assert!((def.sight_range - 429.0 / 8.0).abs() < 0.1);
        assert_eq!(def.model_path, Some("Units/ARMPW.s3o".to_string()));
        assert!((def.collision_radius - 8.0 / 8.0).abs() < 0.001);
        assert_eq!(def.armor_class, "Light");
    }

    #[test]
    fn parse_armpw_weapons() {
        let def = parse_bar_unitdef(ARMPW_LUA).unwrap();
        assert_eq!(def.weapons.len(), 1);

        let w = &def.weapons[0];
        assert_eq!(w.name, "emg");
        assert_eq!(w.damage, 9.0);
        assert_eq!(w.damage_type, "Normal");
        assert!((w.range - 180.0 / 8.0).abs() < 0.1);
        assert_eq!(w.reload_time, (0.3 * 30.0) as u32); // 9
        assert!((w.projectile_speed - 600.0 / 30.0 / 8.0).abs() < 0.001);
        assert!((w.area_of_effect - 8.0 / 8.0).abs() < 0.01);
    }

    #[test]
    fn parse_building_no_weapons() {
        let lua = r#"
return {
    armsolar = {
        health = 300,
        metalcost = 145,
        energycost = 0,
        buildtime = 3600,
        speed = 0,
        sightdistance = 300,
        objectname = "Units/ARMSOLAR.s3o",
        footprintx = 4,
        footprintz = 4,
        energyproduction = 20,
    },
}
"#;
        let def = parse_bar_unitdef(lua).unwrap();
        assert_eq!(def.name, "armsolar");
        assert_eq!(def.max_health, 300.0);
        assert_eq!(def.armor_class, "Building");
        assert!(def.weapons.is_empty());
        assert_eq!(def.energy_production, Some(20.0));
        assert!((def.collision_radius - 2.0).abs() < 0.001); // (4 * 8 / 2) / 8
    }

    #[test]
    fn parse_handles_comments() {
        let lua = r#"
return {
    testunit = {
        health = 100, -- this is health
        -- metalcost = 999, (this line should be ignored)
        metalcost = 50,
        speed = 0,
    },
}
"#;
        let def = parse_bar_unitdef(lua).unwrap();
        assert_eq!(def.max_health, 100.0);
        assert_eq!(def.metal_cost, 50.0);
    }

    #[test]
    fn parse_missing_fields_uses_defaults() {
        let lua = r#"
return {
    minimal = {
        health = 100,
    },
}
"#;
        let def = parse_bar_unitdef(lua).unwrap();
        assert_eq!(def.name, "minimal");
        assert_eq!(def.max_health, 100.0);
        assert_eq!(def.max_speed, 0.0);
        assert_eq!(def.metal_cost, 0.0);
        assert!(def.weapons.is_empty());
        assert!(def.model_path.is_none());
    }

    #[test]
    fn stat_conversions_correct() {
        // Verify the exact conversion factors.
        let speed_spring: f64 = 60.0;
        let speed_ours = speed_spring / 30.0;
        assert!((speed_ours - 2.0).abs() < f64::EPSILON);

        let turnrate_spring: f64 = 18000.0; // 180 degrees/frame in centideg
        let turnrate_ours = turnrate_spring * std::f64::consts::PI / 18000.0;
        assert!((turnrate_ours - std::f64::consts::PI).abs() < 0.001);

        let reload_spring: f64 = 1.0; // 1 second
        let reload_ours = (reload_spring * 30.0) as u32;
        assert_eq!(reload_ours, 30);
    }

    #[test]
    fn hash_unit_name_stable() {
        let h1 = hash_unit_name("armpw");
        let h2 = hash_unit_name("armpw");
        assert_eq!(h1, h2);

        let h3 = hash_unit_name("corsolar");
        assert_ne!(h1, h3);
    }

    #[test]
    fn load_directory_from_temp() {
        let dir = tempfile::tempdir().unwrap();

        // Write two Lua files.
        std::fs::write(dir.path().join("armpw.lua"), ARMPW_LUA).unwrap();

        let solar_lua = r#"
return {
    armsolar = {
        health = 300,
        metalcost = 145,
        energycost = 0,
        buildtime = 3600,
        speed = 0,
        footprintx = 4,
    },
}
"#;
        std::fs::write(dir.path().join("armsolar.lua"), solar_lua).unwrap();

        // Non-lua file should be skipped.
        std::fs::write(dir.path().join("readme.txt"), "ignore me").unwrap();

        let registry = load_bar_unitdefs_directory(dir.path()).unwrap();
        assert_eq!(registry.defs.len(), 2);

        let armpw_id = hash_unit_name("armpw");
        let solar_id = hash_unit_name("armsolar");
        assert!(registry.get(armpw_id).is_some());
        assert!(registry.get(solar_id).is_some());
        assert_eq!(registry.get(armpw_id).unwrap().name, "armpw");
        assert_eq!(registry.get(solar_id).unwrap().name, "armsolar");
    }

    #[test]
    fn parse_multiple_weapons_ordered() {
        let lua = r#"
return {
    armcom = {
        health = 2000,
        speed = 35,
        weapondefs = {
            laser = {
                damage = { default = 75 },
                range = 300,
                reloadtime = 0.4,
                weaponvelocity = 900,
                areaofeffect = 0,
                weapontype = "BeamLaser",
            },
            dgun = {
                damage = { default = 9999 },
                range = 250,
                reloadtime = 2.0,
                weaponvelocity = 400,
                areaofeffect = 96,
                weapontype = "Cannon",
            },
        },
        weapons = {
            [1] = { def = "LASER" },
            [2] = { def = "DGUN" },
        },
    },
}
"#;
        let def = parse_bar_unitdef(lua).unwrap();
        assert_eq!(def.weapons.len(), 2);
        assert_eq!(def.weapons[0].name, "laser");
        assert_eq!(def.weapons[0].damage_type, "Laser");
        assert_eq!(def.weapons[1].name, "dgun");
        assert_eq!(def.weapons[1].damage_type, "Normal");
        assert_eq!(def.weapons[1].damage, 9999.0);
    }
}

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

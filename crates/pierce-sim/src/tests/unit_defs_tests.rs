use super::*;
use std::io::Write;

/// Build a sample [`UnitDef`] for testing.
fn sample_def() -> UnitDef {
    UnitDef {
        name: "Peewee".into(),
        unit_type_id: 1,
        max_health: 200.0,
        armor_class: "Light".into(),
        sight_range: 400.0,
        collision_radius: 12.0,
        max_speed: 2.5,
        acceleration: 0.5,
        turn_rate: 0.1,
        metal_cost: 60.0,
        energy_cost: 800.0,
        build_time: 120,
        weapons: vec![WeaponDefData {
            name: "EMG".into(),
            damage: 10.0,
            damage_type: "Normal".into(),
            range: 250.0,
            reload_time: 15,
            projectile_speed: 8.0,
            area_of_effect: 0.0,
        }],
        model_path: Some("models/peewee.obj".into()),
        icon_path: Some("icons/peewee.png".into()),
        categories: vec!["bot".into(), "t1".into(), "combat".into()],
        can_build: vec![],
        can_build_names: vec![],
        build_power: None,
        metal_production: None,
        energy_production: None,
        is_building: false,
        is_builder: false,
    }
}

fn sample_factory_def() -> UnitDef {
    UnitDef {
        name: "Bot Lab".into(),
        unit_type_id: 10,
        max_health: 2000.0,
        armor_class: "Building".into(),
        sight_range: 300.0,
        collision_radius: 40.0,
        max_speed: 0.0,
        acceleration: 0.0,
        turn_rate: 0.0,
        metal_cost: 500.0,
        energy_cost: 5000.0,
        build_time: 600,
        weapons: vec![],
        model_path: None,
        icon_path: None,
        categories: vec!["building".into(), "factory".into()],
        can_build: vec![1, 2, 3],
        can_build_names: vec![],
        build_power: Some(100.0),
        metal_production: None,
        energy_production: None,
        is_building: true,
        is_builder: true,
    }
}

// -- RON roundtrip ---------------------------------------------------------

#[test]
fn ron_roundtrip() {
    let def = sample_def();
    let pretty = ron::ser::PrettyConfig::default();
    let serialized = ron::ser::to_string_pretty(&def, pretty).unwrap();
    let deserialized: UnitDef = ron::from_str(&serialized).unwrap();

    assert_eq!(deserialized.name, def.name);
    assert_eq!(deserialized.unit_type_id, def.unit_type_id);
    assert_eq!(deserialized.max_health, def.max_health);
    assert_eq!(deserialized.armor_class, def.armor_class);
    assert_eq!(deserialized.weapons.len(), 1);
    assert_eq!(deserialized.weapons[0].name, "EMG");
    assert_eq!(deserialized.categories, def.categories);
    assert_eq!(deserialized.can_build, def.can_build);
    assert_eq!(deserialized.build_power, def.build_power);
    assert_eq!(deserialized.model_path, def.model_path);
}

#[test]
fn ron_roundtrip_factory() {
    let def = sample_factory_def();
    let serialized =
        ron::ser::to_string_pretty(&def, ron::ser::PrettyConfig::default()).unwrap();
    let deserialized: UnitDef = ron::from_str(&serialized).unwrap();

    assert_eq!(deserialized.name, "Bot Lab");
    assert_eq!(deserialized.can_build, vec![1, 2, 3]);
    assert_eq!(deserialized.build_power, Some(100.0));
    assert!(deserialized.weapons.is_empty());
}

// -- Load directory --------------------------------------------------------

#[test]
fn load_directory_multiple_defs() {
    let dir = tempfile::tempdir().unwrap();

    // Write two def files.
    let def1 = sample_def();
    let def2 = sample_factory_def();

    let path1 = dir.path().join("peewee.ron");
    let path2 = dir.path().join("bot_lab.ron");

    UnitDefRegistry::save_def(&def1, &path1).unwrap();
    UnitDefRegistry::save_def(&def2, &path2).unwrap();

    // Also write a non-ron file that should be ignored.
    let mut ignored = std::fs::File::create(dir.path().join("readme.txt")).unwrap();
    writeln!(ignored, "This should be ignored").unwrap();

    let registry = UnitDefRegistry::load_directory(dir.path()).unwrap();
    assert_eq!(registry.defs.len(), 2);
    assert_eq!(registry.get(1).unwrap().name, "Peewee");
    assert_eq!(registry.get(10).unwrap().name, "Bot Lab");
}

// -- Conversion to UnitBlueprint -------------------------------------------

#[test]
fn to_unit_blueprint() {
    let def = sample_def();
    let bp = def.to_unit_blueprint();

    assert_eq!(bp.unit_type_id, 1);
    assert_eq!(bp.metal_cost, SimFloat::from_f64(60.0));
    assert_eq!(bp.energy_cost, SimFloat::from_f64(800.0));
    assert_eq!(bp.build_time, 120);
    assert_eq!(bp.max_health, SimFloat::from_f64(200.0));
}

// -- Weapon conversion -----------------------------------------------------

#[test]
fn to_weapon_defs() {
    let def = sample_def();
    let weapons = def.to_weapon_defs();

    assert_eq!(weapons.len(), 1);
    let w = &weapons[0];
    assert_eq!(w.damage, SimFloat::from_f64(10.0));
    assert_eq!(w.damage_type, DamageType::Normal);
    assert_eq!(w.range, SimFloat::from_f64(250.0));
    assert_eq!(w.reload_time, 15);
    assert_eq!(w.projectile_speed, SimFloat::from_f64(8.0));
    assert_eq!(w.area_of_effect, SimFloat::ZERO);
    assert!(!w.is_paralyzer);
}

#[test]
fn to_weapon_defs_paralyzer() {
    let mut def = sample_def();
    def.weapons[0].damage_type = "Paralyzer".into();
    let weapons = def.to_weapon_defs();

    assert_eq!(weapons[0].damage_type, DamageType::Paralyzer);
    assert!(weapons[0].is_paralyzer);
}

// -- Armor class parsing ---------------------------------------------------

#[test]
fn parse_armor_class_all_variants() {
    let cases = [
        ("Light", ArmorClass::Light),
        ("Medium", ArmorClass::Medium),
        ("Heavy", ArmorClass::Heavy),
        ("Building", ArmorClass::Building),
    ];
    for (s, expected) in &cases {
        let mut def = sample_def();
        def.armor_class = s.to_string();
        assert_eq!(def.parse_armor_class(), *expected);
    }
}

#[test]
fn parse_armor_class_unknown_defaults_to_light() {
    let mut def = sample_def();
    def.armor_class = "Banana".into();
    assert_eq!(def.parse_armor_class(), ArmorClass::Light);
}

// -- Registry operations ---------------------------------------------------

#[test]
fn registry_register_and_get() {
    let mut registry = UnitDefRegistry::new();
    assert!(registry.get(1).is_none());

    registry.register(sample_def());
    assert_eq!(registry.get(1).unwrap().name, "Peewee");

    // Overwrite
    let mut updated = sample_def();
    updated.name = "Peewee Mk2".into();
    registry.register(updated);
    assert_eq!(registry.get(1).unwrap().name, "Peewee Mk2");
}

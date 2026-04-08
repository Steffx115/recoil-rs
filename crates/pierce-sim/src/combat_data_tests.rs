use super::*;

/// Helper: build a simple [`WeaponDef`] with the given damage and type.
fn weapon(damage: SimFloat, damage_type: DamageType) -> WeaponDef {
    WeaponDef {
        damage,
        damage_type,
        range: SimFloat::from_int(100),
        reload_time: 30,
        projectile_speed: SimFloat::ZERO,
        area_of_effect: SimFloat::ZERO,
        is_paralyzer: damage_type == DamageType::Paralyzer,
    }
}

// -- default table value checks -----------------------------------------

#[test]
fn default_table_normal() {
    let t = DamageTable::default();
    assert_eq!(t.get(DamageType::Normal, ArmorClass::Light), SimFloat::ONE);
    assert_eq!(
        t.get(DamageType::Normal, ArmorClass::Medium),
        SimFloat::from_ratio(4, 5)
    );
    assert_eq!(t.get(DamageType::Normal, ArmorClass::Heavy), SimFloat::HALF);
    assert_eq!(
        t.get(DamageType::Normal, ArmorClass::Building),
        SimFloat::HALF
    );
}

#[test]
fn default_table_explosive() {
    let t = DamageTable::default();
    assert_eq!(
        t.get(DamageType::Explosive, ArmorClass::Light),
        SimFloat::HALF
    );
    assert_eq!(
        t.get(DamageType::Explosive, ArmorClass::Medium),
        SimFloat::ONE
    );
    assert_eq!(
        t.get(DamageType::Explosive, ArmorClass::Heavy),
        SimFloat::from_ratio(3, 2)
    );
    assert_eq!(
        t.get(DamageType::Explosive, ArmorClass::Building),
        SimFloat::from_ratio(3, 2)
    );
}

#[test]
fn default_table_laser() {
    let t = DamageTable::default();
    assert_eq!(
        t.get(DamageType::Laser, ArmorClass::Light),
        SimFloat::from_ratio(3, 2)
    );
    assert_eq!(t.get(DamageType::Laser, ArmorClass::Medium), SimFloat::ONE);
    assert_eq!(t.get(DamageType::Laser, ArmorClass::Heavy), SimFloat::HALF);
    assert_eq!(
        t.get(DamageType::Laser, ArmorClass::Building),
        SimFloat::from_ratio(3, 10)
    );
}

#[test]
fn default_table_paralyzer() {
    let t = DamageTable::default();
    assert_eq!(
        t.get(DamageType::Paralyzer, ArmorClass::Light),
        SimFloat::ONE
    );
    assert_eq!(
        t.get(DamageType::Paralyzer, ArmorClass::Medium),
        SimFloat::ONE
    );
    assert_eq!(
        t.get(DamageType::Paralyzer, ArmorClass::Heavy),
        SimFloat::ONE
    );
    assert_eq!(
        t.get(DamageType::Paralyzer, ArmorClass::Building),
        SimFloat::ONE
    );
}

// -- calc_damage for each combo -----------------------------------------

#[test]
fn calc_damage_normal_vs_all() {
    let t = DamageTable::default();
    let w = weapon(SimFloat::from_int(100), DamageType::Normal);

    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Light),
        SimFloat::from_int(100)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Medium),
        SimFloat::from_int(100) * SimFloat::from_ratio(4, 5)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Heavy),
        SimFloat::from_int(50)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Building),
        SimFloat::from_int(50)
    );
}

#[test]
fn calc_damage_explosive_vs_all() {
    let t = DamageTable::default();
    let w = weapon(SimFloat::from_int(100), DamageType::Explosive);

    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Light),
        SimFloat::from_int(50)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Medium),
        SimFloat::from_int(100)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Heavy),
        SimFloat::from_int(150)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Building),
        SimFloat::from_int(150)
    );
}

#[test]
fn calc_damage_laser_vs_all() {
    let t = DamageTable::default();
    let w = weapon(SimFloat::from_int(100), DamageType::Laser);

    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Light),
        SimFloat::from_int(150)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Medium),
        SimFloat::from_int(100)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Heavy),
        SimFloat::from_int(50)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Building),
        SimFloat::from_int(100) * SimFloat::from_ratio(3, 10)
    );
}

#[test]
fn calc_damage_paralyzer_returns_full_damage() {
    let t = DamageTable::default();
    let w = weapon(SimFloat::from_int(60), DamageType::Paralyzer);

    // Paralyzer multiplier is 1.0 for all armor classes → full damage.
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Light),
        SimFloat::from_int(60)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Medium),
        SimFloat::from_int(60)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Heavy),
        SimFloat::from_int(60)
    );
    assert_eq!(
        calc_damage(&t, &w, ArmorClass::Building),
        SimFloat::from_int(60)
    );
}

// -- WeaponSet as ECS component -----------------------------------------

#[test]
fn weapon_set_roundtrip() {
    use bevy_ecs::world::World;

    let mut world = World::new();
    let entity = world
        .spawn(WeaponSet {
            weapons: vec![
                WeaponInstance {
                    def_id: 0,
                    reload_remaining: 10,
                },
                WeaponInstance {
                    def_id: 3,
                    reload_remaining: 0,
                },
            ],
        })
        .id();

    let ws = world.get::<WeaponSet>(entity).unwrap();
    assert_eq!(ws.weapons.len(), 2);
    assert_eq!(ws.weapons[0].def_id, 0);
    assert_eq!(ws.weapons[0].reload_remaining, 10);
    assert_eq!(ws.weapons[1].def_id, 3);
    assert_eq!(ws.weapons[1].reload_remaining, 0);
}

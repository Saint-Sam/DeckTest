#![no_main]

use forge_core::{
    apply, Action, BaseCreatureCharacteristics, CardId, ContinuousEffectDefinition,
    ContinuousEffectId, ContinuousEffectOperation, ContinuousEffectTarget, CreatureKeywords,
    GameOutcome, GameState, ObjectColors, ObjectId, ObjectTypes, Outcome, PlayerId, ZoneId,
    ZoneKind,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    fuzz_characteristics(data);
});

fn fuzz_characteristics(data: &[u8]) {
    let mut reader = ByteReader::new(data);
    let seed = reader.read_u64();
    let mut state = GameState::new();
    let mut players = Vec::new();
    let mut objects = Vec::new();
    let mut effects = Vec::new();

    let _ = apply(&mut state, Action::SetSeed { seed });
    for _ in 0..2 {
        if let Outcome::PlayerAdded(player) = apply(&mut state, Action::AddPlayer) {
            players.push(player);
        }
    }
    for (index, player) in players.iter().copied().enumerate() {
        objects.extend(seed_battlefield(
            &mut state,
            player,
            30_000 + (index as u32 * 1_000),
            4,
        ));
    }

    let mut steps = 0_u16;
    while let Some(selector) = reader.next() {
        if steps >= 256 || state.game_outcome() != GameOutcome::InProgress {
            break;
        }
        steps = steps.saturating_add(1);
        match selector % 8 {
            0 | 1 => register_effect(&mut state, &players, &objects, &mut effects, &mut reader),
            2 => set_base_characteristics(&mut state, &objects, &mut reader),
            3 => move_object(&mut state, &players, &objects, &mut reader),
            4 => mark_damage(&mut state, &objects, &mut reader),
            5 => {
                let _ = apply(&mut state, Action::CheckStateBasedActions);
            }
            6 => query_characteristics(&state, &objects, &mut reader),
            _ => {
                let _ = state.deterministic_hash_streaming();
            }
        }
        assert_invariants(&state);
    }
}

fn seed_battlefield(
    state: &mut GameState,
    player: PlayerId,
    first_card: u32,
    count: u32,
) -> Vec<ObjectId> {
    let mut objects = Vec::new();
    let zone = ZoneId::new(None, ZoneKind::Battlefield);
    for offset in 0..count {
        if let Outcome::ObjectCreated(object) = apply(
            state,
            Action::CreateObject {
                card: CardId::new(first_card.saturating_add(offset)),
                owner: player,
                controller: player,
                zone,
            },
        ) {
            let base =
                BaseCreatureCharacteristics::new(1 + (offset as i32 % 4), 1 + (offset as i32 % 5))
                    .with_keywords(keyword_set(offset as u8));
            let _ = apply(
                state,
                Action::SetBaseCreatureCharacteristics { object, base },
            );
            objects.push(object);
        }
    }
    objects
}

fn register_effect(
    state: &mut GameState,
    players: &[PlayerId],
    objects: &[ObjectId],
    effects: &mut Vec<ContinuousEffectId>,
    reader: &mut ByteReader<'_>,
) {
    let (Some(controller), Some(target_object)) = (
        choose_player(players, reader),
        choose_object(objects, reader),
    ) else {
        return;
    };
    let operation = choose_operation(controller, target_object, players, objects, reader);
    let target = if reader.next().unwrap_or(0) % 4 == 0 {
        ContinuousEffectTarget::AllObjects
    } else {
        ContinuousEffectTarget::Object(target_object)
    };
    let mut definition = ContinuousEffectDefinition::new(controller, target, operation)
        .with_source(target_object)
        .with_timestamp(reader.read_u64() % 32);
    if let Some(dependency) =
        effects.get(usize::from(reader.next().unwrap_or(0)) % effects.len().max(1))
    {
        definition = definition.with_dependencies(vec![*dependency]);
    }
    if let Outcome::ContinuousEffectRegistered(effect) =
        apply(state, Action::RegisterContinuousEffect { definition })
    {
        effects.push(effect);
    }
}

fn choose_operation(
    default_controller: PlayerId,
    default_object: ObjectId,
    players: &[PlayerId],
    objects: &[ObjectId],
    reader: &mut ByteReader<'_>,
) -> ContinuousEffectOperation {
    let small = |byte: u8| i32::from(byte % 9) - 4;
    match reader.next().unwrap_or(0) % 13 {
        0 => ContinuousEffectOperation::CopyBaseCreature {
            from: choose_object(objects, reader).unwrap_or(default_object),
        },
        1 => ContinuousEffectOperation::ChangeController {
            controller: choose_player(players, reader).unwrap_or(default_controller),
        },
        2 => ContinuousEffectOperation::SetTextMarker {
            marker: u32::from(reader.next().unwrap_or(0)),
        },
        3 => ContinuousEffectOperation::SetTypes {
            types: type_set(reader.next().unwrap_or(0)),
        },
        4 => ContinuousEffectOperation::AddTypes {
            types: type_set(reader.next().unwrap_or(0)),
        },
        5 => ContinuousEffectOperation::RemoveTypes {
            types: type_set(reader.next().unwrap_or(0)),
        },
        6 => ContinuousEffectOperation::SetColors {
            colors: color_set(reader.next().unwrap_or(0)),
        },
        7 => ContinuousEffectOperation::AddKeywords {
            keywords: keyword_set(reader.next().unwrap_or(0)),
        },
        8 => ContinuousEffectOperation::RemoveKeywords {
            keywords: keyword_set(reader.next().unwrap_or(0)),
        },
        9 => ContinuousEffectOperation::SetBasePowerToughness {
            power: small(reader.next().unwrap_or(0)),
            toughness: small(reader.next().unwrap_or(0)),
        },
        10 => ContinuousEffectOperation::SetPowerToughness {
            power: small(reader.next().unwrap_or(0)),
            toughness: small(reader.next().unwrap_or(0)),
        },
        11 => ContinuousEffectOperation::ModifyPowerToughness {
            power: small(reader.next().unwrap_or(0)),
            toughness: small(reader.next().unwrap_or(0)),
        },
        _ => ContinuousEffectOperation::SwitchPowerToughness,
    }
}

fn set_base_characteristics(
    state: &mut GameState,
    objects: &[ObjectId],
    reader: &mut ByteReader<'_>,
) {
    let Some(object) = choose_object(objects, reader) else {
        return;
    };
    if reader.next().unwrap_or(0) % 5 == 0 {
        let _ = apply(state, Action::ClearBaseCreatureCharacteristics { object });
    } else {
        let power = i32::from(reader.next().unwrap_or(0) % 8);
        let toughness = i32::from(reader.next().unwrap_or(0) % 8);
        let base = BaseCreatureCharacteristics::new(power, toughness)
            .with_keywords(keyword_set(reader.next().unwrap_or(0)));
        let _ = apply(
            state,
            Action::SetBaseCreatureCharacteristics { object, base },
        );
    }
}

fn move_object(
    state: &mut GameState,
    players: &[PlayerId],
    objects: &[ObjectId],
    reader: &mut ByteReader<'_>,
) {
    let Some(object) = choose_object(objects, reader) else {
        return;
    };
    let zone = match reader.next().unwrap_or(0) % 3 {
        0 => ZoneId::new(None, ZoneKind::Battlefield),
        1 => ZoneId::new(choose_player(players, reader), ZoneKind::Graveyard),
        _ => ZoneId::new(choose_player(players, reader), ZoneKind::Hand),
    };
    let _ = apply(state, Action::MoveObject { object, to: zone });
}

fn mark_damage(state: &mut GameState, objects: &[ObjectId], reader: &mut ByteReader<'_>) {
    let Some(object) = choose_object(objects, reader) else {
        return;
    };
    let _ = apply(
        state,
        Action::MarkDamageOnObject {
            object,
            amount: u32::from(reader.next().unwrap_or(0) % 6),
        },
    );
}

fn query_characteristics(state: &GameState, objects: &[ObjectId], reader: &mut ByteReader<'_>) {
    let Some(object) = choose_object(objects, reader) else {
        return;
    };
    let _ = state.object_characteristics(object);
    let _ = state.object_controller(object);
    let _ = state.creature_characteristics(object);
}

fn assert_invariants(state: &GameState) {
    if let Err(error) = state.validate_zone_conservation() {
        panic!("zone conservation failed: {error:?}");
    }
    if state.deterministic_hash() != state.deterministic_hash_streaming() {
        panic!("allocated and streaming hashes diverged");
    }
}

fn choose_player(players: &[PlayerId], reader: &mut ByteReader<'_>) -> Option<PlayerId> {
    players
        .get(usize::from(reader.next().unwrap_or(0)) % players.len().max(1))
        .copied()
}

fn choose_object(objects: &[ObjectId], reader: &mut ByteReader<'_>) -> Option<ObjectId> {
    objects
        .get(usize::from(reader.next().unwrap_or(0)) % objects.len().max(1))
        .copied()
}

fn color_set(byte: u8) -> ObjectColors {
    let mut colors = ObjectColors::none();
    if byte & 0b0_0001 != 0 {
        colors = colors.with_white();
    }
    if byte & 0b0_0010 != 0 {
        colors = colors.with_blue();
    }
    if byte & 0b0_0100 != 0 {
        colors = colors.with_black();
    }
    if byte & 0b0_1000 != 0 {
        colors = colors.with_red();
    }
    if byte & 0b1_0000 != 0 {
        colors = colors.with_green();
    }
    colors
}

fn type_set(byte: u8) -> ObjectTypes {
    let mut types = ObjectTypes::none();
    if byte & 0b000_0001 != 0 {
        types = types.with_artifact();
    }
    if byte & 0b000_0010 != 0 {
        types = types.with_creature();
    }
    if byte & 0b000_0100 != 0 {
        types = types.with_enchantment();
    }
    if byte & 0b000_1000 != 0 {
        types = types.with_instant();
    }
    if byte & 0b001_0000 != 0 {
        types = types.with_land();
    }
    if byte & 0b010_0000 != 0 {
        types = types.with_planeswalker();
    }
    if byte & 0b100_0000 != 0 {
        types = types.with_sorcery();
    }
    types
}

fn keyword_set(byte: u8) -> CreatureKeywords {
    let mut keywords = CreatureKeywords::none();
    if byte & 0b0000_0001 != 0 {
        keywords = keywords.with_first_strike();
    }
    if byte & 0b0000_0010 != 0 {
        keywords = keywords.with_double_strike();
    }
    if byte & 0b0000_0100 != 0 {
        keywords = keywords.with_trample();
    }
    if byte & 0b0000_1000 != 0 {
        keywords = keywords.with_deathtouch();
    }
    if byte & 0b0001_0000 != 0 {
        keywords = keywords.with_lifelink();
    }
    if byte & 0b0010_0000 != 0 {
        keywords = keywords.with_flying();
    }
    if byte & 0b0100_0000 != 0 {
        keywords = keywords.with_reach();
    }
    if byte & 0b1000_0000 != 0 {
        keywords = keywords.with_menace();
    }
    keywords
}

struct ByteReader<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, index: 0 }
    }

    fn next(&mut self) -> Option<u8> {
        let value = self.data.get(self.index).copied();
        self.index = self.index.saturating_add(1);
        value
    }

    fn read_u64(&mut self) -> u64 {
        let mut value = 0_u64;
        for shift in 0..8 {
            value |= u64::from(self.next().unwrap_or(0)) << (shift * 8);
        }
        value
    }
}

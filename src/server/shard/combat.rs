use std::time::{Duration, Instant};

use crate::{
    entity::{Combatant, Entity, EntityID, Projectile, ProjectileKind, NPC},
    enums::{ItemLocation, SkillTargetType, TargetType},
    error::*,
    defines::EQUIP_SLOT_HAND,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    skills::{self, SkillResult},
    state::ShardServerState,
    tabledata::tdata_get,
    Position,
};

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sTargetNpcId {
    pub iNPC_ID: i32,
}
impl FFPacket for sTargetNpcId {}

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sTargetPcId {
    pub iPC_ID: i32,
}
impl FFPacket for sTargetPcId {}

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sBulletTarget {
    pub target_id: i32,
    pub character_type: i32,
}
impl FFPacket for sBulletTarget {}

const MAX_TARGETS: usize = 3;
const BATTERY_BASE_COST: u32 = 6;

// how early a FIRE or how late a HIT may arrive
// TODO tune against real-latency players
const LAG_ALLOWANCE: Duration = Duration::from_millis(150);

pub fn pc_attack_npcs(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_PC_ATTACK_NPCs = reader.get_struct()?;
    let target_count = pkt.iNPCCnt as usize;
    if target_count == 0 {
        return Ok(());
    }

    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    let mut weapon_boosts_needed = 0;
    for i in 0..target_count {
        // TODO stricter anti-cheat.
        // validate target count, range, attack cooldown, etc against weapon stats
        if i >= MAX_TARGETS {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Player {} tried to attack {} NPCs (max {})",
                    pc_id, pkt.iNPCCnt, MAX_TARGETS
                ),
            ));
        }
        let npc_id = reader.get_struct::<sTargetNpcId>()?.iNPC_ID;
        let npc = match state.get_npc(npc_id) {
            Ok(npc) => npc,
            Err(e) => {
                log_error(e);
                continue;
            }
        };
        weapon_boosts_needed += BATTERY_BASE_COST + npc.get_level() as u32;
        target_ids.push(npc.get_id());
    }

    // consume weapon boosts
    let player = state.get_player_mut(pc_id)?;
    let weapon_boosts = player.get_weapon_boosts();
    let charged = if weapon_boosts >= weapon_boosts_needed {
        player.set_weapon_boosts(weapon_boosts - weapon_boosts_needed);
        true
    } else {
        player.set_weapon_boosts(0);
        false
    };

    // attack handler
    skills::do_basic_attack(player.get_id(), &target_ids, charged, state)?;

    Ok(())
}

pub fn pc_attack_pcs(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_PC_ATTACK_CHARs = reader.get_struct()?;
    let target_count = pkt.iTargetCnt as usize;
    if target_count == 0 {
        return Ok(());
    }

    let player = state.get_player(pc_id)?;
    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    let mut weapon_boosts_needed = 0;
    for i in 0..target_count {
        // TODO see above
        if i >= MAX_TARGETS {
            log(
                Severity::Warning,
                &format!(
                    "{} tried to attack {} PCs (max {})",
                    player, pkt.iTargetCnt, MAX_TARGETS
                ),
            );
            break;
        }

        let target_pc_id = reader.get_struct::<sTargetPcId>()?.iPC_ID;
        if target_pc_id == pc_id {
            log(
                Severity::Warning,
                &format!("{} tried to attack themselves", player),
            );
            continue;
        }

        let target_player = match state.get_player(target_pc_id) {
            Ok(player) => player,
            Err(e) => {
                log_error(e);
                continue;
            }
        };

        weapon_boosts_needed += BATTERY_BASE_COST + target_player.get_level() as u32;
        target_ids.push(target_player.get_id());
    }

    // consume weapon boosts
    let player = state.get_player_mut(pc_id)?;
    let weapon_boosts = player.get_weapon_boosts();
    let charged = if weapon_boosts >= weapon_boosts_needed {
        player.set_weapon_boosts(weapon_boosts - weapon_boosts_needed);
        true
    } else {
        player.set_weapon_boosts(0);
        false
    };

    // attack handler
    skills::do_basic_attack(player.get_id(), &target_ids, charged, state)?;

    Ok(())
}

pub fn pc_rocket_style_fire(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_FIRE = pkt.get()?;

    // TODO validate range -> table data not accurate so actual range
    // server position, not the client's iX/Y/Z
    let start_pos = state.get_player(pc_id)?.get_position();
    let end_pos = Position { x: pkt.iToX, y: pkt.iToY, z: pkt.iToZ };

    let Some(projectile_id) = fire_projectile(ProjectileKind::Rocket, start_pos, end_pos, pc_id, state)? else {
        return Ok(());
    };

    let player = state.get_player(pc_id)?;
    let spc_bullet = sPCBullet::from(player.get_projectile(projectile_id)?);
    let (nano_id, nano_stamina) = player
        .get_active_nano()
        .map(|nano| (nano.get_id(), nano.get_stamina()))
        .unwrap_or((0, 0));

    let resp = sP_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC {
        iSkillID: pkt.iSkillID,
        iX: start_pos.x,
        iY: start_pos.y,
        iZ: start_pos.z,
        iToX: pkt.iToX,
        iToY: pkt.iToY,
        iToZ: pkt.iToZ,
        iBulletID: projectile_id,
        Bullet: spc_bullet,
        iBatteryW: player.get_weapon_boosts() as i32,
        bNanoDeactive: 0,
        iNanoID: nano_id,
        iNanoStamina: nano_stamina,
    };
    client.send_packet(P_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC, &resp);

    let bcast = sP_FE2CL_PC_ROCKET_STYLE_FIRE {
        iPC_ID: pc_id,
        iX: start_pos.x,
        iY: start_pos.y,
        iZ: start_pos.z,
        iToX: pkt.iToX,
        iToY: pkt.iToY,
        iToZ: pkt.iToZ,
        iBulletID: projectile_id,
        Bullet: spc_bullet,
        bNanoDeactive: 0,
    };
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_PC_ROCKET_STYLE_FIRE, &bcast);
        });

    Ok(())
}

pub fn pc_grenade_style_fire(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_GRENADE_STYLE_FIRE = pkt.get()?;

    // grenade FIRE has no launch point
    let start_pos = state.get_player(pc_id)?.get_position();
    let end_pos = Position { x: pkt.iToX, y: pkt.iToY, z: pkt.iToZ };

    let Some(projectile_id) = fire_projectile(ProjectileKind::Grenade, start_pos, end_pos, pc_id, state)? else {
        return Ok(());
    };

    let player = state.get_player(pc_id)?;
    let spc_bullet = sPCBullet::from(player.get_projectile(projectile_id)?);
    let (nano_id, nano_stamina) = player
        .get_active_nano()
        .map(|nano| (nano.get_id(), nano.get_stamina()))
        .unwrap_or((0, 0));

    let resp = sP_FE2CL_REP_PC_GRENADE_STYLE_FIRE_SUCC {
        iSkillID: pkt.iSkillID,
        iToX: pkt.iToX,
        iToY: pkt.iToY,
        iToZ: pkt.iToZ,
        iBulletID: projectile_id,
        Bullet: spc_bullet,
        iBatteryW: player.get_weapon_boosts() as i32,
        bNanoDeactive: 0,
        iNanoID: nano_id,
        iNanoStamina: nano_stamina,
    };
    client.send_packet(P_FE2CL_REP_PC_GRENADE_STYLE_FIRE_SUCC, &resp);

    let bcast = sP_FE2CL_PC_GRENADE_STYLE_FIRE {
        iPC_ID: pc_id,
        iToX: pkt.iToX,
        iToY: pkt.iToY,
        iToZ: pkt.iToZ,
        iBulletID: projectile_id,
        Bullet: spc_bullet,
        bNanoDeactive: 0,
    };
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_PC_GRENADE_STYLE_FIRE, &bcast);
        });

    Ok(())
}

fn fire_projectile(
    projectile_kind: ProjectileKind,
    start_pos: Position,
    end_pos: Position,
    pc_id: i32,
    state: &mut ShardServerState,
) -> FFResult<Option<i8>> {
    let player = state.get_player_mut(pc_id)?;

    let Some(weapon) = *player.get_item(ItemLocation::Equip, EQUIP_SLOT_HAND as usize)? else {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} tried to fire a {} with no item equipped", player, projectile_kind.name()),
        ));
    };

    let reject = |reason: &str| {
        FFError::build(
            Severity::Warning,
            format!("Player {} fired {} weapon {}: {}", pc_id, projectile_kind.name(), weapon.id, reason),
        )
    };

    let weapon_stats = weapon.get_stats()?;
    if weapon_stats.equip_type != Some(projectile_kind.equip_type()) {
        return Err(reject("wrong weapon type"));
    }

    let max_targets = weapon_stats
        .target_number
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| reject("no valid target number"))?;

    // m_iDelayTime is in tenths of a second
    let fire_delay = weapon_stats
        .delay_time
        .and_then(|tenths| u64::try_from(tenths).ok())
        .map(|tenths| Duration::from_millis(tenths * 100))
        .ok_or_else(|| reject("no valid delay time"))?;

    // m_iDurationTime is in tenths of a second; the client explodes the projectile after it
    let flight_time = weapon_stats
        .duration_time
        .and_then(|tenths| u64::try_from(tenths).ok())
        .map(|tenths| Duration::from_millis(tenths * 100))
        .ok_or_else(|| reject("no valid duration time"))?;

    let effect_area = weapon_stats
        .effect_area
        .and_then(|area| u32::try_from(area).ok())
        .ok_or_else(|| reject("no valid effect area"))?;

    // GCRA fire limit, per weapon (the client doesn't carry cooldowns across swaps)
    let now = Instant::now();
    if let Some(next_allowed) = player.get_next_shot_allowed(weapon.id) {
        if now + LAG_ALLOWANCE < next_allowed {
            return Err(reject(&format!(
                "too fast ({} ms before it was due, allowance {} ms, cooldown {} ms)",
                (next_allowed - now).as_millis(),
                LAG_ALLOWANCE.as_millis(),
                fire_delay.as_millis()
            )));
        }
    }

    // TODO finalize weapon boost logic with rockets/grenades
    let weapon_boosts = player.get_weapon_boosts();
    let weapon_boosts_needed = BATTERY_BASE_COST + weapon_stats.required_level as u32;

    let weapon_charged = if weapon_boosts >= weapon_boosts_needed {
        player.set_weapon_boosts(weapon_boosts - weapon_boosts_needed);
        true
    } else {
        player.set_weapon_boosts(0);
        false
    };

    let projectile = Projectile {
        kind: projectile_kind,
        weapon_id: weapon.id,
        single_power: player.get_single_power(),
        multi_power: player.get_multi_power(),
        charged: weapon_charged,
        effect_area,
        max_targets,
        start_pos,
        end_pos,
        fire_time: now,
        end_time: now + flight_time + LAG_ALLOWANCE,
    };

    let Some(projectile_id) = player.add_projectile(projectile) else {
        log(
            Severity::Debug,
            &format!("{} tried to fire a {} but has too many projectiles", player, projectile_kind.name()),
        );
        return Ok(None);
    };
    // book from the due time so waiting can't bank shots
    let due = player.get_next_shot_allowed(weapon.id).map_or(now, |next_allowed| next_allowed.max(now));
    player.set_next_shot_allowed(weapon.id, due + fire_delay);

    Ok(Some(projectile_id))
}

/// The client sends ROCKET_STYLE_HIT for both rockets and grenades.
pub fn pc_projectile_hit(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pc_id = clients.get_sender().get_player_id()?;
    let player = state.get_player_mut(pc_id)?;

    let mut reader = PacketReader::new(&pkt);
    let header: &sP_CL2FE_REQ_PC_ROCKET_STYLE_HIT = reader.get_struct()?;
    let projectile_id = header.iBulletID;

    let Some(projectile) = player.remove_projectile(projectile_id) else {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} reported a hit for unknown projectile {}", player, projectile_id),
        ));
    };

    let now = Instant::now();
    let flight_ms = (now - projectile.fire_time).as_millis();
    let limit_ms = (projectile.end_time - projectile.fire_time).as_millis();
    if now >= projectile.end_time {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "[projectile-timing] {} reported a {} hit for expired projectile {} ({} ms after fire, limit {} ms)",
                player,
                projectile.kind.name(),
                projectile_id,
                flight_ms,
                limit_ms
            ),
        ));
    }
    log(
        Severity::Debug,
        &format!(
            "[projectile-timing] {} {} projectile {} hit {} ms after fire (limit {} ms, {} targets claimed)",
            player,
            projectile.kind.name(),
            projectile_id,
            flight_ms,
            limit_ms,
            header.iTargetCnt
        ),
    );
    let spc_bullet: sPCBullet = sPCBullet::from(&projectile);

    let hit_pos = Position {
        x: header.iX,
        y: header.iY,
        z: header.iZ,
    };

    let player_instance = player.instance_id;

    let mut hits = Vec::new();
    for _ in 0..header.iTargetCnt {
        let npc_id = reader.get_struct::<sBulletTarget>()?.target_id;
        match state.get_npc(npc_id) {
            Ok(npc) => {
                if npc.instance_id != player_instance {
                    log(
                        Severity::Warning,
                        &format!(
                            "Player {} {} hit claimed {} in a different instance ({} != {})",
                            pc_id,
                            projectile.kind.name(),
                            npc,
                            npc.instance_id,
                            player_instance
                        ),
                    );
                    continue;
                }
                if let Some(distance) = hit_to_mob_distance(npc, hit_pos, projectile.effect_area) {
                    hits.push((npc.get_id(), distance));
                }
            }
            Err(e) => log_error(e),
        }
    }

    let target_ids = closest_targets(hits, projectile.max_targets);

    let results: Vec<sAttackResult> = skills::do_projectile_attack(EntityID::Player(pc_id), &target_ids, &projectile, state)?;

    let target_cnt = results.len() as i32;
    let mut bcast = match projectile.kind {
        ProjectileKind::Rocket => {
            PacketBuilder::new(P_FE2CL_PC_ROCKET_STYLE_HIT).with(&sP_FE2CL_PC_ROCKET_STYLE_HIT {
                iPC_ID: pc_id,
                iBulletID: projectile_id,
                Bullet: spc_bullet,
                iTargetCnt: target_cnt,
            })
        }
        ProjectileKind::Grenade => {
            PacketBuilder::new(P_FE2CL_PC_GRENADE_STYLE_HIT).with(&sP_FE2CL_PC_GRENADE_STYLE_HIT {
                iPC_ID: pc_id,
                iBulletID: projectile_id,
                Bullet: spc_bullet,
                iTargetCnt: target_cnt,
            })
        }
    };
    for result in &results {
        bcast.push(result);
    }

    if let Some(bcast) = log_if_failed(bcast.build()) {
        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |c| {
                c.send_payload(bcast.clone());
            });
    }

    Ok(())
}


pub fn nano_skill_use(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let Some(nano) = player.get_active_nano() else {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to use a nano skill without an active nano",
                player
            ),
        ));
    };

    let skill_level = placeholder!(1); // TODO calculate from gumballs

    let Some(skill) = nano.get_skill() else {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} tried to use a skill from a nano with no skill", player),
        ));
    };

    let skill_cost = skill.costs[skill_level as usize - 1] as i16;
    let nano_stamina = nano.get_stamina();
    if nano_stamina < skill_cost {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to use a nano skill without enough stamina ({} needed, {} available)",
                player, skill_cost, nano_stamina
            ),
        ));
    }

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_NANO_SKILL_USE = reader.get_struct()?;
    let target_count = pkt.iTargetCnt as usize;
    if target_count == 0 {
        return Ok(());
    }

    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    for i in 0..target_count {
        if i >= MAX_TARGETS {
            log(
                Severity::Warning,
                &format!(
                    "{} tried to use a nano skill on {} targets (max {})",
                    player, pkt.iTargetCnt, MAX_TARGETS
                ),
            );
            break;
        }

        let target_id = match skill.target_type {
            TargetType::HostileNPCs => {
                let target_npc_id = reader.get_struct::<sTargetNpcId>()?.iNPC_ID;
                EntityID::NPC(target_npc_id)
            }
            TargetType::FriendlyPCs => {
                let target_pc_id = reader.get_struct::<sTargetPcId>()?.iPC_ID;
                EntityID::Player(target_pc_id)
            }
            TargetType::CasterPC => player.get_id(),
        };

        // validate against targeting type
        let valid = match skill.targeting_type {
            SkillTargetType::None => {
                return Err(FFError::build(
                    Severity::Warning,
                    format!(
                        "{} tried to use a nano skill with no targeting type",
                        player
                    ),
                ));
            }
            _ => placeholder!(true), // TODO validate for each targeting type
        };

        if !valid {
            log(
                Severity::Warning,
                &format!(
                    "{} tried to use a nano skill on an invalid target {:?} for targeting type {:?}",
                    player, target_id, skill.targeting_type
                ),
            );
            continue;
        }

        target_ids.push(target_id);
    }

    let results = skills::do_skill(player.get_id(), &target_ids, skill, skill_level, state)?;

    let nano = state
        .get_player_mut(pc_id)
        .unwrap()
        .get_active_nano_mut()
        .unwrap();

    nano.set_stamina(nano_stamina - skill_cost);

    let target_cnt = results.len() as i32;
    let skill_id = nano.selected_skill.unwrap();
    let nano_stamina = nano.get_stamina();
    let nano_deactive = nano_stamina == 0;
    let nano_id = nano.get_id();
    let skill_type = skill.skill_type as i32;

    let mut succ_builder =
        PacketBuilder::new(P_FE2CL_NANO_SKILL_USE_SUCC).with(&sP_FE2CL_NANO_SKILL_USE_SUCC {
            iPC_ID: pc_id,
            iBulletID: pkt.iBulletID,
            iSkillID: skill_id,
            iArg1: pkt.iArg1,
            iArg2: pkt.iArg2,
            iArg3: pkt.iArg3,
            bNanoDeactive: nano_deactive as i32,
            iNanoID: nano_id,
            iNanoStamina: nano_stamina,
            eST: skill_type,
            iTargetCnt: target_cnt,
        });

    let mut bcast_builder =
        PacketBuilder::new(P_FE2CL_NANO_SKILL_USE).with(&sP_FE2CL_NANO_SKILL_USE {
            iPC_ID: pc_id,
            iBulletID: pkt.iBulletID,
            iSkillID: skill_id,
            iArg1: pkt.iArg1,
            iArg2: pkt.iArg2,
            iArg3: pkt.iArg3,
            bNanoDeactive: nano_deactive as i32,
            iNanoID: nano_id,
            iNanoStamina: nano_stamina,
            eST: skill_type,
            iTargetCnt: target_cnt,
        });

    for result in results {
        // These look identical, but since they have different concrete types, each arm
        // is a different push call with a different type parameter.
        match result {
            SkillResult::Damage(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::DotDamage(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::HealHP(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::HealStamina(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::StaminaSelf(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::DamageAndDebuff(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Buff(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::BatteryDrain(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::DamageAndMove(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Move(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Resurrect(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
        }
    }

    if let Some(pkt) = log_if_failed(succ_builder.build()) {
        client.send_payload(pkt);
    }

    if let Some(pkt) = log_if_failed(bcast_builder.build()) {
        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |c| {
                c.send_payload(pkt.clone());
            });
    }

    Ok(())
}

// distance from the hit to the mob's body center
fn hit_to_mob_distance(npc: &NPC, hit_pos: Position, effect_area: u32) -> Option<u32> {
    let stats = tdata_get().get_npc_stats(npc.ty).ok()?;

    let half_height = stats.height / 2;
    let npc_pos = npc.get_position();
    // mob server positions are from the feet
    let body_center = Position {
        z: npc_pos.z + half_height as i32,
        ..npc_pos
    };

    let limit = stats.radius.max(half_height) + effect_area;
    let distance = hit_pos.distance_to(&body_center);
    (distance < limit).then_some(distance)
}

fn closest_targets(mut hits: Vec<(EntityID, u32)>, max_targets: usize) -> Vec<EntityID> {
    hits.sort_by_key(|&(_, distance)| distance);
    hits.truncate(max_targets);
    hits.into_iter().map(|(id, _)| id).collect()
}

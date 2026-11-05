use std::{
    fs::OpenOptions,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    config::config_get,
    defines::EQUIP_SLOT_HAND,
    entity::{Combatant, Entity, EntityID},
    enums::{ItemLocation, ItemType, SkillTargetType, TargetType},
    error::*,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    skills::{self, Bullet, SkillResult},
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
struct sProjectileTarget {
    pub iID: i32,
    pub eCT: i32,
}
impl FFPacket for sProjectileTarget {}

const MAX_TARGETS: usize = 3;
const BATTERY_BASE_COST: u32 = 6;

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

// TODO anticheat for rockets (pc_rocket_ready / pc_rocket_fire / pc_rocket_hit):
// - equipped hand weapon is actually a rocket (WPN_EQUIP_TYPE_DH_ROCKET)
// - fire rate vs weapon m_iDelayTime
// - target count vs weapon m_iTargetNumber
// - targets within weapon m_iEffectArea of the impact point
// - impact point near the FIRE target coords
// - expire bullets that never hit
// TODO weapon boost consumption for rockets (bullets are always uncharged for now)

pub fn pc_rocket_ready(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pc_id = clients.get_sender().get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_READY = pkt.get()?;

    let bcast = sP_FE2CL_PC_ROCKET_STYLE_READY {
        iPC_ID: pc_id,
        iSkillID: pkt.iSkillID,
    };
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_PC_ROCKET_STYLE_READY, &bcast);
        });

    Ok(())
}

pub fn pc_rocket_fire(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_FIRE = pkt.get()?;

    let player = state.get_player_mut(pc_id)?;
    let weapon_id = match player.get_item(ItemLocation::Equip, EQUIP_SLOT_HAND as usize)? {
        Some(weapon) => weapon.id,
        None => {
            return Err(FFError::build(
                Severity::Warning,
                format!("{} tried to fire a rocket with no weapon equipped", player),
            ));
        }
    };

    let bullet = Bullet {
        single_power: player.get_single_power(),
        multi_power: player.get_multi_power(),
        style: Combatant::get_style(player),
        charged: false,
        weapon_id,
        fired_from: Position {
            x: pkt.iX,
            y: pkt.iY,
            z: pkt.iZ,
        },
        aimed_at: Position {
            x: pkt.iToX,
            y: pkt.iToY,
            z: pkt.iToZ,
        },
        fired_at: Instant::now(),
    };
    let bullet_id = player.add_bullet(bullet)?;
    let pc_bullet = make_pc_bullet(&bullet);

    let (nano_id, nano_stamina) = player
        .get_active_nano()
        .map(|nano| (nano.get_id(), nano.get_stamina()))
        .unwrap_or((0, 0));

    let resp = sP_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC {
        iSkillID: pkt.iSkillID,
        iX: pkt.iX,
        iY: pkt.iY,
        iZ: pkt.iZ,
        iToX: pkt.iToX,
        iToY: pkt.iToY,
        iToZ: pkt.iToZ,
        iBulletID: bullet_id,
        Bullet: pc_bullet,
        iBatteryW: player.get_weapon_boosts() as i32,
        bNanoDeactive: 0,
        iNanoID: nano_id,
        iNanoStamina: nano_stamina,
    };
    client.send_packet(P_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC, &resp);

    let bcast = sP_FE2CL_PC_ROCKET_STYLE_FIRE {
        iPC_ID: pc_id,
        iX: pkt.iX,
        iY: pkt.iY,
        iZ: pkt.iZ,
        iToX: pkt.iToX,
        iToY: pkt.iToY,
        iToZ: pkt.iToZ,
        iBulletID: bullet_id,
        Bullet: pc_bullet,
        bNanoDeactive: 0,
    };
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_PC_ROCKET_STYLE_FIRE, &bcast);
        });

    Ok(())
}

pub fn pc_rocket_hit(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pc_id = clients.get_sender().get_player_id()?;

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_HIT = reader.get_struct()?;
    let bullet_id = pkt.iBulletID;
    let target_count = pkt.iTargetCnt;

    let player = state.get_player_mut(pc_id)?;
    let Some(bullet) = player.remove_bullet(bullet_id) else {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} reported a hit for unknown bullet {}", player, bullet_id),
        ));
    };

    let mut claimed_ids = Vec::new();
    let mut target_ids = Vec::new();
    for _ in 0..target_count.max(0) {
        let npc_id = reader.get_struct::<sProjectileTarget>()?.iID;
        claimed_ids.push(npc_id);
        match state.get_npc(npc_id) {
            Ok(npc) => target_ids.push(npc.get_id()),
            Err(e) => log_error(e),
        }
    }

    let hit_pos = Position {
        x: pkt.iX,
        y: pkt.iY,
        z: pkt.iZ,
    };
    rocketlab_log_hit(state, pc_id, bullet_id, &bullet, hit_pos, &claimed_ids);
    warn_rocket_targets_out_of_range(state, pc_id, &bullet, hit_pos, &target_ids);

    let results =
        skills::do_projectile_attack(EntityID::Player(pc_id), &target_ids, &bullet, state)?;

    let mut bcast =
        PacketBuilder::new(P_FE2CL_PC_ROCKET_STYLE_HIT).with(&sP_FE2CL_PC_ROCKET_STYLE_HIT {
            iPC_ID: pc_id,
            iBulletID: bullet_id,
            Bullet: make_pc_bullet(&bullet),
            iTargetCnt: results.len() as i32,
        });
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

/// Result of checking one NPC against a rocket's client-reported hit point.
///
/// Mirrors the client's own target search (`NpcContainer.GetNearListMobTeam`, called
/// from `cnWarHead` with the weapon's `m_iEffectArea`): the NPC is a sphere centered at
/// its mid-height, `body_center = feet + m_iHeight / 2`, with radius
/// `body_size = max(m_iRadius, m_iHeight / 2)`. The blast reaches it if
/// `|hit - body_center| < body_size + effect_area`, measured in 3D.
/// The client uses the raw table values (no `m_fScale`). `slack` (config) is added
/// on top to absorb mob movement between the client's and server's view.
struct BlastCheck {
    /// the NPC's body center (server position is at its feet)
    body_center: Position,
    /// 3D distance from the hit point to `body_center`
    distance: f64,
    /// max(m_iRadius, m_iHeight / 2)
    body_size: f64,
    /// the weapon's blast radius (m_iEffectArea)
    effect_area: f64,
    slack: f64,
}
impl BlastCheck {
    fn new(npc_type: i32, npc_pos: Position, hit_pos: Position, effect_area: f64) -> Self {
        let (radius, height) = tdata_get()
            .get_npc_stats(npc_type)
            .map(|s| (s.radius as f64, s.height as f64))
            .unwrap_or((0.0, 0.0));
        let body_center = Position {
            z: npc_pos.z + (height / 2.0) as i32,
            ..npc_pos
        };
        Self {
            body_center,
            distance: rocketlab_d3(hit_pos, body_center),
            body_size: radius.max(height / 2.0),
            effect_area,
            slack: config_get().shard.rocket_blast_slack.get() as f64,
        }
    }

    fn limit(&self) -> f64 {
        self.body_size + self.effect_area + self.slack
    }

    fn ok(&self) -> bool {
        self.distance < self.limit()
    }

    /// How far inside the limit (negative = beyond it).
    fn margin(&self) -> f64 {
        self.limit() - self.distance
    }
}

/// The weapon's blast radius (m_iEffectArea), or 0 if the weapon has none.
fn weapon_effect_area(weapon_id: i16) -> f64 {
    tdata_get()
        .get_item_stats(weapon_id, ItemType::Hand)
        .ok()
        .and_then(|s| s.effect_area)
        .unwrap_or(0) as f64
}

/// Anticheat (warn-only for now): logs a warning for each claimed target that fails
/// [`BlastCheck`]. Damage is still applied.
fn warn_rocket_targets_out_of_range(
    state: &ShardServerState,
    pc_id: i32,
    bullet: &Bullet,
    hit_pos: Position,
    target_ids: &[EntityID],
) {
    let Ok(player) = state.get_player(pc_id) else {
        return;
    };
    let effect_area = weapon_effect_area(bullet.weapon_id);
    for target_id in target_ids {
        let EntityID::NPC(npc_id) = target_id else {
            continue;
        };
        let Ok(npc) = state.get_npc(*npc_id) else {
            continue;
        };
        let check = BlastCheck::new(npc.ty, npc.get_position(), hit_pos, effect_area);
        if !check.ok() {
            log(
                Severity::Warning,
                &format!(
                    "{} rocket hit claimed {} out of range: distance to body center {:.0} >= limit {:.0} \
                    (body {:.0} + blast {:.0} + slack {:.0}) (hit={:?}, body_center={:?})",
                    player,
                    npc,
                    check.distance,
                    check.limit(),
                    check.body_size,
                    check.effect_area,
                    check.slack,
                    hit_pos,
                    check.body_center
                ),
            );
        }
    }
}

// --- rocketlab: log-only instrumentation for deriving the client's blast rule ---

/// NPCs within this 3D distance of the HIT point are logged (claimed or not).
const ROCKETLAB_LOG_RADIUS: f64 = 1500.0;
/// Unclaimed NPCs up to this far beyond their limit get a `[rocketcheck] NEAR` line.
const ROCKETCHECK_NEAR_BAND: f64 = 150.0;
/// Unique per-process shot counter for the CSV (bullet IDs are reused slots).
static ROCKETLAB_SHOT_SEQ: AtomicU64 = AtomicU64::new(0);

fn rocketlab_d2(a: Position, b: Position) -> f64 {
    let dx = (a.x as f64) - (b.x as f64);
    let dy = (a.y as f64) - (b.y as f64);
    (dx * dx + dy * dy).sqrt()
}

fn rocketlab_d3(a: Position, b: Position) -> f64 {
    let dz = (a.z as f64) - (b.z as f64);
    (rocketlab_d2(a, b).powi(2) + dz * dz).sqrt()
}

fn rocketlab_fmt(p: Position) -> String {
    format!("({},{},{})", p.x, p.y, p.z)
}

/// Logs one HIT summary line, then one MOB line per NPC near the HIT point,
/// marking which ones the client claimed. Grep for `[rocketlab]`.
fn rocketlab_log_hit(
    state: &mut ShardServerState,
    pc_id: i32,
    bullet_id: i8,
    bullet: &Bullet,
    hit_pos: Position,
    claimed_ids: &[i32],
) {
    let Ok(player) = state.get_player(pc_id) else {
        return;
    };
    let shooter_pos = player.get_position();
    let instance_id = player.instance_id;
    let flight_ms = bullet.fired_at.elapsed().as_millis();

    let targets: Vec<String> = claimed_ids
        .iter()
        .map(|id| match state.get_npc(*id) {
            Ok(npc) => {
                let pos = npc.get_position();
                format!(
                    "{}@{} d2={:.0}",
                    id,
                    rocketlab_fmt(pos),
                    rocketlab_d2(pos, hit_pos)
                )
            }
            Err(_) => format!("{}@missing", id),
        })
        .collect();

    log(
        Severity::Info,
        &format!(
            "[rocketlab] HIT pc={} bullet={} weapon={} flight_ms={} hit={} aim={} from={} shooter_now={} \
            hit_aim_d2={:.0} hit_aim_d3={:.0} from_hit_d3={:.0} shooter_hit_d3={:.0} claimed={} targets=[{}]",
            pc_id,
            bullet_id,
            bullet.weapon_id,
            flight_ms,
            rocketlab_fmt(hit_pos),
            rocketlab_fmt(bullet.aimed_at),
            rocketlab_fmt(bullet.fired_from),
            rocketlab_fmt(shooter_pos),
            rocketlab_d2(hit_pos, bullet.aimed_at),
            rocketlab_d3(hit_pos, bullet.aimed_at),
            rocketlab_d3(bullet.fired_from, hit_pos),
            rocketlab_d3(shooter_pos, hit_pos),
            claimed_ids.len(),
            targets.join(", "),
        ),
    );

    // one line per claimed mob: mob position and hit position side by side
    for npc_id in claimed_ids {
        let Ok(npc) = state.get_npc(*npc_id) else {
            log(
                Severity::Info,
                &format!(
                    "[rocketlab] TARGET bullet={} npc={} mob=missing hit={}",
                    bullet_id,
                    npc_id,
                    rocketlab_fmt(hit_pos)
                ),
            );
            continue;
        };
        let mob_pos = npc.get_position();
        let r_eff = tdata_get()
            .get_npc_stats(npc.ty)
            .map(|s| s.radius as f32 * s.scale)
            .unwrap_or(0.0);
        log(
            Severity::Info,
            &format!(
                "[rocketlab] TARGET bullet={} npc={} type={} mob={} hit={} from={} \
                dx={} dy={} dz={} d2={:.0} d3={:.0} r_eff={:.0} from_mob_d3={:.0}",
                bullet_id,
                npc_id,
                npc.ty,
                rocketlab_fmt(mob_pos),
                rocketlab_fmt(hit_pos),
                rocketlab_fmt(bullet.fired_from),
                mob_pos.x - hit_pos.x,
                mob_pos.y - hit_pos.y,
                mob_pos.z - hit_pos.z,
                rocketlab_d2(mob_pos, hit_pos),
                rocketlab_d3(mob_pos, hit_pos),
                r_eff,
                rocketlab_d3(bullet.fired_from, mob_pos),
            ),
        );
    }

    let mut nearby = state.entity_map.find_npcs(|npc| {
        npc.instance_id == instance_id
            && rocketlab_d3(npc.get_position(), hit_pos) <= ROCKETLAB_LOG_RADIUS
    });
    // claimed targets outside the log radius still need a row
    for id in claimed_ids {
        if !nearby.contains(id) && state.get_npc(*id).is_ok() {
            nearby.push(*id);
        }
    }

    let effect_area = weapon_effect_area(bullet.weapon_id);
    let shot_seq = ROCKETLAB_SHOT_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut csv_rows = String::new();

    for npc_id in nearby {
        let Ok(npc) = state.get_npc(npc_id) else {
            continue;
        };
        let pos = npc.get_position();

        let check = BlastCheck::new(npc.ty, pos, hit_pos, effect_area);
        if let Ok(stats) = tdata_get().get_npc_stats(npc.ty) {
            let claimed = claimed_ids.contains(&npc_id);
            let d2 = rocketlab_d2(pos, hit_pos);
            let limit = check.limit();
            let verdict = match (claimed, check.ok()) {
                (true, true) => "OK",
                (true, false) => "OVER",
                (false, true) => "LOOSE",
                (false, false) => "OUT",
            };
            csv_rows.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.0},{},{:.3},{},{:.1},{:.1},{:.1},{:.1},{},{},{:.1},{}\n",
                ts_ms,
                shot_seq,
                pc_id,
                bullet.weapon_id,
                flight_ms,
                hit_pos.x,
                hit_pos.y,
                hit_pos.z,
                bullet.fired_from.x,
                bullet.fired_from.y,
                bullet.fired_from.z,
                shooter_pos.x,
                shooter_pos.y,
                shooter_pos.z,
                claimed_ids.len(),
                npc_id,
                npc.ty,
                npc.summoned,
                npc.is_dead(),
                rocketlab_d3(pos, npc.spawn_position),
                stats.radius,
                stats.scale,
                stats.height,
                stats.radius as f32 * stats.scale,
                stats.height as f32 * stats.scale,
                d2,
                rocketlab_d3(pos, hit_pos),
                pos.z - hit_pos.z,
                claimed,
                limit,
                verdict,
            ));
        }

        // range-rule comparison, logged for every claimed mob and for unclaimed
        // mobs near the limit. margin = how far inside the rule (negative = outside).
        //   OK    claimed, passes
        //   OVER  claimed, fails -> would warn (rule too tight, or cheat)
        //   LOOSE unclaimed, passes -> rule allows more than the client does
        //   NEAR  unclaimed, just fails -> rule agrees with client
        let claimed = claimed_ids.contains(&npc_id);
        let margin = check.margin();
        let verdict = match (claimed, check.ok()) {
            (true, true) => Some("OK"),
            (true, false) => Some("OVER"),
            (false, true) => Some("LOOSE"),
            (false, false) if margin >= -ROCKETCHECK_NEAR_BAND => Some("NEAR"),
            _ => None,
        };
        if let Some(verdict) = verdict {
            log(
                Severity::Info,
                &format!(
                    "[rocketcheck] {:<5} bullet={} npc={} type={} body={:.0} blast={:.0} dist={:.0} limit={:.0} margin={:+.0}",
                    verdict,
                    bullet_id,
                    npc_id,
                    npc.ty,
                    check.body_size,
                    check.effect_area,
                    check.distance,
                    check.limit(),
                    margin
                ),
            );
        }

        let (radius, scale, height) = tdata_get()
            .get_npc_stats(npc.ty)
            .map(|s| (s.radius, s.scale, s.height))
            .unwrap_or((0, 0.0, 0));
        log(
            Severity::Info,
            &format!(
                "[rocketlab] MOB bullet={} npc={} type={} claimed={} dead={} summoned={} pos={} hit={} drift={:.0} \
                r={} scale={:.2} h={} r_eff={:.0} h_eff={:.0} \
                hit_d2={:.0} hit_d3={:.0} hit_dz={} aim_d2={:.0} aim_d3={:.0}",
                bullet_id,
                npc_id,
                npc.ty,
                claimed_ids.contains(&npc_id),
                npc.is_dead(),
                npc.summoned,
                rocketlab_fmt(pos),
                rocketlab_fmt(hit_pos),
                rocketlab_d3(pos, npc.spawn_position),
                radius,
                scale,
                height,
                radius as f32 * scale,
                height as f32 * scale,
                rocketlab_d2(pos, hit_pos),
                rocketlab_d3(pos, hit_pos),
                pos.z - hit_pos.z,
                rocketlab_d2(pos, bullet.aimed_at),
                rocketlab_d3(pos, bullet.aimed_at),
            ),
        );
    }

    rocketlab_append_csv(&csv_rows);
}

const ROCKETLAB_CSV_HEADER: &str = "ts_ms,shot,pc,weapon,flight_ms,hit_x,hit_y,hit_z,from_x,from_y,from_z,\
shooter_x,shooter_y,shooter_z,claimed_cnt,npc,type,summoned,dead,drift,r,scale,h,body_r,body_h,d2,d3,dz,claimed,limit,verdict\n";

/// Appends rows to the rocketlab CSV (survives server restarts). Writes the header if the file is new.
fn rocketlab_append_csv(rows: &str) {
    let path = config_get().shard.rocketlab_csv_path.get();
    if path.is_empty() || rows.is_empty() {
        return;
    }
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| {
            if file.metadata()?.len() == 0 {
                file.write_all(ROCKETLAB_CSV_HEADER.as_bytes())?;
            }
            file.write_all(rows.as_bytes())
        });
    if let Err(e) = result {
        log(
            Severity::Warning,
            &format!("[rocketlab] couldn't write {}: {}", path, e),
        );
    }
}

fn make_pc_bullet(bullet: &Bullet) -> sPCBullet {
    sPCBullet {
        eAT: placeholder!(0), // TODO verify meaning of attack type
        iID: bullet.weapon_id as i32,
        bCharged: bullet.charged as i32,
    }
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

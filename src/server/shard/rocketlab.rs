//! TESTBED ONLY, never part of the PR.
//! Appends one CSV row per NPC near each rocket HIT (claimed or not) to `rocketlab.csv`,
//! in the format tools/rocketlab_fit.ps1 reads. Logged before damage is applied.

use std::{
    fs::OpenOptions,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    entity::{Bullet, BulletType, Combatant, Entity},
    enums::ItemType,
    error::*,
    state::ShardServerState,
    tabledata::tdata_get,
    Position,
};

const CSV_PATH: &str = "rocketlab.csv";
/// NPCs within this 3D distance of the HIT point get a row, claimed or not.
const LOG_RADIUS: f64 = 1500.0;
const HEADER: &str = "ts_ms,shot,pc,weapon,flight_ms,hit_x,hit_y,hit_z,from_x,from_y,from_z,\
shooter_x,shooter_y,shooter_z,claimed_cnt,npc,type,summoned,dead,drift,r,scale,h,body_r,body_h,d2,d3,dz,claimed,limit,verdict\n";

/// Unique per-process shot counter (bullet IDs are reused slots).
static SHOT_SEQ: AtomicU64 = AtomicU64::new(0);

fn d2(a: Position, b: Position) -> f64 {
    let dx = a.x as f64 - b.x as f64;
    let dy = a.y as f64 - b.y as f64;
    (dx * dx + dy * dy).sqrt()
}

fn d3(a: Position, b: Position) -> f64 {
    let dz = a.z as f64 - b.z as f64;
    (d2(a, b).powi(2) + dz * dz).sqrt()
}

/// `claimed_ids` = the raw target list the client sent, before any server filtering.
pub fn log_hit(
    state: &ShardServerState,
    pc_id: i32,
    bullet: &Bullet,
    hit_pos: Position,
    claimed_ids: &[i32],
) {
    let Ok(player) = state.get_player(pc_id) else {
        return;
    };
    let shooter = player.get_position();
    let instance_id = player.instance_id;
    let weapon_id = match bullet.bullet_type {
        BulletType::Rocket(id) => id,
        BulletType::Grenade => 0,
    };
    // time since FIRE = deliver time - time left before expiry
    let flight_ms = tdata_get()
        .get_item_stats(weapon_id, ItemType::Hand)
        .ok()
        .and_then(|s| s.deliver_time)
        .map(|deliver| {
            deliver as i64
                - bullet.end_time.saturating_duration_since(Instant::now()).as_millis() as i64
        })
        .unwrap_or(-1);

    let mut nearby = state.entity_map.find_npcs(|npc| {
        npc.instance_id == instance_id && d3(npc.get_position(), hit_pos) <= LOG_RADIUS
    });
    // claimed targets outside the log radius still need a row
    for id in claimed_ids {
        if !nearby.contains(id) && state.get_npc(*id).is_ok() {
            nearby.push(*id);
        }
    }

    let shot = SHOT_SEQ.fetch_add(1, Ordering::Relaxed);
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let from = bullet.start_pos;
    let effect_area = bullet.effect_area as f64;

    let mut rows = String::new();
    for npc_id in nearby {
        let Ok(npc) = state.get_npc(npc_id) else {
            continue;
        };
        let Ok(stats) = tdata_get().get_npc_stats(npc.ty) else {
            continue;
        };
        let pos = npc.get_position();
        let claimed = claimed_ids.contains(&npc_id);

        // same rule as is_valid_hit: distance to body center < max(r, h/2) + blast
        let half_height = stats.height / 2;
        let center = Position {
            z: pos.z + half_height as i32,
            ..pos
        };
        let limit = stats.radius.max(half_height) as f64 + effect_area;
        let passes = d3(hit_pos, center) < limit;
        let verdict = match (claimed, passes) {
            (true, true) => "OK",
            (true, false) => "OVER",
            (false, true) => "LOOSE",
            (false, false) => "OUT",
        };

        rows.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.0},{},{:.3},{},{:.1},{:.1},{:.1},{:.1},{},{},{:.1},{}\n",
            ts_ms,
            shot,
            pc_id,
            weapon_id,
            flight_ms,
            hit_pos.x,
            hit_pos.y,
            hit_pos.z,
            from.x,
            from.y,
            from.z,
            shooter.x,
            shooter.y,
            shooter.z,
            claimed_ids.len(),
            npc_id,
            npc.ty,
            npc.summoned,
            npc.is_dead(),
            d3(pos, npc.spawn_position),
            stats.radius,
            stats.scale,
            stats.height,
            stats.radius as f32 * stats.scale,
            stats.height as f32 * stats.scale,
            d2(pos, hit_pos),
            d3(pos, hit_pos),
            pos.z - hit_pos.z,
            claimed,
            limit,
            verdict,
        ));
    }

    append(&rows);
}

/// Appends rows (file survives restarts), writing the header if the file is new.
fn append(rows: &str) {
    if rows.is_empty() {
        return;
    }
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(CSV_PATH)
        .and_then(|mut file| {
            if file.metadata()?.len() == 0 {
                file.write_all(HEADER.as_bytes())?;
            }
            file.write_all(rows.as_bytes())
        });
    if let Err(e) = result {
        log(
            Severity::Warning,
            &format!("[rocketlab] couldn't write {}: {}", CSV_PATH, e),
        );
    }
}

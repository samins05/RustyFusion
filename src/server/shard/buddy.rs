use crate::{
    chunk::InstanceID,
    defines::*,
    entity::{BuddyListEntry, Entity, EntityID, PlayerSearchQuery},
    error::{codes::BuddyWarpErr, *},
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    state::ShardServerState,
    util,
};

use std::time::SystemTime;

const ERROR_CODE_BUDDY_DENY: i32 = 6;

pub fn get_buddy_state(clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let mut req = sP_FE2LS_REQ_GET_BUDDY_STATE {
        iPC_UID: player.get_uid(),
        aBuddyUID: [0; 50],
    };

    let buddy_info = player.get_all_buddy_info();
    for (i, buddy_uid) in buddy_info.iter().map(|info| info.pc_uid).enumerate() {
        req.aBuddyUID[i] = buddy_uid;
    }

    if let Some(login_server) = clients.get_login_server() {
        login_server.send_packet(P_FE2LS_REQ_GET_BUDDY_STATE, &req);
        Ok(())
    } else {
        Err(FFError::build(
            Severity::Warning,
            "No login server connected".to_string(),
        ))
    }
}

pub fn request_make_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_REQUEST_MAKE_BUDDY = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let buddy_id = pkt.iBuddyID;
    let buddy_uid = pkt.iBuddyPCUID;

    state.entity_map.validate_proximity(
        &[EntityID::Player(pc_id), EntityID::Player(buddy_id)],
        RANGE_INTERACT,
    )?;

    let player = state.get_player(pc_id)?;
    if player.is_buddies_with(buddy_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} is already buddies with player {}", player, buddy_uid),
        ));
    }

    if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} has too many buddies", player),
        ));
    }

    let req_pkt = sP_FE2CL_REP_REQUEST_MAKE_BUDDY_SUCC_TO_ACCEPTER {
        iRequestID: pc_id,
        iBuddyID: buddy_id,
        szFirstName: util::encode_utf16(&player.first_name).unwrap(),
        szLastName: util::encode_utf16(&player.last_name).unwrap(),
    };

    let buddy = state.get_player(buddy_id)?;
    if buddy.get_uid() != buddy_uid {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Buddy UID mismatch (client: {}, server: {})",
                buddy_uid,
                buddy.get_uid()
            ),
        ));
    }

    if buddy.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        // instant deny
        let deny_pkt = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL {
            iBuddyID: buddy_id,
            iBuddyPCUID: buddy_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };

        client.send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL, &deny_pkt);
        return Ok(());
    }

    let buddy_client = buddy.get_client().unwrap();
    buddy_client.send_packet(P_FE2CL_REP_REQUEST_MAKE_BUDDY_SUCC_TO_ACCEPTER, &req_pkt);

    let player = state.get_player_mut(pc_id).unwrap();
    player.buddy_offered_to = Some(buddy_uid);

    Ok(())
}

pub fn find_name_make_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_FIND_NAME_MAKE_BUDDY = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let pc_uid = player.get_uid();
    if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} has too many buddies", player),
        ));
    }

    let first_name = util::parse_utf16(&pkt.szFirstName)?;
    let last_name = util::parse_utf16(&pkt.szLastName)?;

    let search = PlayerSearchQuery::ByName(first_name, last_name);
    let res = search.execute(state);
    if res.is_none() {
        // TODO cross-shard
        return Ok(());
    }
    let buddy_id = res.unwrap();

    let buddy = state.get_player(buddy_id).unwrap();
    let buddy_uid = buddy.get_uid();
    if buddy.is_buddies_with(pc_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} is already buddies with player {}", player, buddy_id),
        ));
    }

    if buddy.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        // instant deny
        let deny_pkt = sP_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_FAIL {
            iErrorCode: ERROR_CODE_BUDDY_DENY,
            szFirstName: pkt.szFirstName,
            szLastName: pkt.szLastName,
        };

        let client = clients.get_sender();
        client.send_packet(P_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_FAIL, &deny_pkt);
        return Ok(());
    }

    let buddy_client = buddy.get_client().unwrap();
    let player = state.get_player_mut(pc_id).unwrap();
    player.buddy_offered_to = Some(buddy_uid);
    let req_pkt = sP_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_SUCC {
        szFirstName: util::encode_utf16(&player.first_name).unwrap(),
        szLastName: util::encode_utf16(&player.last_name).unwrap(),
        iPCUID: pc_uid,
        iNameCheckFlag: player.flags.name_check as i8,
    };
    buddy_client.send_packet(P_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_SUCC, &req_pkt);
    Ok(())
}

pub fn accept_make_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_ACCEPT_MAKE_BUDDY = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_buddy_info = BuddyListEntry::new(player);
    let pc_uid = state.get_player(pc_id)?.get_uid();
    let buddy_id = pkt.iBuddyID;
    let accepted = pkt.iAcceptFlag == 1;

    let buddy = state.get_player_mut(buddy_id)?;
    let buddy_uid = buddy.get_uid();
    if buddy.buddy_offered_to != Some(pc_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} did not send buddy request to player {}", buddy, pc_id),
        ));
    }

    buddy.buddy_offered_to = None;

    (|| {
        let buddy = state.get_player_mut(buddy_id).unwrap(); // re-borrow
        if !accepted {
            // this failure will be caught and the deny packet will be sent
            return Err(FFError::build(
                Severity::Debug,
                format!("{} denied buddy request from player {}", buddy, pc_id),
            ));
        }

        // player -> buddy
        let pkt_buddy = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: buddy.add_buddy(player_buddy_info.clone())? as i8,
            BuddyInfo: player_buddy_info.into(),
        };

        buddy
            .get_client()
            .unwrap()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_buddy);

        // buddy -> player
        let buddy_buddy_info = BuddyListEntry::new(buddy);
        let player = state.get_player_mut(pc_id).unwrap();
        let pkt_player = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: player.add_buddy(buddy_buddy_info.clone())? as i8,
            BuddyInfo: buddy_buddy_info.into(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_player);

        Ok(())
    })()
    .catch_fail(|| {
        let player = state.get_player_mut(pc_id).unwrap();
        let _ = player.remove_buddy(buddy_uid);

        let buddy = state.get_player_mut(buddy_id).unwrap();
        let _ = buddy.remove_buddy(pc_uid);

        // we send the deny packet to the buddy in case of failure
        let deny_pkt = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL {
            iBuddyID: pc_id,
            iBuddyPCUID: pc_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };

        let buddy_client = buddy.get_client().unwrap();
        buddy_client.send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL, &deny_pkt);
    })
}

pub fn find_name_accept_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_FIND_NAME_ACCEPT_BUDDY = pkt.get()?;

    let accepted = pkt.iAcceptFlag == 1;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_buddy_info = BuddyListEntry::new(player);
    let pc_uid = player.get_uid();
    if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} has too many buddies", player),
        ));
    }

    let buddy_uid = pkt.iBuddyPCUID;
    let search = PlayerSearchQuery::ByUID(buddy_uid);
    let res = search.execute(state);
    if res.is_none() {
        // TODO cross-shard
        return Ok(());
    }
    let buddy_id = res.unwrap();

    let buddy = state.get_player_mut(buddy_id)?;
    if buddy.buddy_offered_to != Some(pc_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} did not send buddy request to player {}", buddy, pc_id),
        ));
    }
    buddy.buddy_offered_to = None;

    (|| {
        let buddy = state.get_player_mut(buddy_id).unwrap(); // re-borrow
        if !accepted {
            // this failure will be caught and the deny packet will be sent
            return Err(FFError::build(
                Severity::Debug,
                format!("{} denied buddy request from player {}", buddy, pc_id),
            ));
        }

        // player -> buddy
        let pkt_buddy = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: buddy.add_buddy(player_buddy_info.clone())? as i8,
            BuddyInfo: player_buddy_info.into(),
        };

        buddy
            .get_client()
            .unwrap()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_buddy);

        // buddy -> player
        let buddy_buddy_info = BuddyListEntry::new(buddy);
        let player = state.get_player_mut(pc_id).unwrap();
        let pkt_player = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: player.add_buddy(buddy_buddy_info.clone())? as i8,
            BuddyInfo: buddy_buddy_info.into(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_player);

        Ok(())
    })()
    .catch_fail(|| {
        let player = state.get_player_mut(pc_id).unwrap();
        let _ = player.remove_buddy(buddy_uid);

        let buddy = state.get_player_mut(buddy_id).unwrap();
        let _ = buddy.remove_buddy(pc_uid);

        // we send the deny packet to the buddy in case of failure
        let deny_pkt = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL {
            iBuddyID: pc_id,
            iBuddyPCUID: pc_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };

        let buddy_client = buddy.get_client().unwrap();
        buddy_client.send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL, &deny_pkt);
    })
}

pub fn pc_buddy_warp(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_BUDDY_WARP = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_uid = player.get_player_uid();
    let player_is_on_skyway = player.get_skyway_ride().is_some();
    let player_payzone_flag = player.get_payzone_flag();
    let player_is_warp_on_cooldown = player.is_warp_on_cooldown();
    let buddy_uid = pkt.iBuddyPCUID;

    let invalid_warp = |msg: String, error_code: i32| -> FFResult<()> {
        let response = sP_FE2CL_REP_PC_BUDDY_WARP_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: error_code,
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_BUDDY_WARP_FAIL, &response);

        Err(FFError::build(Severity::Info, msg))
    };

    if !player.is_buddies_with(buddy_uid) {
        return invalid_warp(
            format!(
                "Buddy {} is not buddies with player {}",
                player_uid, buddy_uid
            ),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if player_is_on_skyway {
        return invalid_warp(
            format!("Player {} is currently on a skyway ride", player_uid),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if player_is_warp_on_cooldown {
        return invalid_warp(
            format!("Player {}'s buddy warp is on cooldown", player_uid),
            BuddyWarpErr::RechargeNotComplete as i32,
        );
    }

    let search = PlayerSearchQuery::ByUID(buddy_uid);
    let res = search.execute(state);
    if res.is_none() {
        let login_server = match clients.get_login_server() {
            Some(ls) => ls,
            None => {
                return Err(FFError::build(
                    Severity::Warning,
                    "No login server connected for cross-shard buddy warp".to_string(),
                ));
            }
        };

        let req_pkt = sP_FE2LS_REQ_BUDDY_WARP {
            iPCPayzoneFlag: player_payzone_flag as i8,
            iFromPCUID: player_uid,
            iBuddyPCUID: buddy_uid,
        };

        login_server.send_packet(P_FE2LS_REQ_BUDDY_WARP, &req_pkt);
        return Ok(());
    }

    let buddy_id = res.unwrap();

    let buddy = state.get_player_mut(buddy_id)?;
    let buddy_is_on_skyway = buddy.get_skyway_ride().is_some();
    let buddy_payzone_flag = buddy.get_payzone_flag();
    let buddy_instance_id = buddy.get_instance_id();
    let buddy_position = buddy.get_position();

    if buddy_is_on_skyway {
        return invalid_warp(
            format!("Buddy {} is currently on a skyway ride", buddy_uid),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if player_payzone_flag != buddy_payzone_flag {
        return invalid_warp(
            format!("Buddy {} is in a different payzone state", buddy_uid,),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if buddy_instance_id.map_num != ID_OVERWORLD {
        return invalid_warp(
            format!("Buddy {} is not in the overworld instance", buddy_uid,),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    {
        let player = state.get_player_mut(pc_id).unwrap();
        player.set_position(buddy_position);
        player.set_instance_id(InstanceID {
            map_num: buddy_instance_id.map_num,
            channel_num: buddy_instance_id.channel_num,
            instance_num: None,
        });
        player.buddy_warp_available_at =
            Some(util::get_timestamp_sec(SystemTime::now()) + BUDDYWARP_INTERVAL);

        state.entity_map.update(EntityID::Player(pc_id), None, true);

        // this packet in client code seems to just leave group
        let same_shard_succ_pkt = sP_FE2CL_REP_PC_BUDDY_WARP_SAME_SHARD_SUCC::default();

        // this packet in client code loads the new position
        let goto_succ_pkt = sP_FE2CL_REP_PC_GOTO_SUCC {
            iX: buddy_position.x,
            iY: buddy_position.y,
            iZ: buddy_position.z,
        };

        let client = clients.get_sender();

        client.send_packet(
            P_FE2CL_REP_PC_BUDDY_WARP_SAME_SHARD_SUCC,
            &same_shard_succ_pkt,
        );

        client.send_packet(P_FE2CL_REP_PC_GOTO_SUCC, &goto_succ_pkt);

        Ok(())
    }
    .catch_fail(|| {
        let response = sP_FE2CL_REP_PC_BUDDY_WARP_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: BuddyWarpErr::CantWarpToLocation as i32,
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_BUDDY_WARP_FAIL, &response);
    })
}

pub fn pc_set_buddy_block(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SET_BUDDY_BLOCK = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_uid = player.get_player_uid();
    let buddy_uid = pkt.iBuddyPCUID;

    buddy_log(
        "block",
        &format!(
            "REQ  {} -> SET_BUDDY_BLOCK target uid={} client_slot={}",
            player, buddy_uid, pkt.iBuddySlot
        ),
    );
    buddy_log("block", &format!("list before: {}", player.describe_buddy_list()));

    // TODO validation (error checking)
    // player side: keep the entry in its slot, flag it as blocked
    let player = state.get_player_mut(pc_id).unwrap();
    let slot = match player.block_player(buddy_uid) {
        Ok(slot) => slot,
        Err(e) => {
            buddy_log(
                "block",
                &format!("FAIL {} couldn't block uid={}: {}", player_uid, buddy_uid, e.get_msg()),
            );

            let fail = sP_FE2CL_REP_SET_BUDDY_BLOCK_FAIL {
                iBuddyPCUID: buddy_uid,
                iErrorCode: 0,
            };
            client.send_packet(P_FE2CL_REP_SET_BUDDY_BLOCK_FAIL, &fail);
            buddy_log("block", "SENT SET_BUDDY_BLOCK_FAIL to sender");
            return Ok(());
        }
    };

    buddy_log("block", &format!("list after:  {}", player.describe_buddy_list()));

    let resp = sP_FE2CL_REP_SET_BUDDY_BLOCK_SUCC {
        iBuddyPCUID: buddy_uid,
        iBuddySlot: slot as i8,
    };
    client.send_packet(P_FE2CL_REP_SET_BUDDY_BLOCK_SUCC, &resp);
    buddy_log(
        "block",
        &format!("SENT SET_BUDDY_BLOCK_SUCC to sender uid={} slot={}", buddy_uid, slot),
    );
    Ok(())
}

pub fn unignore_buddy(
    pkt: Packet,
    client: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = client.get_sender();
    let pkt : &sP_CL2FE_REQ_SET_PC_BLOCK = pkt.get()?;
    let pc_id = client.get_player_id()?;
    
    let player = state.get_player(pc_id)?;
    let player_uid = player.get_player_uid();
    let buddy_uid = pkt.iBlock_PCUID;

    buddy_log(
        "pc_block",
        &format!(
            "REQ  {} -> SET_PC_BLOCK target pc_id={} uid={} (on list: {})",
            player,
            pkt.iBlock_ID,
            buddy_uid,
            player.is_buddies_with(buddy_uid)
        ),
    );
    buddy_log("pc_block", &format!("list before: {}", player.describe_buddy_list()));

    let player = state.get_player_mut(pc_id)?;
    // unblock

    let slot = match player.unblock_player(buddy_uid) {
        Ok(slot) => slot,
        Err(e) => {
            buddy_log(
                "pc_block",
                &format!("FAIL {} couldn't unblock uid={}: {} (no reply sent)", player_uid, buddy_uid, e.get_msg()),
            );
            return Err(e);
        }
    };

    buddy_log("pc_block", &format!("list after:  {}", player.describe_buddy_list()));

    let unignore_succ_pkt = sP_FE2CL_REP_SET_PC_BLOCK_SUCC {
      iBlock_ID: pkt.iBlock_ID, // the unignored player's pc_id, not the sender's
      iBlock_PCUID: buddy_uid,
      iBuddySlot: slot as i8,
    };

    client.send_packet(P_FE2CL_REP_SET_PC_BLOCK_SUCC, &unignore_succ_pkt);
    buddy_log(
        "pc_block",
        &format!(
            "SENT SET_PC_BLOCK_SUCC to sender pc_id={} uid={} slot={}",
            pkt.iBlock_ID, buddy_uid, slot
        ),
    );

    Ok(())
}


pub fn pc_remove_buddy(
    pkt: Packet,
    client: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    // REMOVE_BUDDY: sent for delete (after SET_BUDDY_BLOCK) and for unignore
    let client = client.get_sender();
    let pkt: &sP_CL2FE_REQ_REMOVE_BUDDY = pkt.get()?;
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    // copy out of the packed struct first; format! can't borrow packed fields
    let buddy_uid = pkt.iBuddyPCUID;
    let client_slot = pkt.iBuddySlot;

    buddy_log(
        "remove",
        &format!(
            "REQ  {} -> REMOVE_BUDDY target uid={} client_slot={} (on list: {})",
            player,
            buddy_uid,
            client_slot,
            player.is_buddies_with(buddy_uid)
        ),
    );

    let player_uid = player.get_player_uid();

    let player = state.get_player_mut(pc_id)?;
    let player_slot = player.remove_buddy(buddy_uid)?;
    buddy_log("remove", &format!("list now:    {}", player.describe_buddy_list()));

    let resp = sP_FE2CL_REP_REMOVE_BUDDY_SUCC {
        iBuddyPCUID: buddy_uid,
        iBuddySlot: player_slot as i8,
    };
    client.send_packet(P_FE2CL_REP_REMOVE_BUDDY_SUCC, &resp);
    buddy_log(
        "remove",
        &format!("SENT REMOVE_BUDDY_SUCC to sender uid={} slot={}", buddy_uid, player_slot),
    );

    let Some(buddy_id) = PlayerSearchQuery::ByUID(buddy_uid).execute(state) else {
        // TODO cross sharding
        buddy_log("remove", "buddy not on this shard; skipping their side (TODO offline / cross-shard)");
        return Ok(());
    };

    let buddy = state.get_player_mut(buddy_id)?;
    let buddy_slot = buddy.remove_buddy(player_uid)?;
    buddy_log("remove", &format!("buddy list now: {}", buddy.describe_buddy_list()));

    let buddy_resp = sP_FE2CL_REP_REMOVE_BUDDY_SUCC {
        iBuddyPCUID: player_uid,
        iBuddySlot: buddy_slot as i8,
    };
    if let Some(buddy_client) = buddy.get_client() {
        buddy_client.send_packet(P_FE2CL_REP_REMOVE_BUDDY_SUCC, &buddy_resp);
        buddy_log(
            "remove",
            &format!("SENT REMOVE_BUDDY_SUCC to buddy uid={} slot={}", player_uid, buddy_slot),
        );
    }

    Ok(())
}

/// Consistent prefix so all buddy logs can be filtered with `Select-String "\[buddy:"`.
fn buddy_log(tag: &str, msg: &str) {
    log(Severity::Info, &format!("[buddy:{}] {}", tag, msg));
}

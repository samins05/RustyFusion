use crate::{
    defines::*,
    entity::{Entity, EntityID},
    error::*,
    helpers::send_system_message,
    monitor::monitor_event_to_packet,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    state::ShardServerState,
    util,
};

const CUSTOM_COMMAND_PREFIX: char = '!';

pub async fn send_freechat_message(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SEND_FREECHAT_MESSAGE = pkt.get()?;

    (async {
        let msg = util::parse_utf16(&pkt.szFreeChat)?;
        if let Some(cmdstr) = msg.strip_prefix(CUSTOM_COMMAND_PREFIX) {
            let tokens = cmdstr.split_whitespace().collect::<Vec<_>>();
            if !tokens.is_empty() {
                return commands::handle_custom_command(tokens, clients, state).await;
            }
        }

        let client = clients.get_sender();
        if msg.starts_with('/') {
            return send_system_message(
                client,
                &format!(
                    "You mistyped your built-in command!\n\
                        Or if you meant to use a custom command, try {}help",
                    CUSTOM_COMMAND_PREFIX
                ),
            );
        }

        let pc_id = client.get_player_id()?;
        let player = state.get_player(pc_id)?;
        if player.freechat_muted {
            return Err(FFError::build_dc(
                Severity::Warning,
                "Muted player sent freechat packet".to_string(),
            ));
        }

        let msg = helpers::process_freechat_message(msg);
        if msg.trim().is_empty() {
            return Ok(());
        }

        let resp = sP_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC {
            iPC_ID: pc_id,
            szFreeChat: util::encode_utf16(&msg).unwrap(),
            iEmoteCode: pkt.iEmoteCode,
        };

        log(Severity::Info, &format!("{}: \"{}\"", player, msg));

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::FreeChat,
                from: player.to_string(),
                to: None,
                message: msg,
            };

            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();

            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |client| {
                client.send_packet(P_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC, &resp);
            });
        Ok(())
    })
    .await
    .catch_fail(|| {
        let client = clients.get_sender();
        let resp = sP_FE2CL_REP_SEND_FREECHAT_MESSAGE_FAIL {
            iErrorCode: unused!(),
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };
        client.send_packet(P_FE2CL_REP_SEND_FREECHAT_MESSAGE_FAIL, &resp);
    })
}

pub fn send_menuchat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SEND_MENUCHAT_MESSAGE = pkt.get()?;

    (|| {
        let client = clients.get_sender();
        let pc_id = client.get_player_id()?;
        let player = state.get_player(pc_id)?;

        let msg = util::parse_utf16(&pkt.szFreeChat)?;
        if !helpers::validate_menuchat_message(&msg) {
            return Err(FFError::build(
                Severity::Warning,
                format!("Invalid menuchat message\n\t{}: '{}'", player, msg),
            ));
        }

        log(Severity::Info, &format!("{}: '{}'", player, msg));

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::MenuChat,
                from: player.to_string(),
                to: None,
                message: msg,
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        let resp = sP_FE2CL_REP_SEND_MENUCHAT_MESSAGE_SUCC {
            iPC_ID: pc_id,
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };
        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |client| {
                client.send_packet(P_FE2CL_REP_SEND_MENUCHAT_MESSAGE_SUCC, &resp);
            });
        Ok(())
    })()
    .catch_fail(|| {
        let client = clients.get_sender();
        let resp = sP_FE2CL_REP_SEND_MENUCHAT_MESSAGE_FAIL {
            iErrorCode: unused!(),
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };
        client.send_packet(P_FE2CL_REP_SEND_MENUCHAT_MESSAGE_FAIL, &resp);
    })
}

pub fn send_group_freechat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_ALL_GROUP_FREECHAT_MESSAGE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let msg = util::parse_utf16(&pkt.szFreeChat)?;
    if player.freechat_muted {
        return Err(FFError::build_dc(
            Severity::Warning,
            "Muted player sent freechat packet".to_string(),
        ));
    }

    let msg = helpers::process_freechat_message(msg);
    if msg.trim().is_empty() {
        return Ok(());
    }

    let pkt = sP_FE2CL_REP_SEND_ALL_GROUP_FREECHAT_MESSAGE_SUCC {
        iSendPCID: pc_id,
        szFreeChat: util::encode_utf16(&msg).unwrap(),
        iEmoteCode: pkt.iEmoteCode,
    };

    log(
        Severity::Info,
        &format!("{} (to group): \"{}\"", player, msg),
    );

    if let Some(login_server) = clients.get_login_server() {
        let chat_event = ffmonitor::ChatEvent {
            kind: ffmonitor::ChatKind::GroupChat,
            from: player.to_string(),
            to: None,
            message: msg,
        };
        let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
        login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
    } else {
        log(
            Severity::Warning,
            "No login server to send monitor chat event",
        );
    }

    if let Some(group_id) = player.group_id {
        let group = state.groups.get(&group_id).unwrap();
        for eid in group.get_member_ids() {
            let entity = state.entity_map.get_entity_raw(*eid).unwrap();
            if let Some(client) = entity.get_client() {
                client.send_packet(P_FE2CL_REP_SEND_ALL_GROUP_FREECHAT_MESSAGE_SUCC, &pkt);
            }
        }
    }

    Ok(())
}

pub fn send_group_menuchat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_ALL_GROUP_MENUCHAT_MESSAGE = pkt.get()?;
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let msg = util::parse_utf16(&pkt.szFreeChat)?;
    if !helpers::validate_menuchat_message(&msg) {
        return Err(FFError::build(
            Severity::Warning,
            format!("Invalid menuchat message\n\t{}: '{}'", player, msg),
        ));
    }

    log(Severity::Info, &format!("{} (to group): '{}'", player, msg));

    if let Some(login_server) = clients.get_login_server() {
        let chat_event = ffmonitor::ChatEvent {
            kind: ffmonitor::ChatKind::GroupMenuChat,
            from: player.to_string(),
            to: None,
            message: msg,
        };
        let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
        login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
    } else {
        log(
            Severity::Warning,
            "No login server to send monitor chat event",
        );
    }

    let pkt = sP_FE2CL_REP_SEND_ALL_GROUP_MENUCHAT_MESSAGE_SUCC {
        iSendPCID: pc_id,
        szFreeChat: pkt.szFreeChat,
        iEmoteCode: pkt.iEmoteCode,
    };
    if let Some(group_id) = player.group_id {
        let group = state.groups.get(&group_id).unwrap();
        for eid in group.get_member_ids() {
            let entity = state.entity_map.get_entity_raw(*eid).unwrap();
            if let Some(client) = entity.get_client() {
                client.send_packet(P_FE2CL_REP_SEND_ALL_GROUP_MENUCHAT_MESSAGE_SUCC, &pkt);
            }
        }
    }

    Ok(())
}

pub fn send_buddy_freechat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_BUDDY_FREECHAT_MESSAGE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let buddy_uid = pkt.iBuddyPCUID;

    if !player.is_buddies_with(buddy_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to send freechat to non-buddy UID {}",
                player, buddy_uid
            ),
        ));
    }

    let msg = helpers::process_freechat_message(util::parse_utf16(&pkt.szFreeChat)?);
    let reencoded_msg = util::encode_utf16(&msg).unwrap();

    let response_pkt = sP_FE2CL_REP_SEND_BUDDY_FREECHAT_MESSAGE_SUCC {
        iFromPCUID: player.get_uid(),
        iToPCUID: pkt.iBuddyPCUID,
        szFreeChat: reencoded_msg,
        iEmoteCode: pkt.iEmoteCode,
    };

    if let Some(buddy) = state.get_player_by_uid(buddy_uid) {
        log(
            Severity::Info,
            &format!("{} (to {}): \"{}\"", player, buddy, msg),
        );

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::BuddyChat,
                from: player.to_string(),
                to: Some(buddy.to_string()),
                message: msg,
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        if let Some(buddy_client) = buddy.get_client() {
            buddy_client.send_packet(P_FE2CL_REP_SEND_BUDDY_FREECHAT_MESSAGE_SUCC, &response_pkt);
        }

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_SEND_BUDDY_FREECHAT_MESSAGE_SUCC, &response_pkt);

        return Ok(());
    }

    if let Some(login_server) = clients.get_login_server() {
        let cross_shard_pkt = sP_FE2LS_REQ_SEND_BUDDY_FREECHAT {
            iFromPCUID: player.get_uid(),
            iToPCUID: pkt.iBuddyPCUID,
            szFreeChat: reencoded_msg,
            iEmoteCode: pkt.iEmoteCode,
        };

        login_server.send_packet(P_FE2LS_REQ_SEND_BUDDY_FREECHAT, &cross_shard_pkt);
    } else {
        return Err(FFError::build(
            Severity::Warning,
            "No login server found to forward buddy freechat message".to_string(),
        ));
    }

    Ok(())
}

pub fn send_buddy_menuchat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_BUDDY_MENUCHAT_MESSAGE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let buddy_uid = pkt.iBuddyPCUID;

    if !player.is_buddies_with(buddy_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to send menuchat to non-buddy UID {}",
                player, buddy_uid
            ),
        ));
    }

    let msg = util::parse_utf16(&pkt.szFreeChat)?;
    if !helpers::validate_menuchat_message(&msg) {
        return Err(FFError::build(
            Severity::Warning,
            format!("Invalid menuchat message\n\t{}: '{}'", player, msg),
        ));
    }

    let response_pkt = sP_FE2CL_REP_SEND_BUDDY_MENUCHAT_MESSAGE_SUCC {
        iFromPCUID: player.get_uid(),
        iToPCUID: pkt.iBuddyPCUID,
        szFreeChat: pkt.szFreeChat,
        iEmoteCode: pkt.iEmoteCode,
    };

    if let Some(buddy) = state.get_player_by_uid(buddy_uid) {
        log(
            Severity::Info,
            &format!("{} (to {}): '{}'", player, buddy, msg),
        );

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::BuddyMenuChat,
                from: player.to_string(),
                to: Some(buddy.to_string()),
                message: msg,
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        if let Some(buddy_client) = buddy.get_client() {
            buddy_client.send_packet(P_FE2CL_REP_SEND_BUDDY_MENUCHAT_MESSAGE_SUCC, &response_pkt);
        }

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_SEND_BUDDY_MENUCHAT_MESSAGE_SUCC, &response_pkt);

        return Ok(());
    }

    if let Some(login_server) = clients.get_login_server() {
        let cross_shard_pkt = sP_FE2LS_REQ_SEND_BUDDY_MENUCHAT {
            iFromPCUID: player.get_uid(),
            iToPCUID: pkt.iBuddyPCUID,
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };

        login_server.send_packet(P_FE2LS_REQ_SEND_BUDDY_MENUCHAT, &cross_shard_pkt);
    } else {
        return Err(FFError::build(
            Severity::Warning,
            "No login server found to forward buddy menuchat message".to_string(),
        ));
    }

    Ok(())
}

pub fn pc_avatar_emotes_chat(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_AVATAR_EMOTES_CHAT = pkt.get()?;

    let resp = sP_FE2CL_REP_PC_AVATAR_EMOTES_CHAT {
        iID_From: pkt.iID_From,
        iEmoteCode: pkt.iEmoteCode,
    };

    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |client| {
            client.send_packet(P_FE2CL_REP_PC_AVATAR_EMOTES_CHAT, &resp);
        });

    Ok(())
}

mod helpers {
    pub fn validate_menuchat_message(_msg: &str) -> bool {
        // TODO validate
        true
    }

    pub fn process_freechat_message(msg: String) -> String {
        // TODO process
        msg
    }
}

mod commands {
    use std::{collections::HashMap, future::Future, pin::Pin, sync::OnceLock, time::SystemTime};

    use crate::{
        chunk::TickMode,
        config::config_get,
        database::{db_get, DbImpl as _},
        entity::Combatant,
        entity::NPC,
        enums::{CombatantTeam, ItemLocation},
        net::FFProtocol,
        scripting::scripting_get,
        tabledata::tdata_get,
        Position,
    };

    use super::super::gm::helpers as gm_helpers;
    use super::*;

    struct Command {
        description: &'static str,
        handler: CommandHandler,
    }

    static AVAILABLE_COMMANDS: OnceLock<HashMap<&'static str, Command>> = OnceLock::new();
    type CommandHandler = for<'a> fn(
        Vec<&'a str>,
        &'a ClientMap<'a>,
        &'a mut ShardServerState,
    )
        -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>>;

    fn init_commands() -> HashMap<&'static str, Command> {
        #[rustfmt::skip]
        let commands: [(&'static str, &'static str, CommandHandler); 22] = [
            ("about", "Show information about the server", cmd_about),
            ("ban_a", "Ban an account", cmd_ban),
            ("ban_i", "Ban a player and their account", cmd_ban),
            ("unban", "Unban an account", cmd_unban),
            ("followme", "Make the nearest NPC start following you", cmd_followme),
            ("unfollowme", "Stop the nearest NPC from following you", cmd_unfollowme),
            ("changeai", "Change the AI script of an NPC", cmd_changeai),
            ("queryai", "Query the AI script of an NPC", cmd_queryai),
            ("reload", "Reload all Lua scripts", cmd_reload),
            ("perms", "View or change a player's permissions level", cmd_perms),
            ("ping", "View your current ping to the server", cmd_ping),
            ("redeem", "Redeem a code item", cmd_redeem),
            ("refresh", "Reinsert the player into the current chunk", cmd_refresh),
            ("registerall", "Register all transportation locations", cmd_registerall),
            ("unregisterall", "Unregister all transportation locations", cmd_unregisterall),
            ("mobring", "[testbed] Spawn mobs in rings around you", cmd_mobring),
            ("mobgrid", "[testbed] Spawn mobs in a grid around you", cmd_mobgrid),
            ("mobmix", "[testbed] Spawn a grid of interleaved mob types around you", cmd_mobmix),
            ("mobsweep", "[testbed] Spawn one batch of every spawned mob type, sorted by size", cmd_mobsweep),
            ("mobclear", "[testbed] Remove all summoned NPCs in your instance", cmd_mobclear),
            ("godmode", "[testbed] Toggle server-side invulnerability (can still attack)", cmd_godmode),
            ("help", "Show this help message", cmd_help),
        ];

        commands
            .into_iter()
            .map(|(name, description, handler)| {
                (
                    name,
                    Command {
                        description,
                        handler,
                    },
                )
            })
            .collect()
    }

    fn parse_pc_id(token: &str) -> Result<Option<i32>, ()> {
        if token == "." {
            return Ok(None);
        }
        token.parse::<i32>().map_err(|_| ()).map(Some)
    }

    pub async fn handle_custom_command<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> FFResult<()> {
        let cmds = AVAILABLE_COMMANDS.get_or_init(init_commands);

        let cmd_name = tokens[0];
        if let Some(cmd) = cmds.get(cmd_name) {
            (cmd.handler)(tokens, clients, state).await
        } else {
            send_system_message(
                clients.get_sender(),
                &format!(
                    "Unknown command {}{}\nUse {}help for a list of available commands",
                    CUSTOM_COMMAND_PREFIX, cmd_name, CUSTOM_COMMAND_PREFIX
                ),
            )
        }
    }

    fn cmd_about<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        _state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        let protocol_version = config_get().general.protocol.get();
        Box::pin(async move {
            send_system_message(
                clients.get_sender(),
                &format!(
                    "RustyFusion by ycc\n\
                Library version: {}\n\
                Protocol version: {:?}\n\
                Database version: {}",
                    LIB_VERSION, protocol_version, DB_VERSION,
                ),
            )
        })
    }

    fn cmd_ban<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let is_ban_i = tokens[0] == "ban_i";
            if tokens.len() < 3 {
                return send_system_message(
                    client,
                    &if is_ban_i {
                        format!(
                            "Usage: {}ban_i <pc_id> <duration> [reason]\n\
                    Duration example: 1d3h5m42s (no spaces!)",
                            CUSTOM_COMMAND_PREFIX
                        )
                    } else {
                        format!(
                            "Usage: {}ban_a <account_id> <duration> [reason]\n\
                    Duration example: 1d3h5m42s (no spaces!)",
                            CUSTOM_COMMAND_PREFIX
                        )
                    },
                );
            }

            let own_pc_id = client.get_player_id()?;
            let player = state.get_player(own_pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to ban players");
            }

            let pc_id = match parse_pc_id(tokens[1]) {
                Ok(Some(pc_id)) => pc_id,
                Ok(None) => own_pc_id,
                Err(_) => return send_system_message(client, "Invalid player ID"),
            };

            if own_pc_id == pc_id {
                return send_system_message(client, "You cannot ban yourself");
            }

            let db = db_get();

            let acc_id = if is_ban_i {
                let Ok(player) = state.get_player(pc_id) else {
                    return send_system_message(client, &format!("Player {} not found", pc_id));
                };

                let pc_uid = player.get_uid();
                let Some(acc) = db.find_account_from_player(pc_uid).await? else {
                    return send_system_message(
                        client,
                        &format!("Account not found for player {}", pc_uid),
                    );
                };

                acc.id
            } else {
                let Ok(acc_id) = tokens[1].parse::<i64>() else {
                    return send_system_message(client, "Invalid account ID");
                };

                acc_id
            };

            let Ok(duration) = util::get_duration_from_shorthand(tokens[2]) else {
                return send_system_message(client, "Invalid duration");
            };

            if duration.is_zero() {
                return send_system_message(client, "Duration must be non-zero");
            }

            let banned_until = SystemTime::now() + duration;
            let ban_reason = if tokens.len() > 3 {
                tokens[3..].join(" ")
            } else {
                "No reason given".to_string()
            };

            match db.ban_account(acc_id, banned_until, &ban_reason).await {
                Ok(()) => {
                    let ban_msg = format!(
                        "Account {} banned for {}\n\
                    Reason: {}",
                        acc_id,
                        util::format_duration(duration),
                        ban_reason,
                    );
                    log_if_failed(send_system_message(client, &ban_msg));
                    log(
                        Severity::Info,
                        &format!("{}\nBanned by: {}", ban_msg, player),
                    );
                }
                Err(e) => {
                    return send_system_message(client, &format!("Failed to ban: {}", e.get_msg()));
                }
            }

            if is_ban_i {
                let banned_player = state.get_player(pc_id)?;
                let banned_client = banned_player.get_client().unwrap();
                let pkt = sP_FE2CL_REP_PC_EXIT_SUCC {
                    iID: pc_id,
                    iExitCode: EXIT_CODE_REQ_BY_GM as i32,
                };

                banned_client.send_packet(P_FE2CL_REP_PC_EXIT_SUCC, &pkt);
                banned_client.disconnect();

                let client = clients.get_sender();
                log_if_failed(send_system_message(
                    client,
                    &format!("{} kicked", banned_player),
                ));
            }

            Ok(())
        })
    }

    fn cmd_unban<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            if tokens.len() < 2 {
                return send_system_message(
                    client,
                    &format!("Usage: {}unban <account_id>", CUSTOM_COMMAND_PREFIX),
                );
            }

            let player = state.get_player(client.get_player_id()?)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to unban players");
            }

            let Ok(acc_id) = tokens[1].parse::<i64>() else {
                return send_system_message(client, "Invalid account ID");
            };

            let db = db_get();
            match db.unban_account(acc_id).await {
                Ok(()) => {
                    let unban_msg = format!("Account {} unbanned", acc_id);
                    log(
                        Severity::Info,
                        &format!("{}\nUnbanned by: {}", unban_msg, player),
                    );
                    send_system_message(client, &unban_msg)
                }
                Err(e) => send_system_message(client, &format!("Failed to unban: {}", e.get_msg())),
            }
        })
    }

    fn cmd_queryai<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to change AI");
            }

            let npc_id = if tokens.len() < 2 {
                let player_pos = player.get_position();
                let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
                let mut closest_npc_id = None;
                let mut closest_distance = u32::MAX;
                for eid in candidates {
                    if let EntityID::NPC(npcid) = eid {
                        let npc = state.get_npc(npcid).unwrap();
                        let npc_pos = npc.get_position();
                        let distance = player_pos.distance_to(&npc_pos);
                        if distance < RANGE_INTERACT && distance < closest_distance {
                            closest_npc_id = Some(npc.id);
                            closest_distance = distance;
                        }
                    }
                }
                closest_npc_id
            } else {
                match tokens[1].parse::<i32>() {
                    Ok(id) => Some(id),
                    Err(_) => return send_system_message(client, "Invalid NPC ID"),
                }
            };

            if let Some(npc_id) = npc_id {
                let npc = state.get_npc(npc_id).unwrap();
                match npc.ai.as_ref() {
                    Some(ai_script_name) => send_system_message(
                        client,
                        &format!("NPC {} has AI script: {}", npc, ai_script_name),
                    ),
                    None => send_system_message(
                        client,
                        &format!("NPC {} has no AI script assigned", npc),
                    ),
                }
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_changeai<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to change AI");
            }

            let ai_script_name = if tokens.len() < 2 {
                return send_system_message(
                    client,
                    &format!("Usage: {}changeai <ai_script_name> [tick_mode]\nai_script_name: 'none' to clear\ntick_mode: when_loaded (default) | always", CUSTOM_COMMAND_PREFIX));
            } else {
                tokens[1]
            };

            let tick_mode = if tokens.len() < 3 {
                TickMode::WhenLoaded
            } else {
                match tokens[2].to_lowercase().as_str() {
                    "when_loaded" => TickMode::WhenLoaded,
                    "always" => TickMode::Always,
                    _ => {
                        return send_system_message(
                            client,
                            "Invalid tick mode. Valid options: when_loaded | always",
                        );
                    }
                }
            };

            let player_pos = player.get_position();
            let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
            let mut closest_npc_id = None;
            let mut closest_distance = u32::MAX;
            for eid in candidates {
                if let EntityID::NPC(npcid) = eid {
                    let npc = state.get_npc(npcid).unwrap();
                    let npc_pos = npc.get_position();
                    let distance = player_pos.distance_to(&npc_pos);
                    if distance < RANGE_INTERACT && distance < closest_distance {
                        closest_npc_id = Some(npc.id);
                        closest_distance = distance;
                    }
                }
            }

            if let Some(npc_id) = closest_npc_id {
                let npc = state.get_npc_mut(npc_id).unwrap();
                send_system_message(
                    client,
                    &format!("Changing AI of {} to {}", npc, ai_script_name),
                )?;

                npc.ai = match ai_script_name.to_lowercase().as_str() {
                    "none" => None,
                    other => Some(other.to_string()),
                };

                state
                    .entity_map
                    .set_tick(EntityID::NPC(npc_id), tick_mode)
                    .unwrap();

                Ok(())
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_reload<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to reload scripts");
            }

            let mut scripting = scripting_get().lock();
            scripting.reload()?;
            send_system_message(client, "Scripts reloaded")
        })
    }

    fn cmd_followme<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to move NPCs");
            }

            let player_pos = player.get_position();
            let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
            let mut closest_npc_id = None;
            let mut closest_distance = u32::MAX;
            for eid in candidates {
                if let EntityID::NPC(npcid) = eid {
                    let npc = state.get_npc(npcid).unwrap();
                    let npc_pos = npc.get_position();
                    let distance = player_pos.distance_to(&npc_pos);
                    if distance < RANGE_INTERACT && distance < closest_distance {
                        closest_npc_id = Some(npc.id);
                        closest_distance = distance;
                    }
                }
            }

            if let Some(npc_id) = closest_npc_id {
                let npc = state.get_npc_mut(npc_id).unwrap();
                npc.set_follow(EntityID::Player(pc_id));
                match &npc.ai {
                    None => {
                        npc.ai = Some("follow".to_string());
                        npc.team = CombatantTeam::Friendly;
                        npc.reset();
                        log_if_failed(send_system_message(
                            client,
                            &format!("{} is now following you", npc),
                        ));

                        state
                            .entity_map
                            .set_tick(EntityID::NPC(npc_id), TickMode::WhenLoaded)
                            .unwrap();
                    }
                    Some(ai_script) => {
                        log_if_failed(send_system_message(
                            client,
                            &format!(
                                "{} is unable to follow you (already running AI script: {})",
                                npc, ai_script
                            ),
                        ));
                    }
                }
                Ok(())
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_unfollowme<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to move NPCs");
            }
            let player_pos = player.get_position();

            let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
            let mut closest_npc_id = None;
            let mut closest_distance = u32::MAX;
            for eid in candidates {
                if let EntityID::NPC(npcid) = eid {
                    let npc = state.get_npc(npcid).unwrap();
                    let npc_pos = npc.get_position();
                    let distance = player_pos.distance_to(&npc_pos);
                    if distance < RANGE_INTERACT && distance < closest_distance {
                        closest_npc_id = Some(npc.id);
                        closest_distance = distance;
                    }
                }
            }

            if let Some(npc_id) = closest_npc_id {
                let npc = state.get_npc_mut(npc_id).unwrap();
                if npc.loose_follow == Some(EntityID::Player(pc_id)) {
                    npc.loose_follow = None;
                    npc.ai = None;
                    log_if_failed(send_system_message(
                        client,
                        &format!("{} is no longer following you", npc),
                    ));

                    state
                        .entity_map
                        .set_tick(EntityID::NPC(npc_id), TickMode::Never)
                        .unwrap();

                    Ok(())
                } else {
                    send_system_message(client, &format!("{} is not following you!", npc))
                }
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    // --- rocket anticheat testbed: deterministic mob spawning ---

    const TESTBED_MAX_MOBS: usize = 200;
    const TESTBED_DEFAULT_RADII: [i32; 4] = [100, 200, 300, 400];
    const TESTBED_DEFAULT_PER_RING: usize = 8;

    /// How testbed mobs behave once spawned.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum TestbedAi {
        /// frozen in place (exact positions)
        Idle,
        /// normal mob AI: aggro, chase, attack
        Mob,
        /// wanders around its spawn point, never attacks (`testbed_roam.luau`)
        Roam,
    }

    /// Strips a trailing `ai` / `roam` flag. Returns (args without the flag, mode).
    fn testbed_split_ai_flag<'t>(tokens: &'t [&'t str]) -> (&'t [&'t str], TestbedAi) {
        match tokens.split_last() {
            Some((last, rest)) if rest.len() >= 2 && last.eq_ignore_ascii_case("ai") => {
                (rest, TestbedAi::Mob)
            }
            Some((last, rest)) if rest.len() >= 2 && last.eq_ignore_ascii_case("roam") => {
                (rest, TestbedAi::Roam)
            }
            _ => (tokens, TestbedAi::Idle),
        }
    }

    /// Spawns one mob of `npc_type` at each position. Returns the spawned NPC IDs.
    fn testbed_spawn(
        state: &mut ShardServerState,
        pc_id: i32,
        npc_type: i32,
        positions: &[Position],
        ai: TestbedAi,
    ) -> FFResult<Vec<i32>> {
        let spawns: Vec<(i32, Position)> = positions.iter().map(|p| (npc_type, *p)).collect();
        testbed_spawn_typed(state, pc_id, &spawns, ai)
    }

    /// Spawns one mob per `(npc_type, position)` pair. Returns the spawned NPC IDs.
    fn testbed_spawn_typed(
        state: &mut ShardServerState,
        pc_id: i32,
        spawns: &[(i32, Position)],
        ai: TestbedAi,
    ) -> FFResult<Vec<i32>> {
        let player = state.get_player(pc_id)?;
        let angle = player.get_rotation();
        let instance_id = player.instance_id;

        let entity_map = &mut state.entity_map;
        let mut ids = Vec::with_capacity(spawns.len());
        for (npc_type, pos) in spawns {
            let npc_id = entity_map.gen_next_npc_id();
            let mut npc = NPC::new(npc_id, *npc_type, *pos, angle, instance_id)?;
            npc.spawn_position = *pos;
            gm_helpers::spawn_temp_npc(entity_map, npc, ai == TestbedAi::Mob);
            if ai == TestbedAi::Roam {
                let eid = EntityID::NPC(npc_id);
                if let Some(npc) = entity_map.get_entity_mut::<NPC>(eid) {
                    npc.ai = Some("testbed_roam".to_string());
                }
                entity_map.set_tick(eid, TickMode::WhenLoaded)?;
            }
            ids.push(npc_id);
        }
        Ok(ids)
    }

    fn testbed_report(
        clients: &ClientMap,
        kind: &str,
        npc_type: i32,
        center: Position,
        layout: &str,
        ai: TestbedAi,
        ids: &[i32],
    ) -> FFResult<()> {
        let mob_radius = tdata_get()
            .get_npc_stats(npc_type)
            .map(|s| s.radius)
            .unwrap_or_default();
        let summary = format!(
            "[testbed] {} type={} mob_radius={} center=({}, {}, {}) {} ai={:?} count={}",
            kind,
            npc_type,
            mob_radius,
            center.x,
            center.y,
            center.z,
            layout,
            ai,
            ids.len(),
        );
        log(Severity::Info, &format!("{} ids={:?}", summary, ids));
        send_system_message(clients.get_sender(), &summary)
    }

    fn testbed_check_perms(state: &ShardServerState, pc_id: i32) -> FFResult<bool> {
        Ok(state.get_player(pc_id)?.perms <= CN_ACCOUNT_LEVEL__GM as i16)
    }

    fn cmd_mobring<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            if !testbed_check_perms(state, pc_id)? {
                return send_system_message(client, "You do not have permission to spawn NPCs");
            }

            let (args, ai_mode) = testbed_split_ai_flag(&tokens);
            let usage = format!(
                "Usage: {}mobring <npc_type> [r1,r2,... | .] [per_ring] [ai|roam]\n\
                Default radii: 100,200,300,400; default per_ring: {}",
                CUSTOM_COMMAND_PREFIX, TESTBED_DEFAULT_PER_RING
            );
            if args.len() < 2 || args.len() > 4 {
                return send_system_message(client, &usage);
            }

            let Ok(npc_type) = args[1].parse::<i32>() else {
                return send_system_message(client, "Invalid NPC type");
            };
            if tdata_get().get_npc_stats(npc_type).is_err() {
                return send_system_message(client, &format!("Unknown NPC type {}", npc_type));
            }

            let radii: Vec<i32> = match args.get(2) {
                None | Some(&".") => TESTBED_DEFAULT_RADII.to_vec(),
                Some(list) => {
                    let parsed: Result<Vec<i32>, _> =
                        list.split(',').map(|r| r.trim().parse::<i32>()).collect();
                    match parsed {
                        Ok(radii) if !radii.is_empty() && radii.iter().all(|r| *r > 0) => radii,
                        _ => return send_system_message(client, "Invalid radius list"),
                    }
                }
            };

            let per_ring = match args.get(3) {
                None => TESTBED_DEFAULT_PER_RING,
                Some(n) => match n.parse::<usize>() {
                    Ok(n) if n > 0 => n,
                    _ => return send_system_message(client, "Invalid per_ring count"),
                },
            };

            let total = radii.len().saturating_mul(per_ring);
            if total > TESTBED_MAX_MOBS {
                return send_system_message(
                    client,
                    &format!("Too many mobs ({} > {})", total, TESTBED_MAX_MOBS),
                );
            }

            let center = state.get_player(pc_id)?.get_position();
            let mut positions = Vec::with_capacity(total);
            for r in &radii {
                for i in 0..per_ring {
                    let theta = std::f32::consts::TAU * i as f32 / per_ring as f32;
                    positions.push(Position {
                        x: center.x + (*r as f32 * theta.cos()).round() as i32,
                        y: center.y + (*r as f32 * theta.sin()).round() as i32,
                        z: center.z,
                    });
                }
            }

            let ids = testbed_spawn(state, pc_id, npc_type, &positions, ai_mode)?;
            let layout = format!("radii={:?} per_ring={}", radii, per_ring);
            testbed_report(clients, "mobring", npc_type, center, &layout, ai_mode, &ids)
        })
    }

    fn cmd_mobgrid<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            if !testbed_check_perms(state, pc_id)? {
                return send_system_message(client, "You do not have permission to spawn NPCs");
            }

            let (args, ai_mode) = testbed_split_ai_flag(&tokens);
            if args.len() != 5 {
                return send_system_message(
                    client,
                    &format!(
                        "Usage: {}mobgrid <npc_type> <rows> <cols> <spacing> [ai|roam]",
                        CUSTOM_COMMAND_PREFIX
                    ),
                );
            }

            let Ok(npc_type) = args[1].parse::<i32>() else {
                return send_system_message(client, "Invalid NPC type");
            };
            if tdata_get().get_npc_stats(npc_type).is_err() {
                return send_system_message(client, &format!("Unknown NPC type {}", npc_type));
            }

            let (Ok(rows), Ok(cols), Ok(spacing)) = (
                args[2].parse::<usize>(),
                args[3].parse::<usize>(),
                args[4].parse::<i32>(),
            ) else {
                return send_system_message(client, "Invalid rows/cols/spacing");
            };
            if rows == 0 || cols == 0 || spacing <= 0 {
                return send_system_message(client, "rows, cols and spacing must be positive");
            }

            let total = rows.saturating_mul(cols);
            if total > TESTBED_MAX_MOBS {
                return send_system_message(
                    client,
                    &format!("Too many mobs ({} > {})", total, TESTBED_MAX_MOBS),
                );
            }

            let center = state.get_player(pc_id)?.get_position();
            let positions: Vec<Position> = testbed_grid_cells(center, rows, cols, spacing)
                .into_iter()
                .map(|(_, _, pos)| pos)
                .collect();

            let ids = testbed_spawn(state, pc_id, npc_type, &positions, ai_mode)?;
            let layout = format!("rows={} cols={} spacing={}", rows, cols, spacing);
            testbed_report(clients, "mobgrid", npc_type, center, &layout, ai_mode, &ids)
        })
    }

    /// Grid cells centered on `center`, as `(row, col, position)`.
    fn testbed_grid_cells(
        center: Position,
        rows: usize,
        cols: usize,
        spacing: i32,
    ) -> Vec<(usize, usize, Position)> {
        let row_offset = (rows - 1) as f32 / 2.0;
        let col_offset = (cols - 1) as f32 / 2.0;
        let mut cells = Vec::with_capacity(rows * cols);
        for row in 0..rows {
            for col in 0..cols {
                cells.push((
                    row,
                    col,
                    Position {
                        x: center.x + ((col as f32 - col_offset) * spacing as f32).round() as i32,
                        y: center.y + ((row as f32 - row_offset) * spacing as f32).round() as i32,
                        z: center.z,
                    },
                ));
            }
        }
        cells
    }

    fn cmd_mobmix<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            if !testbed_check_perms(state, pc_id)? {
                return send_system_message(client, "You do not have permission to spawn NPCs");
            }

            let (args, ai_mode) = testbed_split_ai_flag(&tokens);
            if args.len() != 5 {
                return send_system_message(
                    client,
                    &format!(
                        "Usage: {}mobmix <t1,t2,...> <rows> <cols> <spacing> [ai|roam]\n\
                        Example: {}mobmix 2463,121,438,593,600,2097 6 6 200",
                        CUSTOM_COMMAND_PREFIX, CUSTOM_COMMAND_PREFIX
                    ),
                );
            }

            let parsed: Result<Vec<i32>, _> =
                args[1].split(',').map(|t| t.trim().parse::<i32>()).collect();
            let types = match parsed {
                Ok(types) if !types.is_empty() => types,
                _ => return send_system_message(client, "Invalid NPC type list"),
            };
            for t in &types {
                if tdata_get().get_npc_stats(*t).is_err() {
                    return send_system_message(client, &format!("Unknown NPC type {}", t));
                }
            }

            let (Ok(rows), Ok(cols), Ok(spacing)) = (
                args[2].parse::<usize>(),
                args[3].parse::<usize>(),
                args[4].parse::<i32>(),
            ) else {
                return send_system_message(client, "Invalid rows/cols/spacing");
            };
            if rows == 0 || cols == 0 || spacing <= 0 {
                return send_system_message(client, "rows, cols and spacing must be positive");
            }

            let total = rows.saturating_mul(cols);
            if total > TESTBED_MAX_MOBS {
                return send_system_message(
                    client,
                    &format!("Too many mobs ({} > {})", total, TESTBED_MAX_MOBS),
                );
            }

            // (row + col) % n: horizontal and vertical neighbours are different types
            let center = state.get_player(pc_id)?.get_position();
            let spawns: Vec<(i32, Position)> = testbed_grid_cells(center, rows, cols, spacing)
                .into_iter()
                .map(|(row, col, pos)| (types[(row + col) % types.len()], pos))
                .collect();

            let ids = testbed_spawn_typed(state, pc_id, &spawns, ai_mode)?;

            let per_type: Vec<String> = types
                .iter()
                .map(|t| {
                    let n = spawns.iter().filter(|(ty, _)| ty == t).count();
                    let body = tdata_get()
                        .get_npc_stats(*t)
                        .map(|s| s.radius as f32 * s.scale)
                        .unwrap_or(0.0);
                    format!("{}(body {:.0})x{}", t, body, n)
                })
                .collect();
            let summary = format!(
                "[testbed] mobmix center=({}, {}, {}) rows={} cols={} spacing={} ai={:?} count={} types: {}",
                center.x,
                center.y,
                center.z,
                rows,
                cols,
                spacing,
                ai_mode,
                ids.len(),
                per_type.join(", "),
            );
            log(Severity::Info, &format!("{} ids={:?}", summary, ids));
            send_system_message(client, &summary)
        })
    }

    const TESTBED_SWEEP_DEFAULT_PER_BATCH: usize = 12;
    const TESTBED_SWEEP_COPIES: usize = 3;
    const TESTBED_SWEEP_DEFAULT_SPACING: i32 = 250;

    /// Every hostile type that spawns in the world, sorted by body radius (m_iRadius * m_fScale).
    fn testbed_sweep_types() -> Vec<(i32, f32)> {
        let tdata = tdata_get();
        let mut types: Vec<(i32, f32)> = tdata
            .get_spawned_mob_types()
            .into_iter()
            .filter_map(|t| {
                tdata
                    .get_npc_stats(t)
                    .ok()
                    .map(|s| (t, s.radius as f32 * s.scale))
            })
            .collect();
        types.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        types
    }

    /// `!mobsweep <batch> [per_batch] [spacing] [ai|roam]`: batch `b` of `n` takes every
    /// `n`-th type from the size-sorted list, so each batch spans small to large mobs and all
    /// batches together cover every spawned mob type. Each type is spawned `TESTBED_SWEEP_COPIES` times.
    fn cmd_mobsweep<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            if !testbed_check_perms(state, pc_id)? {
                return send_system_message(client, "You do not have permission to spawn NPCs");
            }

            let (args, ai_mode) = testbed_split_ai_flag(&tokens);
            let all_types = testbed_sweep_types();

            let parse_or = |i: usize, default: usize| match args.get(i) {
                None => Some(default),
                Some(s) => s.parse::<usize>().ok().filter(|n| *n > 0),
            };
            let Some(per_batch) = parse_or(2, TESTBED_SWEEP_DEFAULT_PER_BATCH) else {
                return send_system_message(client, "Invalid per_batch");
            };
            let Some(spacing) = parse_or(3, TESTBED_SWEEP_DEFAULT_SPACING as usize) else {
                return send_system_message(client, "Invalid spacing");
            };
            let num_batches = all_types.len().div_ceil(per_batch).max(1);

            let batch = match args.get(1).map(|s| s.parse::<usize>()) {
                Some(Ok(b)) if (1..=num_batches).contains(&b) && args.len() <= 4 => b,
                _ => {
                    return send_system_message(
                        client,
                        &format!(
                            "Usage: {}mobsweep <batch 1-{}> [per_batch={}] [spacing={}] [ai|roam]\n\
                            {} spawned mob types, body {:.0} to {:.0}; {} copies each",
                            CUSTOM_COMMAND_PREFIX,
                            num_batches,
                            per_batch,
                            spacing,
                            all_types.len(),
                            all_types.first().map(|t| t.1).unwrap_or(0.0),
                            all_types.last().map(|t| t.1).unwrap_or(0.0),
                            TESTBED_SWEEP_COPIES,
                        ),
                    );
                }
            };

            let types: Vec<(i32, f32)> = all_types
                .iter()
                .enumerate()
                .filter(|(i, _)| i % num_batches == batch - 1)
                .map(|(_, t)| *t)
                .collect();

            let total = types.len() * TESTBED_SWEEP_COPIES;
            if total > TESTBED_MAX_MOBS {
                return send_system_message(
                    client,
                    &format!("Too many mobs ({} > {})", total, TESTBED_MAX_MOBS),
                );
            }
            let cols = (total as f32).sqrt().ceil() as usize;
            let rows = total.div_ceil(cols);

            // cycle types across cells so neighbours differ in size
            let center = state.get_player(pc_id)?.get_position();
            let spawns: Vec<(i32, Position)> =
                testbed_grid_cells(center, rows, cols, spacing as i32)
                    .into_iter()
                    .take(total)
                    .enumerate()
                    .map(|(i, (_, _, pos))| (types[i % types.len()].0, pos))
                    .collect();

            let ids = testbed_spawn_typed(state, pc_id, &spawns, ai_mode)?;

            let listed: Vec<String> = types
                .iter()
                .map(|(t, body)| format!("{}({:.0})", t, body))
                .collect();
            let summary = format!(
                "[testbed] mobsweep batch {}/{} ai={:?} count={} spacing={} types(body): {}",
                batch,
                num_batches,
                ai_mode,
                ids.len(),
                spacing,
                listed.join(" "),
            );
            log(Severity::Info, &format!("{} ids={:?}", summary, ids));
            send_system_message(client, &summary)
        })
    }

    /// Server-side invulnerability without telling the client. The client's GM
    /// `/invulnerable` state blocks attacking, so this lets you fight AI mobs in the testbed.
    fn cmd_godmode<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            if !testbed_check_perms(state, pc_id)? {
                return send_system_message(client, "You do not have permission to use god mode");
            }
            let player = state.get_player_mut(pc_id)?;
            player.invulnerable = !player.invulnerable;
            let msg = format!(
                "[testbed] god mode {} (server-side only; you can still attack)",
                if player.invulnerable { "ON" } else { "OFF" }
            );
            log(Severity::Info, &format!("{} for {}", msg, player));
            send_system_message(client, &msg)
        })
    }

    fn cmd_mobclear<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            if !testbed_check_perms(state, pc_id)? {
                return send_system_message(client, "You do not have permission to remove NPCs");
            }

            let instance_id = state.get_player(pc_id)?.instance_id;
            let npc_ids = state
                .entity_map
                .find_npcs(|npc| npc.summoned && npc.instance_id == instance_id);

            let mut removed = 0;
            for npc_id in npc_ids {
                // skip NPCs already despawned by AI (pending garbage collection);
                // untracking those here would double-untrack and panic
                if state
                    .entity_map
                    .get_entity_chunk(EntityID::NPC(npc_id))
                    .is_none()
                {
                    continue;
                }
                gm_helpers::remove_temp_npc(state, npc_id);
                removed += 1;
            }

            log(
                Severity::Info,
                &format!("[testbed] mobclear removed {} summoned NPCs", removed),
            );
            send_system_message(client, &format!("Removed {} summoned NPCs", removed))
        })
    }

    fn cmd_perms<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            if tokens.len() < 2 {
                return send_system_message(
                    client,
                    &format!(
                        "Usage: {}perms <pc_id> [new_level] [\"save\"]\n\
                Use . for pc_id to select yourself\n\
                Leave new_level empty to view the current level\n\
                Add \"save\" to save the new level to the account",
                        CUSTOM_COMMAND_PREFIX
                    ),
                );
            }

            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            let own_perms = player.perms;

            let target_pc_id = match parse_pc_id(tokens[1]) {
                Ok(Some(pc_id)) => pc_id,
                Ok(None) => pc_id,
                Err(_) => return send_system_message(client, "Invalid player ID"),
            };

            let Ok(target_player) = state.get_player_mut(target_pc_id) else {
                return send_system_message(client, &format!("Player {} not found", target_pc_id));
            };
            let target_perms = target_player.perms;
            let target_uid = target_player.get_uid();

            if tokens.len() < 3 {
                return send_system_message(
                    client,
                    &format!("{} has permissions level {}", target_player, target_perms),
                );
            }

            let Ok(new_perms) = tokens[2].parse::<i16>() else {
                return send_system_message(client, "Invalid permissions level");
            };

            if !(1..=99).contains(&new_perms) {
                return send_system_message(client, "Permissions level out of range [1, 99]");
            }

            if new_perms <= own_perms {
                return send_system_message(
                    client,
                    &format!(
                        "Can only grant weaker permissions than your own (> {})",
                        own_perms
                    ),
                );
            }

            if pc_id != target_pc_id && target_perms <= own_perms {
                return send_system_message(
                client, &format!(
                    "Can only change the permissions of a player with weaker ones than your own (> {})",
                    own_perms
                ),
            );
            }

            target_player.perms = new_perms;
            log_if_failed(send_system_message(
                client,
                &format!(
                    "Permissions level changed to {} for {}",
                    new_perms, target_player
                ),
            ));

            if tokens.get(3).is_some_and(|arg| *arg == "save") {
                let db = db_get();
                let saved = async {
                    let acc =
                        db.find_account_from_player(target_uid)
                            .await?
                            .ok_or(FFError::build(
                                Severity::Warning,
                                format!("Account not found for player with UID {}", target_uid),
                            ))?;
                    db.change_account_level(acc.id, new_perms as i32).await
                }
                .await;
                match saved {
                    Ok(()) => log_if_failed(send_system_message(
                        client,
                        "Permissions level saved to account!",
                    )),
                    Err(e) => log_if_failed(send_system_message(
                        client,
                        &format!("Failed to save permissions level: {}", e.get_msg()),
                    )),
                };
            }
            Ok(())
        })
    }

    fn cmd_ping<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        _state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let meta = client.meta.read();
            match meta.ping_ms {
                Some(ref ping) => {
                    let ms = ping.load(std::sync::atomic::Ordering::Relaxed);
                    drop(meta);
                    send_system_message(client, &format!("Your ping is {} ms", ms))
                }
                None => {
                    drop(meta);
                    send_system_message(client, "Ping not available")
                }
            }
        })
    }

    fn cmd_redeem<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let Some(code) = tokens.get(1) else {
                return send_system_message(
                    client,
                    &format!("Usage: {}redeem <code>", CUSTOM_COMMAND_PREFIX),
                );
            };

            let code = code.to_lowercase();
            let Some(items) = tdata_get().get_code_items(&code) else {
                return send_system_message(client, "Unknown code");
            };

            let pc_id = client.get_player_id()?;
            let player = state.get_player_mut(pc_id)?;
            let pc_uid = player.get_uid();

            if player.get_free_slots(ItemLocation::Inven) < items.len() {
                return send_system_message(client, "Not enough space in inventory");
            }

            let db = db_get();
            if db.is_code_redeemed(pc_uid, &code).await? {
                return send_system_message(client, "You have already redeemed this code item");
            }

            // record the redemption before handing out the items so they can't be duplicated
            db.record_code_redemption(pc_uid, &code).await?;

            for item in items {
                let slot_num = player.find_free_slot(ItemLocation::Inven)?;
                player.set_item(ItemLocation::Inven, slot_num, Some(*item))?;

                let location = ItemLocation::Inven as i32;
                match config_get().general.protocol.get() {
                    FFProtocol::v0104 => client.send_packet(
                        P_FE2CL_REP_PC_GIVE_ITEM_SUCC,
                        &v0104::sP_FE2CL_REP_PC_GIVE_ITEM_SUCC {
                            eIL: location,
                            iSlotNum: slot_num as i32,
                            Item: Some(*item).into_proto(),
                        },
                    ),
                    FFProtocol::v1013 => client.send_packet(
                        P_FE2CL_REP_PC_GIVE_ITEM_SUCC,
                        &v1013::sP_FE2CL_REP_PC_GIVE_ITEM_SUCC {
                            eIL: location,
                            iSlotNum: slot_num as i32,
                            Item: Some(*item).into_proto(),
                        },
                    ),
                }
            }

            log(
                Severity::Info,
                &format!("{} redeemed code {}", player, code),
            );

            send_system_message(
                client,
                if items.len() == 1 {
                    "You have redeemed a code item"
                } else {
                    "You have redeemed code items"
                },
            )
        })
    }

    fn cmd_refresh<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pc_id = clients.get_sender().get_player_id()?;
            let player = state.get_player(pc_id)?;
            let chunk_coords = player.get_chunk_coords();

            // remove from chunk completely; pulls all entities out of view
            state.entity_map.update(EntityID::Player(pc_id), None, true);

            // re-add to chunk; pushes all entities back into view
            state
                .entity_map
                .update(EntityID::Player(pc_id), Some(chunk_coords), true);

            Ok(())
        })
    }

    fn cmd_registerall<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pc_id = clients.get_sender().get_player_id()?;
            let player = state.get_player_mut(pc_id)?;

            player.flags.scamper_flags.set_all_chunks(i32::MAX);
            player.flags.skyway_flags.set_all_chunks(i64::MAX);

            let pkt = sP_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC {
                eTT: unused!(),
                iLocationID: unused!(),
                iWarpLocationFlag: player.flags.scamper_flags.get_chunk(0).unwrap(),
                aWyvernLocationFlag: player.flags.skyway_flags.to_array().unwrap(),
            };

            player
                .get_client()
                .unwrap()
                .send_packet(P_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC, &pkt);

            send_system_message(
                clients.get_sender(),
                "All warp locations have been registered.",
            )
        })
    }

    fn cmd_unregisterall<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pc_id = clients.get_sender().get_player_id()?;
            let player = state.get_player_mut(pc_id)?;

            player.flags.scamper_flags.set_all_chunks(0);
            player.flags.skyway_flags.set_all_chunks(0);

            let pkt = sP_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC {
                eTT: unused!(),
                iLocationID: unused!(),
                iWarpLocationFlag: player.flags.scamper_flags.get_chunk(0).unwrap(),
                aWyvernLocationFlag: player.flags.skyway_flags.to_array().unwrap(),
            };

            player
                .get_client()
                .unwrap()
                .send_packet(P_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC, &pkt);

            send_system_message(
                clients.get_sender(),
                "All warp locations have been unregistered.",
            )
        })
    }

    fn cmd_help<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        _state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut help_msg = "Available commands\n".to_string();
            for (cmd_name, cmd) in AVAILABLE_COMMANDS.get().unwrap() {
                help_msg.push_str(&format!(
                    "{}{}: {}\n",
                    CUSTOM_COMMAND_PREFIX, cmd_name, cmd.description
                ));
            }

            help_msg.pop(); // remove trailing newline
            send_system_message(clients.get_sender(), &help_msg)
        })
    }
}

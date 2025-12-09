use crate::ManagedReportState;
use serde_json::Value;
use shaco::rest::RESTClient;
use std::collections::HashSet;
use tauri::{AppHandle, Manager};

const REPORT_CATEGORIES: &[&str] = &[
    "NEGATIVE_ATTITUDE",
    "VERBAL_ABUSE",
    "LEAVING_AFK",
    "ASSISTING_ENEMY_TEAM",
    "HATE_SPEECH",
    "THIRD_PARTY_TOOLS",
    "INAPPROPRIATE_NAME",
];

fn extract_game_id(value: &Value) -> Option<u64> {
    fn parse_candidate(v: &Value) -> Option<u64> {
        match v {
            Value::Number(num) => num.as_u64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    if let Some(id) = value.get("gameId").and_then(parse_candidate) {
        return Some(id);
    }

    for path in [
        "/gameId",
        "/gameResult/gameId",
        "/gameSummary/gameId",
        "/teams/0/gameId",
        "/localPlayer/gameId",
    ] {
        if let Some(v) = value.pointer(path) {
            if let Some(id) = parse_candidate(v) {
                return Some(id);
            }
        }
    }

    match value {
        Value::Object(map) => map.values().find_map(extract_game_id),
        Value::Array(arr) => arr.iter().find_map(extract_game_id),
        _ => None,
    }
}

fn parse_u64_from_value(value: &Value) -> Option<u64> {
    match value {
        Value::Number(num) => num.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

async fn fetch_friend_ids(app_client: &RESTClient) -> HashSet<String> {
    let response = match app_client.get("/lol-chat/v1/friends".to_string()).await {
        Ok(res) => res,
        Err(_) => return HashSet::new(), // fail-safe
    };

    let mut ids = HashSet::new();

    if let Some(arr) = response.as_array() {
        for friend in arr {
            if let Some(id_val) = friend.get("summonerId") {
                if let Some(id_str) = id_val.as_str() {
                    ids.insert(id_str.trim().trim_matches('"').to_string());
                } else if let Some(id_num) = id_val.as_u64() {
                    ids.insert(id_num.to_string());
                }
            }
        }
    }

    ids
}

pub async fn handle_end_game_start_(
    app_handle: AppHandle,
    app_client: RESTClient,
    remoting_client: RESTClient,
) {
    let response = match remoting_client
        .get("/lol-end-of-game/v1/eog-stats-block".to_string())
        .await
    {
        Ok(data) => data,
        Err(_) => return,
    };

    let game_id = match extract_game_id(&response) {
        Some(id) => id,
        None => return,
    };

    {
        let state = app_handle.state::<ManagedReportState>();
        let mut guard = state.0.lock().await;
        if guard.last_report == Some(game_id) {
            return;
        }
        guard.last_report = Some(game_id);
    }

    let friend_ids = fetch_friend_ids(&app_client).await;
    let local_player = response.get("localPlayer").and_then(|p| {
        let id = p.get("summonerId").and_then(parse_u64_from_value)?;
        let puuid = p.get("puuid")?.as_str()?.to_string();
        Some((id, puuid))
    });

    
    if let Some((local_summoner_id, _)) = local_player {
        if let Some(teams) = response.get("teams").and_then(|v| v.as_array()) {
            for team in teams {
                if let Some(players) = team.get("players").and_then(|v| v.as_array()) {
                    for player in players {
                        // parse player summoner ID
                        let Some(player_id) =
                            player.get("summonerId").and_then(parse_u64_from_value)
                        else { continue };

                        let player_id_str = player_id.to_string();

                        // skip self + friends
                        if player_id == local_summoner_id
                            || friend_ids.contains(&player_id_str)
                        {
                            continue;
                        }

                        let Some(player_puuid) = player.get("puuid").and_then(|v| v.as_str())
                        else { continue };

                        // send report
                        let report_payload = serde_json::json!({
                            "gameId": game_id,
                            "categories": REPORT_CATEGORIES,
                            "offenderSummonerId": player_id,
                            "offenderPuuid": player_puuid,
                        });

                        let _ = remoting_client
                            .post(
                                "/lol-player-report-sender/v1/end-of-game-reports".to_string(),
                                report_payload,
                            )
                            .await;

                        // notify frontend
                        let frontend_payload = serde_json::json!({
                            "summonerId": player_id,
                            "puuid": player_puuid,
                            "gameId": game_id
                        });

                        let _ = app_handle.emit_all("end_of_game_started", frontend_payload);
                    }
                }
            }
        }
    }
}

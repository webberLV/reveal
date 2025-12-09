use crate::{
    champ_select::handle_champ_select_start,
    end_game::handle_end_game_start_,
    AppConfig,
};
use shaco::rest::RESTClient;
use tauri::{AppHandle, Manager};

pub async fn get_gameflow_state(remoting_client: &RESTClient) -> String {
    let state = remoting_client
        .get("/lol-gameflow/v1/gameflow-phase".to_string())
        .await
        .unwrap()
        .to_string();
    state.replace('\"', "")
}

pub async fn handle_client_state(
    client_state: String,
    app_handle: &AppHandle,
    remoting_client: &RESTClient,
    app_client: &RESTClient,
) {
    println!("Client State Update: {}", client_state);

    match client_state.as_str() {

        "ChampSelect" => {
            let cloned_app = app_handle.clone();
            let cloned_app_client = app_client.clone();
            let cloned_remoting = remoting_client.clone();
            
            let cfg = app_handle.state::<AppConfig>();
            let cfg = cfg.0.lock().await.clone();

            tauri::async_runtime::spawn(async move {
                handle_champ_select_start(
                    &cloned_app_client,
                    &cloned_remoting,
                    &cfg,
                    &cloned_app,
                )
                .await;
            });
        }

        "ReadyCheck" => {
            let cfg_state = app_handle.state::<AppConfig>();
            let cfg = cfg_state.0.lock().await;

            if cfg.auto_accept {
                tokio::time::sleep(std::time::Duration::from_millis(
                    (cfg.accept_delay as u64).saturating_sub(1000),
                ))
                .await;

                let _ = remoting_client
                    .post(
                        "/lol-matchmaking/v1/ready-check/accept".to_string(),
                        serde_json::json!({}),
                    )
                    .await;
            }
        }

        "PreEndOfGame" | "EndOfGame" => {
            let cfg_state = app_handle.state::<AppConfig>();
            let cfg = cfg_state.0.lock().await;

            if cfg.auto_report {
                let cloned_app = app_handle.clone();
                let cloned_app_client = app_client.clone();
                let cloned_remoting = remoting_client.clone();

                tauri::async_runtime::spawn(async move {
                    handle_end_game_start_(
                        cloned_app,
                        cloned_app_client,
                        cloned_remoting,
                    )
                    .await;
                });
            }
        }

        _ => {}
    }

    app_handle.emit_all("client_state_update", client_state).unwrap();
}

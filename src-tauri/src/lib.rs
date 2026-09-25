//! SFTPinguin – a modern, minimalist file transfer client.

mod commands;
mod edit;
mod error;
mod events;
mod known_hosts;
mod local;
mod model;
mod remote;
mod secrets;
mod session;
mod sites;
mod storage;
mod tls;
mod transfer;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

pub struct AppState {
    pub app: AppHandle,
    pub sites: sites::SiteStore,
    pub secrets: secrets::SecretStore,
    pub known_hosts: known_hosts::KnownHosts,
    pub sessions: session::SessionManager,
    pub transfers: transfer::TransferManager,
    pub edits: edit::EditManager,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let state = Arc::new(AppState {
                app: app.handle().clone(),
                sites: sites::SiteStore::load(config_dir.join("sites.json")),
                secrets: secrets::SecretStore::new(data_dir.join("secrets.json")),
                known_hosts: known_hosts::KnownHosts::load(config_dir.join("known_hosts.json")),
                sessions: Default::default(),
                transfers: Default::default(),
                edits: Default::default(),
            });
            app.manage(state.clone());
            transfer::TransferManager::start_progress_ticker(state.clone());
            edit::EditManager::start_watcher(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::platform_info,
            commands::list_sites,
            commands::save_site,
            commands::delete_site,
            commands::filezilla_default_path,
            commands::import_filezilla,
            commands::trust_host_key,
            commands::list_host_keys,
            commands::remove_host_key,
            commands::connect,
            commands::disconnect,
            commands::list_remote,
            commands::remote_stat,
            commands::remote_mkdir,
            commands::remote_create_file,
            commands::remote_rename,
            commands::remote_delete,
            commands::remote_chmod,
            commands::list_local,
            commands::local_places,
            commands::local_default_dir,
            commands::local_stat,
            commands::local_mkdir,
            commands::local_create_file,
            commands::local_rename,
            commands::local_delete,
            commands::local_chmod,
            commands::enqueue_transfers,
            commands::list_transfers,
            commands::cancel_transfer,
            commands::cancel_all_transfers,
            commands::retry_transfer,
            commands::clear_transfers,
            commands::set_transfer_options,
            commands::open_remote_file,
            commands::upload_edited,
            commands::stop_editing,
            commands::list_edited,
            commands::open_local_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SFTPinguin");
}

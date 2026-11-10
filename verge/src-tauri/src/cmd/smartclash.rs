use crate::{
    config::smartclash::Settings,
    core::smartclash::{self, Snapshot},
};

#[tauri::command]
#[specta::specta]
pub async fn get_smartclash_state() -> Snapshot {
    smartclash::get().await
}

#[tauri::command]
#[specta::specta]
pub async fn patch_smartclash_settings(settings: Settings) -> Snapshot {
    smartclash::patch(settings).await
}

#[tauri::command]
#[specta::specta]
pub async fn optimize_smartclash() -> Snapshot {
    smartclash::optimize().await
}

#[tauri::command]
#[specta::specta]
pub async fn get_smartclash_map() -> crate::core::smartclash_geo::MapSnapshot {
    crate::core::smartclash_geo::snapshot().await
}

#[tauri::command]
#[specta::specta]
pub async fn get_smartclash_subscription() -> Result<String, String> {
    crate::core::smartclash_export::link()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn export_smartclash_subscription() -> Result<String, String> {
    crate::core::smartclash_export::export()
        .await
        .map_err(|error| error.to_string())
}

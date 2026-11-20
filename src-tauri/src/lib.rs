pub mod models;
pub mod pdf_handler;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(pdf_handler::PiperState::default())
        .manage(pdf_handler::KokoroState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = pdf_handler::warmup_kokoro(&handle).await {
                    eprintln!("Kokoro warmup skipped: {}", err);
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            pdf_handler::parse_pdf,
            pdf_handler::generate_audio])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
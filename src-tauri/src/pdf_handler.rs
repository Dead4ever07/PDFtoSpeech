use crate::models::{PdfPageData, PdfTextElement};
use pdfium_render::prelude::*;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

static AUDIO_REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

struct PiperWorker {
    voice_name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for PiperWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Holds a persistent Piper TTS process so the 63MB ONNX voice model is loaded once per voice.
#[derive(Default)]
pub struct PiperState {
    worker: Mutex<Option<PiperWorker>>,
}

impl PiperWorker {
    fn spawn(app: &AppHandle, voice_name: &str) -> Result<Self, String> {
        if voice_name.is_empty()
            || !voice_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("Invalid voice name: {}", voice_name));
        }

        let piper_path = app
            .path()
            .resolve(
                "resources/piper/piper",
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| format!("Could not find Piper: {}", e))?;

        let piper_dir = piper_path
            .parent()
            .ok_or("Could not find Piper directory")?;

        let model_path = app
            .path()
            .resolve(
                format!("resources/models/{}/voice.onnx", voice_name),
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| format!("Could not find Voice Model {}: {}", voice_name, e))?;

        let config_path = app
            .path()
            .resolve(
                format!("resources/models/{}/voice.json", voice_name),
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| format!("Could not find Voice Config {}: {}", voice_name, e))?;

        let temp_dir = app.path().temp_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

        let mut child = Command::new(&piper_path)
            .current_dir(piper_dir)
            .env("LD_LIBRARY_PATH", piper_dir)
            .args([
                "--model",
                model_path.to_str().ok_or("Invalid model path")?,
                "--config",
                config_path.to_str().ok_or("Invalid config path")?,
                "--json-input",
                "--output_dir",
                temp_dir.to_str().ok_or("Invalid temp dir")?,
                "-q",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start Piper TTS: {}", e))?;

        let stdin = child.stdin.take().ok_or("Failed to open Piper stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to open Piper stdout")?;

        Ok(Self {
            voice_name: voice_name.to_string(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn synthesize(&mut self, text: &str, out_path: &Path) -> Result<Vec<u8>, String> {
        let clean_text = text.replace(['\r', '\n'], " ");
        let out_str = out_path.to_str().ok_or("Invalid output audio path")?;

        let payload = serde_json::json!({
            "text": clean_text,
            "output_file": out_str,
        });

        writeln!(self.stdin, "{}", payload).map_err(|e| format!("Piper write error: {}", e))?;
        self.stdin
            .flush()
            .map_err(|e| format!("Piper flush error: {}", e))?;

        let mut response_line = String::new();
        let bytes_read = self
            .stdout
            .read_line(&mut response_line)
            .map_err(|e| format!("Piper read error: {}", e))?;

        if bytes_read == 0 {
            return Err("Piper process closed stdout unexpectedly".to_string());
        }

        let audio_bytes =
            std::fs::read(out_path).map_err(|e| format!("Failed to read audio: {}", e))?;
        let _ = std::fs::remove_file(out_path);

        Ok(audio_bytes)
    }
}

#[tauri::command]
pub async fn parse_pdf(app: AppHandle, path: String) -> Result<Vec<PdfPageData>, String> {
    tauri::async_runtime::spawn_blocking(move || parse_pdf_sync(&app, &path))
        .await
        .map_err(|e| format!("PDF parsing task failed: {}", e))?
}

fn parse_pdf_sync(app: &AppHandle, path: &str) -> Result<Vec<PdfPageData>, String> {
    println!("Loading the library");

    let resource_dir = app
        .path()
        .resolve("resources", tauri::path::BaseDirectory::Resource)
        .map_err(|e| format!("Failed to resolve resource directory: {}", e))?;

    let resource_dir_str = resource_dir
        .to_str()
        .ok_or("Invalid characters in resource path")?;

    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(
            resource_dir_str,
        ))
        .map_err(|e| format!("Failed to bind to Pdfium at {}: {}", resource_dir_str, e))?,
    );

    println!("Starting the file parsing");
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {}", e))?;

    println!("File loaded, starting the parsing");

    let mut pages_data = Vec::new();

    for (index, page) in document.pages().iter().enumerate() {
        let mut raw_elements = Vec::new();

        // 1. Collect all valid words on the page
        for object in page.objects().iter() {
            if let Some(text_object) = object.as_text_object() {
                let raw_text = text_object.text();
                if raw_text.chars().any(|c| c.is_alphanumeric()) {
                    if let Ok(bounds) = text_object.bounds() {
                        raw_elements.push(PdfTextElement {
                            text: raw_text.trim().to_string(),
                            x: bounds.left().value,
                            y: page.height().value - bounds.top().value,
                            width: bounds.width().value,
                            height: bounds.height().value,
                        });
                    }
                }
            }
        }

        raw_elements.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));

        let mut line_elements = Vec::new();
        let mut current_line: Vec<PdfTextElement> = Vec::new();
        let y_tolerance = 5.0; // Margin of error for words on the same line

        // 3. Group words into lines
        for el in raw_elements {
            if current_line.is_empty() {
                current_line.push(el);
            } else {
                let baseline_y = current_line[0].y;
                // If the word is on the same vertical level
                if (el.y - baseline_y).abs() <= y_tolerance {
                    current_line.push(el);
                } else {
                    // Merge the current line and start a new one
                    line_elements.push(merge_line_elements(&mut current_line));
                    current_line.push(el);
                }
            }
        }

        if !current_line.is_empty() {
            line_elements.push(merge_line_elements(&mut current_line));
        }

        pages_data.push(PdfPageData {
            page_number: index as u16 + 1,
            width: page.width().value,
            height: page.height().value,
            elements: line_elements, // ⬅️ We now send merged lines instead of individual words!
        });
    }
    println!("Data sent to the front end");
    Ok(pages_data)
}

// Helper function to merge an array of words into a single line bounding box
fn merge_line_elements(line: &mut Vec<PdfTextElement>) -> PdfTextElement {
    // Sort words left-to-right
    line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let first = line.first().unwrap();
    let last = line.last().unwrap();

    let min_x = first.x;
    let max_x = last.x + last.width; // The far right edge of the last word

    // Find the highest top edge and tallest height in the line
    let max_y = line.iter().map(|e| e.y).fold(f32::MIN, f32::max);
    let max_height = line.iter().map(|e| e.height).fold(f32::MIN, f32::max);

    // Combine all the text with spaces
    let combined_text = line
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<&str>>()
        .join(" ");

    let merged = PdfTextElement {
        text: combined_text,
        x: min_x,
        y: max_y,
        width: max_x - min_x,
        height: max_height,
    };

    line.clear(); // Empty the vector so it can be reused for the next line
    merged
}

#[tauri::command]
pub async fn generate_audio(
    app: tauri::AppHandle,
    text: String,
    voice_name: String,
) -> Result<tauri::ipc::Response, String> {
    let audio_bytes = tauri::async_runtime::spawn_blocking(move || {
        let temp_dir = app.path().temp_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

        let req_id = AUDIO_REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
        let out_path = temp_dir.join(format!("pdftospeech_{}_{}.wav", std::process::id(), req_id));

        let state = app.state::<PiperState>();
        let mut guard = state
            .worker
            .lock()
            .map_err(|_| "Piper worker lock poisoned".to_string())?;

        // Respawn if voice changed or no worker exists yet
        let needs_spawn = guard
            .as_ref()
            .map(|w| w.voice_name != voice_name)
            .unwrap_or(true);

        if needs_spawn {
            *guard = None;
            *guard = Some(PiperWorker::spawn(&app, &voice_name)?);
        }

        let first_attempt = guard.as_mut().unwrap().synthesize(&text, &out_path);
        match first_attempt {
            Ok(bytes) => Ok(bytes),
            Err(_) => {
                // Self-heal once if the background Piper process exited
                let _ = std::fs::remove_file(&out_path);
                *guard = None;
                *guard = Some(PiperWorker::spawn(&app, &voice_name)?);
                guard.as_mut().unwrap().synthesize(&text, &out_path)
            }
        }
    })
    .await
    .map_err(|e| format!("Audio synthesis task failed: {}", e))??;

    Ok(tauri::ipc::Response::new(audio_bytes))
}

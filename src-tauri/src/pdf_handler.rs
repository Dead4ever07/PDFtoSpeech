use crate::layout_detector::{
    get_or_init_layout_detector, intersection_ratio, xy_cut_sort, LayoutBox, RegionAction,
    RegionRect, LAYOUT_INPUT_SIZE,
};
use crate::models::{PdfPageData, PdfTextElement};
use bincode::{config::standard, decode_from_slice};
use ndarray::Array;
use ort::{
    inputs,
    session::{RunOptions, Session},
    value::TensorRef,
};
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, CStr, CString};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

static AUDIO_REQ_COUNTER: AtomicU64 = AtomicU64::new(0);
const KOKORO_SAMPLE_RATE: u32 = 24_000;
const KOKORO_MAX_CLAUSE_CHARS: usize = 260;

#[derive(Clone, Copy)]
struct KokoroVoiceConfig {
    pack_key: &'static str,
    espeak_lang: &'static str,
    is_british: bool,
}

fn map_kokoro_voice(voice_name: &str) -> Option<KokoroVoiceConfig> {
    match voice_name {
        "kokoro_af_heart" => Some(KokoroVoiceConfig {
            pack_key: "af_heart",
            espeak_lang: "en-us",
            is_british: false,
        }),
        "kokoro_af_bella" => Some(KokoroVoiceConfig {
            pack_key: "af_bella",
            espeak_lang: "en-us",
            is_british: false,
        }),
        "kokoro_am_michael" => Some(KokoroVoiceConfig {
            pack_key: "am_michael",
            espeak_lang: "en-us",
            is_british: false,
        }),
        "kokoro_am_adam" => Some(KokoroVoiceConfig {
            pack_key: "am_adam",
            espeak_lang: "en-us",
            is_british: false,
        }),
        "kokoro_bf_emma" => Some(KokoroVoiceConfig {
            pack_key: "bf_emma",
            espeak_lang: "en-gb-x-rp",
            is_british: true,
        }),
        "kokoro_bm_george" => Some(KokoroVoiceConfig {
            pack_key: "bm_george",
            espeak_lang: "en-gb-x-rp",
            is_british: true,
        }),
        _ => None,
    }
}

type EspeakInitializeFn = unsafe extern "C" fn(c_int, c_int, *const c_char, c_int) -> c_int;
type EspeakSetVoiceByNameFn = unsafe extern "C" fn(*const c_char) -> c_int;
type EspeakTextToPhonemesFn =
    unsafe extern "C" fn(*mut *const c_char, c_int, c_int) -> *const c_char;

struct EspeakPhonemizer {
    _lib: libloading::Library,
    set_voice_by_name: EspeakSetVoiceByNameFn,
    text_to_phonemes: EspeakTextToPhonemesFn,
    active_lang: String,
}

// libloading::Library and function pointers are Send; access is serialized via Mutex.
unsafe impl Send for EspeakPhonemizer {}

impl EspeakPhonemizer {
    fn load(lib_path: &Path, data_dir: &Path) -> Result<Self, String> {
        let lib = unsafe {
            libloading::Library::new(lib_path)
                .map_err(|e| format!("Failed to load libespeak-ng: {}", e))?
        };

        let initialize: EspeakInitializeFn = unsafe {
            *lib.get::<EspeakInitializeFn>(b"espeak_Initialize\0")
                .map_err(|e| format!("Missing espeak_Initialize: {}", e))?
        };
        let set_voice_by_name: EspeakSetVoiceByNameFn = unsafe {
            *lib.get::<EspeakSetVoiceByNameFn>(b"espeak_SetVoiceByName\0")
                .map_err(|e| format!("Missing espeak_SetVoiceByName: {}", e))?
        };
        let text_to_phonemes: EspeakTextToPhonemesFn = unsafe {
            *lib.get::<EspeakTextToPhonemesFn>(b"espeak_TextToPhonemes\0")
                .map_err(|e| format!("Missing espeak_TextToPhonemes: {}", e))?
        };

        let data_cstr = CString::new(
            data_dir
                .to_str()
                .ok_or("Invalid espeak-ng-data directory path")?,
        )
        .map_err(|e| e.to_string())?;

        // 2 = AUDIO_OUTPUT_SYNCHRONOUS
        let rc = unsafe { initialize(2, 0, data_cstr.as_ptr(), 0) };
        if rc < 0 {
            return Err(format!("espeak_Initialize failed with code {}", rc));
        }

        let mut instance = Self {
            _lib: lib,
            set_voice_by_name,
            text_to_phonemes,
            active_lang: String::new(),
        };
        instance.set_language("en-us")?;
        Ok(instance)
    }

    fn set_language(&mut self, lang: &str) -> Result<(), String> {
        if self.active_lang == lang {
            return Ok(());
        }
        let lang_cstr = CString::new(lang).map_err(|e| e.to_string())?;
        let rc = unsafe { (self.set_voice_by_name)(lang_cstr.as_ptr()) };
        if rc != 0 {
            return Err(format!("espeak_SetVoiceByName({}) failed: {}", lang, rc));
        }
        self.active_lang = lang.to_string();
        Ok(())
    }

    fn phonemize_clause(&mut self, clause: &str, lang: &str) -> Result<String, String> {
        self.set_language(lang)?;
        let sanitized = clause.replace('\0', " ");
        let cstr = CString::new(sanitized).map_err(|e| e.to_string())?;
        let mut ptr = cstr.as_ptr();
        let mut out = String::new();

        // textmode = 1 (UTF-8), phonememode = 2 (IPA)
        while !ptr.is_null() {
            let ph_ptr = unsafe { (self.text_to_phonemes)(&mut ptr, 1, 2) };
            if ph_ptr.is_null() {
                break;
            }
            let ph = unsafe { CStr::from_ptr(ph_ptr) }.to_string_lossy();
            let trimmed = ph.trim();
            if !trimmed.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(trimmed);
            }
        }
        Ok(out)
    }
}

pub struct KokoroEngine {
    session: tauri::async_runtime::Mutex<Session>,
    voices: HashMap<String, Vec<Vec<Vec<f32>>>>,
    espeak: Mutex<EspeakPhonemizer>,
}

#[derive(Default)]
pub struct KokoroState {
    engine: tauri::async_runtime::Mutex<Option<Arc<KokoroEngine>>>,
}

#[derive(Default)]
pub struct ExportState {
    cancelled: AtomicBool,
}

#[derive(serde::Serialize, Clone)]
struct AudioExportProgress {
    current: usize,
    total: usize,
}

fn split_clauses_with_punct(text: &str) -> Vec<(String, Option<char>)> {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return Vec::new();
    }

    let bytes = clean.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;

    for (idx, ch) in clean.char_indices() {
        if !matches!(ch, ',' | ';' | ':' | '.' | '!' | '?' | '—' | '…') {
            continue;
        }
        let next_idx = idx + ch.len_utf8();
        let next_is_boundary = next_idx >= clean.len()
            || clean[next_idx..]
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(true);

        if !next_is_boundary {
            // Keep decimals like 3.14 or URLs inside the clause
            continue;
        }

        if ch == '.' {
            // Keep common abbreviations attached to the surrounding phrase for natural reading
            let before = clean[start..idx].trim_end();
            let last_word = before.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("");
            let lower = last_word.to_ascii_lowercase();
            let is_abbrev = matches!(
                lower.as_str(),
                "dr" | "mr" | "mrs" | "ms" | "prof" | "fig" | "figs" | "eq" | "eqs"
                    | "vol" | "no" | "pp" | "st" | "vs" | "cf" | "al" | "e.g" | "i.e"
            ) || (last_word.len() == 1 && last_word.chars().all(|c| c.is_ascii_alphabetic()));
            if is_abbrev && next_idx < clean.len() {
                continue;
            }
        }

        let clause = clean[start..idx].trim().to_string();
        if !clause.is_empty() {
            result.push((clause, Some(ch)));
        }
        start = next_idx;
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
    }

    if start < clean.len() {
        let tail = clean[start..].trim().to_string();
        if !tail.is_empty() {
            result.push((tail, None));
        }
    }

    result
}

fn espeak_ipa_to_misaki(ipa: &str, is_british: bool) -> String {
    let mut mapped = ipa
        .replace('\u{0303}', "")
        .replace("aɪ", "I")
        .replace("aʊ", "W")
        .replace("eɪ", "A")
        .replace("oʊ", "O")
        .replace("əʊ", "Q")
        .replace("ɔɪ", "Y")
        .replace("tʃ", "ʧ")
        .replace("dʒ", "ʤ")
        .replace("əl", "ᵊl")
        .replace("ɚ", "əɹ")
        .replace('r', "ɹ")
        .replace('x', "k")
        .replace('ç', "k")
        .replace('ɐ', "ə")
        .replace('ɬ', "l")
        .replace('ʔ', "t")
        .replace('ʲ', "");

    if is_british {
        mapped = mapped.replace("eə", "ɛː");
    }
    mapped
}

fn kokoro_vocab_id(ch: char) -> Option<i64> {
    let id = match ch {
        ';' => 1,
        ':' => 2,
        ',' => 3,
        '.' => 4,
        '!' => 5,
        '?' => 6,
        '—' => 9,
        '…' => 10,
        '"' => 11,
        '(' => 12,
        ')' => 13,
        '“' => 14,
        '”' => 15,
        ' ' => 16,
        '\u{0303}' => 17,
        'ʣ' => 18,
        'ʥ' => 19,
        'ʦ' => 20,
        'ʨ' => 21,
        'ᵝ' => 22,
        '\u{AB67}' => 23,
        'A' => 24,
        'I' => 25,
        'O' => 31,
        'Q' => 33,
        'S' => 35,
        'T' => 36,
        'W' => 39,
        'Y' => 41,
        'ᵊ' => 42,
        'a' => 43,
        'b' => 44,
        'c' => 45,
        'd' => 46,
        'e' => 47,
        'f' => 48,
        'h' => 50,
        'i' => 51,
        'j' => 52,
        'k' => 53,
        'l' => 54,
        'm' => 55,
        'n' => 56,
        'o' => 57,
        'p' => 58,
        'q' => 59,
        'r' => 60,
        's' => 61,
        't' => 62,
        'u' => 63,
        'v' => 64,
        'w' => 65,
        'x' => 66,
        'y' => 67,
        'z' => 68,
        'ɑ' => 69,
        'ɐ' => 70,
        'ɒ' => 71,
        'æ' => 72,
        'β' => 75,
        'ɔ' => 76,
        'ɕ' => 77,
        'ç' => 78,
        'ɖ' => 80,
        'ð' => 81,
        'ʤ' => 82,
        'ə' => 83,
        'ɚ' => 85,
        'ɛ' => 86,
        'ɜ' => 87,
        'ɟ' => 90,
        'ɡ' => 92,
        'ɥ' => 99,
        'ɨ' => 101,
        'ɪ' => 102,
        'ʝ' => 103,
        'ɯ' => 110,
        'ɰ' => 111,
        'ŋ' => 112,
        'ɳ' => 113,
        'ɲ' => 114,
        'ɴ' => 115,
        'ø' => 116,
        'ɸ' => 118,
        'θ' => 119,
        'œ' => 120,
        'ɹ' => 123,
        'ɾ' => 125,
        'ɻ' => 126,
        'ʁ' => 128,
        'ɽ' => 129,
        'ʂ' => 130,
        'ʃ' => 131,
        'ʈ' => 132,
        'ʧ' => 133,
        'ʊ' => 135,
        'ʋ' => 136,
        'ʌ' => 138,
        'ɣ' => 139,
        'ɤ' => 140,
        'χ' => 142,
        'ʎ' => 143,
        'ʒ' => 147,
        'ʔ' => 148,
        'ˈ' => 156,
        'ˌ' => 157,
        'ː' => 158,
        'ʰ' => 162,
        'ʲ' => 164,
        '↓' => 169,
        '→' => 171,
        '↗' => 172,
        '↘' => 173,
        'ᵻ' => 177,
        _ => return None,
    };
    Some(id)
}

fn phonemes_to_token_ids(phonemes: &str) -> Vec<i64> {
    let mut tokens = Vec::with_capacity(phonemes.len() + 2);
    tokens.push(0);
    for ch in phonemes.chars() {
        if let Some(id) = kokoro_vocab_id(ch) {
            tokens.push(id);
        }
    }
    tokens.push(0);
    tokens
}

impl KokoroEngine {
    fn load_from_paths(
        model_path: &Path,
        voices_path: &Path,
        espeak_lib_path: &Path,
        espeak_data_dir: &Path,
    ) -> Result<Self, String> {
        let voices_bytes =
            std::fs::read(voices_path).map_err(|e| format!("Failed to read voices.bin: {}", e))?;
        let (voices, _): (HashMap<String, Vec<Vec<Vec<f32>>>>, usize) =
            decode_from_slice(&voices_bytes, standard())
                .map_err(|e| format!("Failed to decode voices.bin: {}", e))?;

        let session = Session::builder()
            .map_err(|e| format!("Failed to create ORT session builder: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| format!("Failed to load Kokoro ONNX model: {}", e))?;

        let espeak = EspeakPhonemizer::load(espeak_lib_path, espeak_data_dir)?;

        Ok(Self {
            session: tauri::async_runtime::Mutex::new(session),
            voices,
            espeak: Mutex::new(espeak),
        })
    }

    fn text_to_phonemes(&self, text: &str, cfg: KokoroVoiceConfig) -> Result<String, String> {
        let clauses = split_clauses_with_punct(text);
        if clauses.is_empty() {
            return Ok(String::new());
        }

        let mut espeak = self
            .espeak
            .lock()
            .map_err(|_| "eSpeak phonemizer lock poisoned".to_string())?;
        let mut combined = String::new();

        for (clause, punct) in clauses {
            let ipa = espeak.phonemize_clause(&clause, cfg.espeak_lang)?;
            let misaki = espeak_ipa_to_misaki(&ipa, cfg.is_british);
            if !misaki.is_empty() {
                if !combined.is_empty() {
                    combined.push(' ');
                }
                combined.push_str(&misaki);
                if let Some(p) = punct {
                    combined.push(p);
                }
            }
        }
        Ok(combined)
    }

    async fn synthesize_text(&self, text: &str, cfg: KokoroVoiceConfig) -> Result<Vec<f32>, String> {
        let pack = self
            .voices
            .get(cfg.pack_key)
            .ok_or_else(|| format!("Voice pack '{}' not found in voices.bin", cfg.pack_key))?;

        let chunks = split_kokoro_segments(text, KOKORO_MAX_CLAUSE_CHARS);
        let mut all_samples = Vec::new();

        for chunk in chunks {
            let phonemes = self.text_to_phonemes(&chunk, cfg)?;
            let mut token_ids = phonemes_to_token_ids(&phonemes);
            if token_ids.len() <= 2 {
                continue;
            }
            if token_ids.len() > pack.len() {
                token_ids.truncate(pack.len() - 1);
                token_ids.push(0);
            }

            let ref_s = pack[token_ids.len() - 1]
                .first()
                .cloned()
                .ok_or("Empty voice style vector")?;

            let tokens_arr = Array::from_shape_vec((1, token_ids.len()), token_ids)
                .map_err(|e| format!("Invalid token tensor shape: {}", e))?;
            let style_arr = Array::from_shape_vec((1, ref_s.len()), ref_s)
                .map_err(|e| format!("Invalid style tensor shape: {}", e))?;
            let speed_arr = Array::from_vec(vec![1.0f32]);

            let options = RunOptions::new().map_err(|e| e.to_string())?;
            let mut session = self.session.lock().await;
            let outputs = session
                .run_async(
                    inputs![
                        "tokens" => TensorRef::from_array_view(&tokens_arr).map_err(|e| e.to_string())?,
                        "style" => TensorRef::from_array_view(&style_arr).map_err(|e| e.to_string())?,
                        "speed" => TensorRef::from_array_view(&speed_arr).map_err(|e| e.to_string())?,
                    ],
                    &options,
                )
                .map_err(|e| format!("Kokoro inference failed: {}", e))?
                .await
                .map_err(|e| format!("Kokoro async inference failed: {}", e))?;

            let (_, audio) = outputs["audio"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Failed to extract Kokoro audio tensor: {}", e))?;
            all_samples.extend_from_slice(audio);
        }

        Ok(all_samples)
    }
}

async fn get_or_init_kokoro(app: &AppHandle) -> Result<Arc<KokoroEngine>, String> {
    let state = app.state::<KokoroState>();
    let mut guard = state.engine.lock().await;
    if let Some(engine) = guard.as_ref() {
        return Ok(Arc::clone(engine));
    }

    let model_path = app
        .path()
        .resolve(
            "resources/kokoro/kokoro-v1.0.int8.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not resolve Kokoro model path: {}", e))?;

    let voices_path = app
        .path()
        .resolve(
            "resources/kokoro/voices.bin",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not resolve Kokoro voices path: {}", e))?;

    let espeak_lib_path = app
        .path()
        .resolve(
            "resources/piper/libespeak-ng.so.1",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not resolve libespeak-ng path: {}", e))?;

    let espeak_data_dir = app
        .path()
        .resolve(
            "resources/piper/espeak-ng-data",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not resolve espeak-ng-data path: {}", e))?;

    if !model_path.exists() || !voices_path.exists() {
        return Err("Kokoro model assets not found in resources/kokoro".to_string());
    }

    let engine = tauri::async_runtime::spawn_blocking(move || {
        KokoroEngine::load_from_paths(&model_path, &voices_path, &espeak_lib_path, &espeak_data_dir)
    })
    .await
    .map_err(|e| format!("Kokoro init task failed: {}", e))??;

    let engine = Arc::new(engine);
    *guard = Some(Arc::clone(&engine));
    Ok(engine)
}

pub async fn warmup_kokoro(app: &AppHandle) -> Result<(), String> {
    let _ = get_or_init_kokoro(app).await?;
    Ok(())
}

fn split_kokoro_segments(text: &str, max_chars: usize) -> Vec<String> {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return Vec::new();
    }
    if clean.len() <= max_chars {
        return vec![clean];
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    for word in clean.split(' ') {
        if !current.is_empty() && current.len() + 1 + word.len() > max_chars {
            segments.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn encode_wav_pcm16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let pcm = (clamped * 32767.0) as i16;
        buf.extend_from_slice(&pcm.to_le_bytes());
    }
    buf
}

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
                "--sentence_silence",
                "0.05",
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

struct RawPdfWord {
    el: PdfTextElement,
    had_leading_ws: bool,
    had_trailing_ws: bool,
}

fn normalize_pdf_text(raw: &str) -> String {
    raw.replace('\u{00A0}', " ")
        .replace('ﬀ', "ff")
        .replace('ﬁ', "fi")
        .replace('ﬂ', "fl")
        .replace('ﬃ', "ffi")
        .replace('ﬄ', "ffl")
}

fn group_words_into_lines(
    mut words: Vec<RawPdfWord>,
    layout_label: Option<String>,
    is_filtered: bool,
) -> Vec<PdfTextElement> {
    words.sort_by(|a, b| {
        a.el.y
            .partial_cmp(&b.el.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.el.x.partial_cmp(&b.el.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines = Vec::new();
    let mut current_line: Vec<RawPdfWord> = Vec::new();
    let y_tolerance = 5.0;

    for item in words {
        if current_line.is_empty() {
            current_line.push(item);
        } else {
            let baseline_y = current_line[0].el.y;
            if (item.el.y - baseline_y).abs() <= y_tolerance {
                current_line.push(item);
            } else {
                lines.push(merge_line_elements(
                    &mut current_line,
                    layout_label.clone(),
                    is_filtered,
                ));
                current_line.push(item);
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(merge_line_elements(
            &mut current_line,
            layout_label,
            is_filtered,
        ));
    }

    lines
}

fn process_page_with_layout(
    raw_elements: Vec<RawPdfWord>,
    boxes: &[LayoutBox],
) -> Vec<PdfTextElement> {
    if boxes.is_empty() {
        return group_words_into_lines(raw_elements, None, false);
    }

    let keep_boxes: Vec<&LayoutBox> = boxes
        .iter()
        .filter(|b| b.action == RegionAction::Keep)
        .collect();
    let drop_boxes: Vec<&LayoutBox> = boxes
        .iter()
        .filter(|b| b.action == RegionAction::Drop)
        .collect();

    let mut keep_buckets: Vec<Vec<RawPdfWord>> = (0..keep_boxes.len()).map(|_| Vec::new()).collect();
    let mut unassigned: Vec<RawPdfWord> = Vec::new();
    let mut filtered_buckets: HashMap<&'static str, Vec<RawPdfWord>> = HashMap::new();

    for word in raw_elements {
        let wx1 = word.el.x;
        let wy1 = word.el.y;
        let wx2 = wx1 + word.el.width.max(0.5);
        let wy2 = wy1 + word.el.height.max(0.5);
        let cx = 0.5 * (wx1 + wx2);
        let cy = 0.5 * (wy1 + wy2);

        let mut best_keep: Option<(usize, f32)> = None;
        for (idx, kb) in keep_boxes.iter().enumerate() {
            let ratio = intersection_ratio(wx1, wy1, wx2, wy2, kb);
            let center_in = cx >= kb.x1 && cx <= kb.x2 && cy >= kb.y1 && cy <= kb.y2;
            let eff = if center_in { ratio.max(0.5) } else { ratio };
            if eff > 0.20 && best_keep.map(|(_, b)| eff > b).unwrap_or(true) {
                best_keep = Some((idx, eff));
            }
        }

        let mut best_drop: Option<(&'static str, f32)> = None;
        for db in drop_boxes.iter() {
            let ratio = intersection_ratio(wx1, wy1, wx2, wy2, db);
            let center_in = cx >= db.x1 && cx <= db.x2 && cy >= db.y1 && cy <= db.y2;
            let eff = if center_in { ratio.max(0.5) } else { ratio };
            if eff >= 0.35 && best_drop.map(|(_, b)| eff > b).unwrap_or(true) {
                best_drop = Some((db.label, eff));
            }
        }

        let keep_score = best_keep.map(|(_, s)| s).unwrap_or(0.0);
        if let Some((drop_label, drop_score)) = best_drop {
            if drop_score > keep_score * 0.85 {
                filtered_buckets.entry(drop_label).or_default().push(word);
                continue;
            }
        }

        if let Some((keep_idx, _)) = best_keep {
            keep_buckets[keep_idx].push(word);
        } else {
            unassigned.push(word);
        }
    }

    // Build region list (detected Keep boxes + synthetic regions for unassigned words) for XY-cut reading order
    let mut region_items: Vec<(Option<String>, Vec<RawPdfWord>, RegionRect)> = Vec::new();
    for (idx, bucket) in keep_buckets.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        let kb = keep_boxes[idx];
        let r_idx = region_items.len();
        region_items.push((
            Some(kb.label.to_string()),
            bucket,
            RegionRect {
                index: r_idx,
                x1: kb.x1,
                y1: kb.y1,
                x2: kb.x2,
                y2: kb.y2,
            },
        ));
    }

    for line_el in group_words_into_lines(unassigned, None, false) {
        let r_idx = region_items.len();
        let rect = RegionRect {
            index: r_idx,
            x1: line_el.x,
            y1: line_el.y,
            x2: line_el.x + line_el.width,
            y2: line_el.y + line_el.height,
        };
        region_items.push((
            None,
            vec![RawPdfWord {
                el: line_el,
                had_leading_ws: false,
                had_trailing_ws: false,
            }],
            rect,
        ));
    }

    let mut rects: Vec<RegionRect> = region_items.iter().map(|(_, _, r)| *r).collect();
    xy_cut_sort(&mut rects);

    let mut ordered_lines = Vec::new();
    for rect in rects {
        let (label, words, _) = &mut region_items[rect.index];
        if words.len() == 1 && label.is_none() {
            // Already merged unassigned line
            ordered_lines.push(words.pop().unwrap().el);
        } else {
            let taken = std::mem::take(words);
            ordered_lines.extend(group_words_into_lines(taken, label.clone(), false));
        }
    }

    // Append filtered lines marked is_filtered = true so toggling Skip Headers off can still reveal them
    for (drop_label, words) in filtered_buckets {
        ordered_lines.extend(group_words_into_lines(
            words,
            Some(drop_label.to_string()),
            true,
        ));
    }

    ordered_lines
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

    let layout_detector = match get_or_init_layout_detector(app) {
        Ok(det) => Some(det),
        Err(err) => {
            eprintln!("Layout detector unavailable, using fallback order: {}", err);
            None
        }
    };

    println!("File loaded, starting the parsing");

    let mut pages_data = Vec::new();

    for (index, page) in document.pages().iter().enumerate() {
        let page_width = page.width().value;
        let page_height = page.height().value;
        let mut raw_elements = Vec::new();

        // 1. Collect all valid words on the page
        for object in page.objects().iter() {
            if let Some(text_object) = object.as_text_object() {
                let normalized = normalize_pdf_text(&text_object.text());
                if normalized.chars().any(|c| c.is_alphanumeric()) {
                    if let Ok(bounds) = text_object.bounds() {
                        let had_leading_ws = normalized
                            .chars()
                            .next()
                            .map(|c| c.is_whitespace())
                            .unwrap_or(false);
                        let had_trailing_ws = normalized
                            .chars()
                            .last()
                            .map(|c| c.is_whitespace())
                            .unwrap_or(false);
                        raw_elements.push(RawPdfWord {
                            el: PdfTextElement {
                                text: normalized.trim().to_string(),
                                x: bounds.left().value,
                                y: page_height - bounds.top().value,
                                width: bounds.width().value,
                                height: bounds.height().value,
                                layout_label: None,
                                is_filtered: false,
                            },
                            had_leading_ws,
                            had_trailing_ws,
                        });
                    }
                }
            }
        }

        // 2. Run PP-DocLayout ONNX layout detection on a 640x640 low-res render ("Detect & Intersect")
        let line_elements = if let Some(det) = layout_detector.as_ref() {
            let render_cfg = PdfRenderConfig::new()
                .set_fixed_size(LAYOUT_INPUT_SIZE as Pixels, LAYOUT_INPUT_SIZE as Pixels);
            match page.render_with_config(&render_cfg) {
                Ok(bitmap) => {
                    let rgba = bitmap.as_rgba_bytes();
                    match det.detect_from_rgba(
                        &rgba,
                        LAYOUT_INPUT_SIZE,
                        LAYOUT_INPUT_SIZE,
                        page_width,
                        page_height,
                    ) {
                        Ok(boxes) => process_page_with_layout(raw_elements, &boxes),
                        Err(err) => {
                            eprintln!("Layout detection failed on page {}: {}", index + 1, err);
                            group_words_into_lines(raw_elements, None, false)
                        }
                    }
                }
                Err(err) => {
                    eprintln!("Page render failed for layout on page {}: {}", index + 1, err);
                    group_words_into_lines(raw_elements, None, false)
                }
            }
        } else {
            group_words_into_lines(raw_elements, None, false)
        };

        pages_data.push(PdfPageData {
            page_number: index as u16 + 1,
            width: page_width,
            height: page_height,
            elements: line_elements,
        });
    }
    println!("Data sent to the front end");
    Ok(pages_data)
}

// Helper function to merge an array of words into a single line bounding box
fn merge_line_elements(
    line: &mut Vec<RawPdfWord>,
    layout_label: Option<String>,
    is_filtered: bool,
) -> PdfTextElement {
    // Sort words left-to-right
    line.sort_by(|a, b| {
        a.el.x
            .partial_cmp(&b.el.x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let first = &line.first().unwrap().el;
    let last = &line.last().unwrap().el;

    let min_x = first.x;
    let max_x = last.x + last.width; // The far right edge of the last word

    // Find the highest top edge and tallest height in the line
    let max_y = line.iter().map(|w| w.el.y).fold(f32::MIN, f32::max);
    let max_height = line.iter().map(|w| w.el.height).fold(f32::MIN, f32::max);

    // Combine text objects, preserving spaces from whitespace or horizontal gaps
    let mut combined_text = String::new();
    let mut prev_end_x: Option<f32> = None;
    let mut prev_height: f32 = 10.0;
    let mut prev_had_trailing = false;

    for w in line.iter() {
        if let Some(end_x) = prev_end_x {
            let gap = w.el.x - end_x;
            let starts_with_punct = w
                .el
                .text
                .chars()
                .next()
                .map(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']'))
                .unwrap_or(false);
            let threshold = if starts_with_punct {
                prev_height * 0.2
            } else {
                prev_height * 0.05
            };
            if prev_had_trailing || w.had_leading_ws || gap > threshold {
                combined_text.push(' ');
            }
        }
        combined_text.push_str(&w.el.text);
        prev_end_x = Some(w.el.x + w.el.width);
        if w.el.height > 0.0 {
            prev_height = w.el.height;
        }
        prev_had_trailing = w.had_trailing_ws;
    }

    let merged = PdfTextElement {
        text: combined_text,
        x: min_x,
        y: max_y,
        width: max_x - min_x,
        height: max_height,
        layout_label,
        is_filtered,
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
    if let Some(voice_cfg) = map_kokoro_voice(&voice_name) {
        let engine = get_or_init_kokoro(&app).await?;
        let samples = engine.synthesize_text(&text, voice_cfg).await?;
        let wav_bytes = encode_wav_pcm16(&samples, KOKORO_SAMPLE_RATE);
        return Ok(tauri::ipc::Response::new(wav_bytes));
    }

    let audio_bytes =
        tauri::async_runtime::spawn_blocking(move || synthesize_piper_blocking(&app, &text, &voice_name))
            .await
            .map_err(|e| format!("Audio synthesis task failed: {}", e))??;

    Ok(tauri::ipc::Response::new(audio_bytes))
}

#[tauri::command]
pub async fn save_text_file(path: String, content: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, content.as_bytes())
            .map_err(|e| format!("Failed to save file '{}': {}", path, e))
    })
    .await
    .map_err(|e| format!("Save task failed: {}", e))?
}

#[tauri::command]
pub fn cancel_audio_export(state: tauri::State<'_, ExportState>) {
    state.cancelled.store(true, Ordering::SeqCst);
}

fn write_wav_header(
    writer: &mut impl Write,
    sample_rate: u32,
    data_len: u32,
) -> std::io::Result<()> {
    writer.write_all(b"RIFF")?;
    writer.write_all(&(36u32.saturating_add(data_len)).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16u32.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&sample_rate.to_le_bytes())?;
    writer.write_all(&(sample_rate * 2).to_le_bytes())?;
    writer.write_all(&2u16.to_le_bytes())?;
    writer.write_all(&16u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

fn extract_wav_pcm_info(wav: &[u8]) -> Option<(u32, &[u8])> {
    if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    let mut pos = 12usize;
    while pos + 8 <= wav.len() {
        let chunk_id = &wav[pos..pos + 4];
        let chunk_size =
            u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let data_start = pos + 8;
        if chunk_id == b"data" {
            let data_end = data_start.saturating_add(chunk_size).min(wav.len());
            return Some((sample_rate, &wav[data_start..data_end]));
        }
        pos = data_start.saturating_add(chunk_size + (chunk_size % 2));
    }
    None
}

fn synthesize_piper_blocking(
    app: &AppHandle,
    text: &str,
    voice_name: &str,
) -> Result<Vec<u8>, String> {
    let temp_dir = app.path().temp_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let req_id = AUDIO_REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let out_path = temp_dir.join(format!("pdftospeech_{}_{}.wav", std::process::id(), req_id));

    let state = app.state::<PiperState>();
    let mut guard = state
        .worker
        .lock()
        .map_err(|_| "Piper worker lock poisoned".to_string())?;

    let needs_spawn = guard
        .as_ref()
        .map(|w| w.voice_name != voice_name)
        .unwrap_or(true);

    if needs_spawn {
        *guard = None;
        *guard = Some(PiperWorker::spawn(app, voice_name)?);
    }

    let first_attempt = guard.as_mut().unwrap().synthesize(text, &out_path);
    match first_attempt {
        Ok(bytes) => Ok(bytes),
        Err(_) => {
            let _ = std::fs::remove_file(&out_path);
            *guard = None;
            *guard = Some(PiperWorker::spawn(app, voice_name)?);
            guard.as_mut().unwrap().synthesize(text, &out_path)
        }
    }
}

#[tauri::command]
pub async fn export_audio_file(
    app: AppHandle,
    path: String,
    chunks: Vec<String>,
    voice_name: String,
) -> Result<(), String> {
    let export_state = app.state::<ExportState>();
    export_state.cancelled.store(false, Ordering::SeqCst);

    let total = chunks.len();
    if total == 0 {
        return Err("No readable text found to export.".to_string());
    }

    let mut file = std::fs::File::create(&path)
        .map_err(|e| format!("Failed to create output file '{}': {}", path, e))?;
    let mut sample_rate = KOKORO_SAMPLE_RATE;
    write_wav_header(&mut file, sample_rate, 0).map_err(|e| e.to_string())?;
    let mut total_pcm_bytes: u64 = 0;

    let kokoro_cfg = map_kokoro_voice(&voice_name);
    let kokoro_engine = if kokoro_cfg.is_some() {
        Some(get_or_init_kokoro(&app).await?)
    } else {
        None
    };

    for (idx, chunk) in chunks.into_iter().enumerate() {
        if app.state::<ExportState>().cancelled.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err("Export cancelled.".to_string());
        }

        if let (Some(cfg), Some(engine)) = (kokoro_cfg, kokoro_engine.as_ref()) {
            let samples = engine.synthesize_text(&chunk, cfg).await?;
            for sample in samples {
                let pcm = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                file.write_all(&pcm.to_le_bytes())
                    .map_err(|e| e.to_string())?;
                total_pcm_bytes += 2;
            }
        } else {
            let app_clone = app.clone();
            let voice_clone = voice_name.clone();
            let wav = tauri::async_runtime::spawn_blocking(move || {
                synthesize_piper_blocking(&app_clone, &chunk, &voice_clone)
            })
            .await
            .map_err(|e| format!("Audio export task failed: {}", e))??;

            if let Some((sr, pcm_data)) = extract_wav_pcm_info(&wav) {
                sample_rate = sr;
                file.write_all(pcm_data).map_err(|e| e.to_string())?;
                total_pcm_bytes += pcm_data.len() as u64;
            }
        }

        let _ = app.emit(
            "audio-export-progress",
            AudioExportProgress {
                current: idx + 1,
                total,
            },
        );
    }

    let clamped_data_len = total_pcm_bytes.min(u32::MAX as u64 - 36) as u32;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    write_wav_header(&mut file, sample_rate, clamped_data_len).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_detector::LayoutDetector;

    #[test]
    fn kokoro_runtime_synthesis_produces_wav() {
        let model_path = Path::new("resources/kokoro/kokoro-v1.0.int8.onnx");
        let voices_path = Path::new("resources/kokoro/voices.bin");
        let espeak_lib = Path::new("resources/piper/libespeak-ng.so.1");
        let espeak_data = Path::new("resources/piper/espeak-ng-data");
        if !model_path.exists() || !voices_path.exists() || !espeak_lib.exists() {
            return;
        }

        let engine = KokoroEngine::load_from_paths(model_path, voices_path, espeak_lib, espeak_data)
            .expect("KokoroEngine::load_from_paths should succeed");

        let cfg = map_kokoro_voice("kokoro_af_heart").unwrap();
        let sample_text =
            "In 2024, we don't misspell IT, AI, PDF, or experimental results.";
        let phonemes = engine
            .text_to_phonemes(sample_text, cfg)
            .expect("phonemization should succeed");
        println!("US Misaki phonemes: {}", phonemes);
        assert!(!phonemes.is_empty());

        let uk_cfg = map_kokoro_voice("kokoro_bf_emma").unwrap();
        let uk_phonemes = engine
            .text_to_phonemes("Hello world! Water bath car.", uk_cfg)
            .expect("UK phonemization should succeed");
        println!("UK Misaki phonemes: {}", uk_phonemes);
        assert!(uk_phonemes.contains('Q'));

        let t0 = std::time::Instant::now();
        let samples = tauri::async_runtime::block_on(engine.synthesize_text(sample_text, cfg))
            .expect("Kokoro synthesis should succeed");
        let elapsed = t0.elapsed();

        assert!(!samples.is_empty(), "Kokoro returned empty audio");
        let wav = encode_wav_pcm16(&samples, KOKORO_SAMPLE_RATE);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        println!(
            "Kokoro test: {} samples ({:.2}s audio) synthesized in {:?}",
            samples.len(),
            samples.len() as f32 / KOKORO_SAMPLE_RATE as f32,
            elapsed
        );
    }

    #[test]
    fn merge_line_preserves_spaces_and_ligatures() {
        let mut words = vec![
            RawPdfWord {
                el: PdfTextElement {
                    text: normalize_pdf_text("ﬁrst "),
                    x: 10.0,
                    y: 100.0,
                    width: 30.0,
                    height: 10.0,
                    layout_label: None,
                    is_filtered: false,
                },
                had_leading_ws: false,
                had_trailing_ws: true,
            },
            RawPdfWord {
                el: PdfTextElement {
                    text: "result".to_string(),
                    x: 40.0, // gap == 0 due to trailing space width inside bounds
                    y: 100.0,
                    width: 30.0,
                    height: 10.0,
                    layout_label: None,
                    is_filtered: false,
                },
                had_leading_ws: false,
                had_trailing_ws: false,
            },
        ];
        // Re-trim as parse_pdf_sync does
        words[0].el.text = words[0].el.text.trim().to_string();
        let merged = merge_line_elements(&mut words, Some("text".to_string()), false);
        assert_eq!(merged.text, "first result");
        assert_eq!(merged.layout_label.as_deref(), Some("text"));
        assert!(!merged.is_filtered);
    }

    #[test]
    fn detect_and_intersect_sample_pdf() {
        let pdf_path = Path::new("/tmp/test_paper.pdf");
        let model_path = Path::new("resources/layout/pp-doclayout-l.onnx");
        let labels_path = Path::new("resources/layout/labels.json");
        let pdfium_path = Path::new("resources/libpdfium.so");
        if !pdf_path.exists() || !model_path.exists() || !pdfium_path.exists() {
            return;
        }

        let pdfium = Pdfium::new(
            Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("resources"))
                .expect("bind pdfium"),
        );
        let document = pdfium.load_pdf_from_file(pdf_path, None).expect("load pdf");
        let page = document.pages().get(0).expect("page 0");
        let det = LayoutDetector::load(model_path, labels_path).expect("load layout");

        let mut raw_elements = Vec::new();
        for object in page.objects().iter() {
            if let Some(text_object) = object.as_text_object() {
                let normalized = normalize_pdf_text(&text_object.text());
                if normalized.chars().any(|c| c.is_alphanumeric()) {
                    if let Ok(bounds) = text_object.bounds() {
                        raw_elements.push(RawPdfWord {
                            el: PdfTextElement {
                                text: normalized.trim().to_string(),
                                x: bounds.left().value,
                                y: page.height().value - bounds.top().value,
                                width: bounds.width().value,
                                height: bounds.height().value,
                                layout_label: None,
                                is_filtered: false,
                            },
                            had_leading_ws: false,
                            had_trailing_ws: false,
                        });
                    }
                }
            }
        }

        let render_cfg = PdfRenderConfig::new()
            .set_fixed_size(LAYOUT_INPUT_SIZE as Pixels, LAYOUT_INPUT_SIZE as Pixels);
        let bitmap = page.render_with_config(&render_cfg).expect("render");
        let rgba = bitmap.as_rgba_bytes();

        let t0 = std::time::Instant::now();
        let boxes = det
            .detect_from_rgba(
                &rgba,
                LAYOUT_INPUT_SIZE,
                LAYOUT_INPUT_SIZE,
                page.width().value,
                page.height().value,
            )
            .expect("detect");
        let elapsed = t0.elapsed();

        println!("Detected {} boxes in {:?}:", boxes.len(), elapsed);
        for b in &boxes {
            println!(
                "  {:?} ({:.2}) [{:.1}, {:.1}, {:.1}, {:.1}] -> {:?}",
                b.label, b.score, b.x1, b.y1, b.x2, b.y2, b.action
            );
        }

        let lines = process_page_with_layout(raw_elements, &boxes);
        for l in &lines {
            println!(
                "  filtered={} label={:?} text={:?}",
                l.is_filtered, l.layout_label, l.text
            );
        }

        let kept: Vec<&str> = lines
            .iter()
            .filter(|l| !l.is_filtered)
            .map(|l| l.text.as_str())
            .collect();
        let filtered: Vec<&str> = lines
            .iter()
            .filter(|l| l.is_filtered)
            .map(|l| l.text.as_str())
            .collect();

        assert!(!kept.is_empty(), "Should keep body lines");
        assert!(!filtered.is_empty(), "Should filter header/footer/number lines");

        // Verify left column appears before right column
        let left_pos = kept.iter().position(|t| t.contains("First column")).unwrap();
        let right_pos = kept.iter().position(|t| t.contains("Right column")).unwrap();
        assert!(left_pos < right_pos, "Left column should precede right column");
    }
}

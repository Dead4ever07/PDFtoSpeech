use ort::{inputs, session::Session, value::TensorRef};
use serde::Deserialize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

pub const LAYOUT_INPUT_SIZE: usize = 640;
pub const LAYOUT_DEFAULT_THRESHOLD: f32 = 0.45;

pub const LAYOUT_LABELS: [&str; 23] = [
    "paragraph_title", // 0
    "image",           // 1
    "text",            // 2
    "number",          // 3
    "abstract",        // 4
    "content",         // 5
    "figure_title",    // 6
    "formula",         // 7
    "table",           // 8
    "table_title",     // 9
    "reference",       // 10
    "doc_title",       // 11
    "footnote",        // 12
    "header",          // 13
    "algorithm",       // 14
    "footer",          // 15
    "seal",            // 16
    "chart_title",     // 17
    "chart",           // 18
    "formula_number",  // 19
    "header_image",    // 20
    "footer_image",    // 21
    "aside_text",      // 22
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAction {
    Keep,
    Drop,
}

#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub class_id: usize,
    pub label: &'static str,
    pub score: f32,
    /// Bounding box in PDF top-down page coordinates
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub action: RegionAction,
}

#[derive(Deserialize)]
struct LabelsFile {
    labels: Vec<String>,
    size: Option<[usize; 2]>,
    threshold: Option<f32>,
}

#[derive(Default)]
pub struct LayoutState {
    detector: Mutex<Option<Arc<LayoutDetector>>>,
}

pub struct LayoutDetector {
    session: Mutex<Session>,
    labels: Vec<String>,
    input_size: usize,
    threshold: f32,
}

impl LayoutDetector {
    pub fn load(model_path: &Path, labels_path: &Path) -> Result<Self, String> {
        let mut labels: Vec<String> = LAYOUT_LABELS.iter().map(|s| (*s).to_string()).collect();
        let mut input_size = LAYOUT_INPUT_SIZE;
        let mut threshold = LAYOUT_DEFAULT_THRESHOLD;

        if labels_path.exists() {
            if let Ok(raw) = std::fs::read_to_string(labels_path) {
                if let Ok(parsed) = serde_json::from_str::<LabelsFile>(&raw) {
                    if !parsed.labels.is_empty() {
                        labels = parsed.labels;
                    }
                    if let Some([h, _w]) = parsed.size {
                        if h > 0 {
                            input_size = h;
                        }
                    }
                    if let Some(t) = parsed.threshold {
                        if t > 0.0 && t < 1.0 {
                            threshold = t;
                        }
                    }
                }
            }
        }

        let session = Session::builder()
            .map_err(|e| format!("Failed to create layout ORT session builder: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| format!("Failed to load PP-DocLayout ONNX model: {}", e))?;

        Ok(Self {
            session: Mutex::new(session),
            labels,
            input_size,
            threshold,
        })
    }

    pub fn classify_label(label: &str) -> RegionAction {
        match label {
            "header" | "footer" | "number" | "footnote" | "aside_text" | "header_image"
            | "footer_image" | "figure_title" | "table_title" | "chart_title"
            | "formula_number" | "seal" | "image" | "table" | "chart" => RegionAction::Drop,
            _ => RegionAction::Keep,
        }
    }

    pub fn detect_from_rgba(
        &self,
        rgba: &[u8],
        img_w: usize,
        img_h: usize,
        page_width: f32,
        page_height: f32,
    ) -> Result<Vec<LayoutBox>, String> {
        let size = self.input_size;
        if img_w != size || img_h != size || rgba.len() < size * size * 4 {
            return Err(format!(
                "Expected {}x{} RGBA image (got {}x{}, {} bytes)",
                size,
                size,
                img_w,
                img_h,
                rgba.len()
            ));
        }

        // PP-DocLayout-L preprocessing: RGB in [0.0, 1.0], mean=[0,0,0], std=[1,1,1], CHW
        let plane = size * size;
        let mut chw = vec![0.0f32; 3 * plane];
        let (r_plane, gb) = chw.split_at_mut(plane);
        let (g_plane, b_plane) = gb.split_at_mut(plane);

        for i in 0..plane {
            let p = i * 4;
            r_plane[i] = rgba[p] as f32 / 255.0;
            g_plane[i] = rgba[p + 1] as f32 / 255.0;
            b_plane[i] = rgba[p + 2] as f32 / 255.0;
        }

        let image_arr = ndarray::Array::from_shape_vec((1, 3, size, size), chw)
            .map_err(|e| format!("Invalid layout image tensor: {}", e))?;
        let im_shape_arr = ndarray::Array::from_shape_vec((1, 2), vec![size as f32, size as f32])
            .map_err(|e| e.to_string())?;
        let scale_factor_arr = ndarray::Array::from_shape_vec((1, 2), vec![1.0f32, 1.0f32])
            .map_err(|e| e.to_string())?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| "Layout detector lock poisoned".to_string())?;

        let outputs = session
            .run(inputs![
                "im_shape" => TensorRef::from_array_view(&im_shape_arr).map_err(|e| e.to_string())?,
                "image" => TensorRef::from_array_view(&image_arr).map_err(|e| e.to_string())?,
                "scale_factor" => TensorRef::from_array_view(&scale_factor_arr).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("Layout inference failed: {}", e))?;

        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract layout boxes tensor: {}", e))?;

        let rows = if shape.len() == 2 && shape[1] >= 6 {
            shape[0] as usize
        } else {
            data.len() / 6
        };
        let cols = if shape.len() == 2 && shape[1] >= 6 {
            shape[1] as usize
        } else {
            6
        };

        let scale_x = page_width / size as f32;
        let scale_y = page_height / size as f32;
        let mut boxes = Vec::new();

        for r in 0..rows {
            let off = r * cols;
            let class_id = data[off] as isize;
            let score = data[off + 1];
            if class_id < 0 || score < self.threshold {
                continue;
            }
            let cid = class_id as usize;
            let label = self
                .labels
                .get(cid)
                .map(|s| s.as_str())
                .unwrap_or("text");
            let static_label: &'static str = LAYOUT_LABELS
                .iter()
                .copied()
                .find(|&l| l == label)
                .unwrap_or("text");

            let x1 = (data[off + 2] * scale_x).clamp(0.0, page_width);
            let y1 = (data[off + 3] * scale_y).clamp(0.0, page_height);
            let x2 = (data[off + 4] * scale_x).clamp(0.0, page_width);
            let y2 = (data[off + 5] * scale_y).clamp(0.0, page_height);

            if x2 > x1 && y2 > y1 {
                boxes.push(LayoutBox {
                    class_id: cid,
                    label: static_label,
                    score,
                    x1,
                    y1,
                    x2,
                    y2,
                    action: Self::classify_label(static_label),
                });
            }
        }

        Ok(boxes)
    }
}

pub fn get_or_init_layout_detector(app: &AppHandle) -> Result<Arc<LayoutDetector>, String> {
    let state = app.state::<LayoutState>();
    let mut guard = state
        .detector
        .lock()
        .map_err(|_| "Layout state lock poisoned".to_string())?;
    if let Some(det) = guard.as_ref() {
        return Ok(Arc::clone(det));
    }

    let model_path = app
        .path()
        .resolve(
            "resources/layout/pp-doclayout-l.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not resolve PP-DocLayout model path: {}", e))?;

    let labels_path = app
        .path()
        .resolve(
            "resources/layout/labels.json",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not resolve PP-DocLayout labels path: {}", e))?;

    if !model_path.exists() {
        return Err("PP-DocLayout ONNX model not found in resources/layout".to_string());
    }

    let detector = Arc::new(LayoutDetector::load(&model_path, &labels_path)?);
    *guard = Some(Arc::clone(&detector));
    Ok(detector)
}

pub fn warmup_layout(app: &AppHandle) -> Result<(), String> {
    let _ = get_or_init_layout_detector(app)?;
    Ok(())
}

/// Computes what fraction of `(wx1, wy1, wx2, wy2)` is covered by `region`.
pub fn intersection_ratio(wx1: f32, wy1: f32, wx2: f32, wy2: f32, region: &LayoutBox) -> f32 {
    let word_w = (wx2 - wx1).max(0.01);
    let word_h = (wy2 - wy1).max(0.01);
    let word_area = word_w * word_h;

    let ix1 = wx1.max(region.x1);
    let iy1 = wy1.max(region.y1);
    let ix2 = wx2.min(region.x2);
    let iy2 = wy2.min(region.y2);

    if ix2 <= ix1 || iy2 <= iy1 {
        return 0.0;
    }
    ((ix2 - ix1) * (iy2 - iy1)) / word_area
}

#[derive(Clone, Copy)]
pub struct RegionRect {
    pub index: usize,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// Recursively orders regions in natural multi-column reading order (top-to-bottom bands, left-to-right columns).
pub fn xy_cut_sort(rects: &mut [RegionRect]) {
    if rects.len() <= 1 {
        return;
    }

    // 1. Try horizontal cut (top-to-bottom bands separated by clear vertical whitespace)
    rects.sort_by(|a, b| a.y1.partial_cmp(&b.y1).unwrap_or(std::cmp::Ordering::Equal));
    let mut max_y2 = rects[0].y2;
    for i in 1..rects.len() {
        if rects[i].y1 > max_y2 + 2.0 {
            let (top, bottom) = rects.split_at_mut(i);
            xy_cut_sort(top);
            xy_cut_sort(bottom);
            return;
        }
        max_y2 = max_y2.max(rects[i].y2);
    }

    // 2. Try vertical cut (left-to-right columns separated by clear horizontal gutter)
    rects.sort_by(|a, b| a.x1.partial_cmp(&b.x1).unwrap_or(std::cmp::Ordering::Equal));
    let mut max_x2 = rects[0].x2;
    for i in 1..rects.len() {
        if rects[i].x1 > max_x2 + 4.0 {
            let (left, right) = rects.split_at_mut(i);
            xy_cut_sort(left);
            xy_cut_sort(right);
            return;
        }
        max_x2 = max_x2.max(rects[i].x2);
    }

    // 3. Fallback within overlapping cluster: top-to-bottom, then left-to-right
    rects.sort_by(|a, b| {
        a.y1.partial_cmp(&b.y1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.x1.partial_cmp(&b.x1).unwrap_or(std::cmp::Ordering::Equal))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pp_doclayout_model_loads_and_reports_io() {
        let model_path = Path::new("resources/layout/pp-doclayout-l.onnx");
        let labels_path = Path::new("resources/layout/labels.json");
        if !model_path.exists() {
            return;
        }
        let detector = LayoutDetector::load(model_path, labels_path)
            .expect("PP-DocLayout-L should load cleanly");
        let blank_rgba = vec![255u8; 640 * 640 * 4];
        let boxes = detector
            .detect_from_rgba(&blank_rgba, 640, 640, 612.0, 792.0)
            .expect("inference should run");
        assert_eq!(boxes.len(), 0);
    }

    #[test]
    fn xy_cut_orders_title_then_two_columns() {
        let mut rects = vec![
            RegionRect { index: 0, x1: 320.0, y1: 120.0, x2: 560.0, y2: 700.0 }, // right col
            RegionRect { index: 1, x1: 50.0, y1: 40.0, x2: 560.0, y2: 90.0 },    // title
            RegionRect { index: 2, x1: 50.0, y1: 120.0, x2: 290.0, y2: 700.0 },  // left col
        ];
        xy_cut_sort(&mut rects);
        let order: Vec<usize> = rects.iter().map(|r| r.index).collect();
        assert_eq!(order, vec![1, 2, 0]);
    }
}

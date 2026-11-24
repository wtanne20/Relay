//! What sd-server can run: its image recipes (a model family and the files
//! it needs) and the model files in the image model folders, plus guesses
//! for which file goes in which slot.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Slot {
    pub id: String,
    pub label: String,
    pub required: bool,
    /// Folders (categories) its files come from.
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub model_url: Option<String>,
    pub slots: Vec<Slot>,
    /// Groups of slots of which at least one must be filled.
    pub required_any: Vec<Vec<String>>,
    /// width, height, steps, cfg_scale ... as sd-server's recipe gives them.
    pub defaults: Value,
    /// Can follow edit instructions with the previous picture as reference.
    pub edit: bool,
    /// Can start from a picture (image to image).
    pub img2img: bool,
    pub size_multiple: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelFile {
    pub id: String,
    pub name: String,
    pub category: String,
}

/// A model found in the folders that can be set up in one tap.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Suggestion {
    pub recipe: String,
    pub name: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Catalog {
    pub recipes: Vec<Recipe>,
    pub files: Vec<ModelFile>,
    pub suggestions: Vec<Suggestion>,
    pub errors: Vec<String>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn feature(r: &Value, name: &str) -> bool {
    if let Some(b) = r
        .get("effective_features")
        .and_then(|f| f.get(name))
        .and_then(Value::as_bool)
    {
        return b;
    }
    let listed = |k: &str| {
        r.get(k)
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(name)))
    };
    match name {
        // On unless turned off.
        "img2img" => !listed("disabled_features"),
        _ => listed("features") && !listed("disabled_features"),
    }
}

/// Image recipes from `GET /sdcpp/v1/recipes` (video ones are left out).
pub fn parse_recipes(v: &Value) -> Vec<Recipe> {
    let empty = Vec::new();
    let mut out: Vec<Recipe> = v
        .get("recipes")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter(|r| r.pointer("/defaults/video_frames").is_none())
        .filter_map(|r| {
            let id = s(r, "id");
            if id.is_empty() {
                return None;
            }
            let slots = r
                .get("slots")
                .and_then(Value::as_array)
                .unwrap_or(&empty)
                .iter()
                .map(|sl| {
                    let id = s(sl, "id");
                    let mut cats: Vec<String> = sl
                        .get("categories")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|c| c.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    if cats.is_empty() {
                        let c = s(sl, "category");
                        cats = if c.is_empty() {
                            default_categories(&id)
                        } else {
                            vec![c]
                        };
                    }
                    let label = match s(sl, "label") {
                        l if l.is_empty() => default_label(&id),
                        l => l,
                    };
                    Slot {
                        label,
                        required: sl.get("required").and_then(Value::as_bool).unwrap_or(false),
                        categories: cats,
                        id,
                    }
                })
                .collect::<Vec<_>>();
            let required_any = r
                .get("required_any")
                .and_then(Value::as_array)
                .map(|g| {
                    g.iter()
                        .filter_map(|x| {
                            x.as_array().map(|a| {
                                a.iter()
                                    .filter_map(|y| y.as_str().map(str::to_string))
                                    .collect()
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Recipe {
                name: match s(r, "name") {
                    n if n.is_empty() => id.clone(),
                    n => n,
                },
                description: s(r, "description"),
                model_url: r
                    .get("model_url")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                slots,
                required_any,
                defaults: r.get("defaults").cloned().unwrap_or(Value::Null),
                edit: feature(r, "ref_images"),
                img2img: feature(r, "img2img"),
                size_multiple: r.get("size_multiple").and_then(Value::as_u64).unwrap_or(8) as u32,
                id,
            })
        })
        .collect();
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}

/// Files from `GET /sdcpp/v1/model-files`.
pub fn parse_files(v: &Value) -> Vec<ModelFile> {
    let empty = Vec::new();
    v.get("files")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter_map(|f| {
            let id = s(f, "id");
            (!id.is_empty()).then(|| ModelFile {
                name: match s(f, "name") {
                    n if n.is_empty() => id.clone(),
                    n => n,
                },
                category: s(f, "category"),
                id,
            })
        })
        .collect()
}

pub fn parse_errors(v: &Value) -> Vec<String> {
    v.get("errors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| {
                    let m = s(e, "message");
                    let f = s(e, "file");
                    match (f.is_empty(), m.is_empty()) {
                        (_, true) => e.to_string(),
                        (true, false) => m,
                        (false, false) => format!("{f}: {m}"),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn default_categories(slot: &str) -> Vec<String> {
    let c: &[&str] = match slot {
        "model" => &["checkpoints"],
        "diffusion_model" | "high_noise_diffusion_model" | "uncond_diffusion_model" => {
            &["diffusion_models"]
        }
        "clip_l" | "clip_g" | "t5xxl" | "llm" | "audio_encoder" | "embeddings_connectors" => {
            &["text_encoders"]
        }
        "clip_vision" => &["clip_vision"],
        "llm_vision" => &["text_encoders", "clip_vision"],
        "vae" | "audio_vae" => &["vae"],
        "tae" | "taesd" => &["vae", "vae_approx"],
        "tokenizer" => &["tokenizers"],
        "control_net" => &["controlnet"],
        other => return vec![other.to_string()],
    };
    c.iter().map(|s| s.to_string()).collect()
}

fn default_label(slot: &str) -> String {
    match slot {
        "model" => "Checkpoint".into(),
        "diffusion_model" => "Diffusion model".into(),
        "clip_l" => "CLIP-L".into(),
        "clip_g" => "CLIP-G".into(),
        "t5xxl" => "T5-XXL".into(),
        "llm" => "Text encoder".into(),
        "vae" => "VAE".into(),
        "tae" | "taesd" => "TAE".into(),
        other => other.replace('_', " "),
    }
}

const MODEL_EXTS: &[&str] = &[
    "gguf",
    "safetensors",
    "sft",
    "ckpt",
    "pt",
    "pth",
    "bin",
    "json",
];

fn strip_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((a, ext)) if MODEL_EXTS.contains(&ext.to_ascii_lowercase().as_str()) => a,
        _ => name,
    }
}

/// Lowercase words and numbers: "Qwen3-4B_Q8.gguf" -> qwen 3 4 b q 8.
fn tokens(s: &str) -> Vec<String> {
    let stem = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let stem = strip_ext(stem);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut digit = false;
    for ch in stem.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let d = ch.is_ascii_digit();
        if !cur.is_empty() && d != digit {
            out.push(std::mem::take(&mut cur));
        }
        digit = d;
        cur.push(ch.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn compact(s: &str) -> String {
    let stem = strip_ext(s.rsplit(['/', '\\']).next().unwrap_or(s));
    stem.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The recipe's family name without generic words ("sdxl-checkpoint" -> "sdxl").
fn recipe_core(id: &str) -> String {
    let words: Vec<&str> = id
        .split(['-', '_'])
        .filter(|w| !matches!(*w, "checkpoint" | "dev" | "base"))
        .collect();
    compact(&words.join(""))
}

/// How well a file name fits a slot of a recipe (0 = not at all).
fn slot_score(recipe: &Recipe, slot: &Slot, file: &ModelFile) -> i32 {
    let ft = tokens(&file.name);
    let mut score = 0;
    let slot_words: Vec<String> = tokens(&slot.id)
        .into_iter()
        .chain(tokens(&slot.label))
        .filter(|w| !matches!(w.as_str(), "model" | "text" | "encoder" | "diffusion"))
        .collect();
    for w in &slot_words {
        if ft.contains(w) {
            score += 2;
        }
    }
    for w in tokens(&recipe.id) {
        if w.len() > 1 && !matches!(w.as_str(), "checkpoint") && ft.contains(&w) {
            score += 1;
        }
    }
    let primary = matches!(slot.id.as_str(), "model" | "diffusion_model");
    if primary {
        let core = recipe_core(&recipe.id);
        if !core.is_empty() && compact(&file.name).contains(&core) {
            score += 3 + core.len() as i32;
        }
    }
    if slot.id == "vae" && ft.first().is_some_and(|w| w == "ae") {
        score += 1;
    }
    score
}

/// Picks a file for each slot: the only candidate, or the best-named one.
pub fn fill_slots(recipe: &Recipe, files: &[ModelFile]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for slot in &recipe.slots {
        let cands: Vec<&ModelFile> = files
            .iter()
            .filter(|f| slot.categories.contains(&f.category))
            .collect();
        let best = cands
            .iter()
            .map(|f| (slot_score(recipe, slot, f), *f))
            .filter(|(s, _)| *s > 0)
            .max_by_key(|(s, _)| *s)
            .map(|(_, f)| f);
        // Optional slots stay empty unless a file clearly fits.
        let pick = best.or_else(|| (slot.required && cands.len() == 1).then(|| cands[0]));
        if let Some(f) = pick {
            out.insert(slot.id.clone(), f.id.clone());
        }
    }
    // A TAE is only needed when there's no VAE.
    if out.contains_key("vae") {
        out.remove("tae");
    }
    out
}

/// Whether every required slot (and each required_any group) is filled.
pub fn complete(recipe: &Recipe, files: &BTreeMap<String, String>) -> bool {
    let has = |id: &str| files.get(id).is_some_and(|v| !v.is_empty());
    recipe
        .slots
        .iter()
        .filter(|s| s.required)
        .all(|s| has(&s.id))
        && recipe
            .required_any
            .iter()
            .all(|g| g.iter().any(|id| has(id)))
}

/// One ready-to-use setup per main model file whose name says its family.
pub fn suggest(recipes: &[Recipe], files: &[ModelFile]) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for f in files {
        if !matches!(f.category.as_str(), "checkpoints" | "diffusion_models") {
            continue;
        }
        let mut best: Option<(i32, &Recipe)> = None;
        for r in recipes {
            let Some(slot) = r
                .slots
                .iter()
                .find(|s| s.required && s.categories.contains(&f.category))
            else {
                continue;
            };
            if !matches!(slot.id.as_str(), "model" | "diffusion_model") {
                continue;
            }
            let core = recipe_core(&r.id);
            if core.is_empty() || !compact(&f.name).contains(&core) {
                continue;
            }
            let score = core.len() as i32;
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, r));
            }
        }
        // Plain checkpoints rarely say their family: "xl" in the name means SDXL.
        if best.is_none() && f.category == "checkpoints" {
            let c = compact(&f.name);
            let id = if c.contains("xl") || c.contains("pony") || c.contains("illustrious") {
                "sdxl-checkpoint"
            } else {
                "sd1-checkpoint"
            };
            best = recipes.iter().find(|r| r.id == id).map(|r| (0, r));
        }
        let Some((_, r)) = best else { continue };
        let mut picked = fill_slots(r, files);
        if let Some(slot) = r
            .slots
            .iter()
            .find(|s| matches!(s.id.as_str(), "model" | "diffusion_model"))
        {
            picked.insert(slot.id.clone(), f.id.clone());
        }
        if complete(r, &picked) {
            out.push(Suggestion {
                recipe: r.id.clone(),
                name: pretty_name(&f.name),
                files: picked,
            });
        }
    }
    out
}

/// "z_image_turbo-Q4_K.gguf" -> "z image turbo Q4_K".
pub fn pretty_name(file: &str) -> String {
    let stem = strip_ext(file.rsplit(['/', '\\']).next().unwrap_or(file));
    stem.replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn recipes() -> Vec<Recipe> {
        parse_recipes(&json!({"recipes": [
            {"id": "z-image-turbo", "name": "Z-Image Turbo", "slots": [
                {"id": "diffusion_model", "required": true, "categories": ["diffusion_models"]},
                {"id": "llm", "required": true, "label": "Qwen 3 Text Encoder", "categories": ["text_encoders"]},
                {"id": "vae", "required": false, "categories": ["vae"]},
                {"id": "tae", "required": false, "categories": ["vae", "vae_approx"]}
            ], "required_any": [["vae", "tae"]], "defaults": {"steps": 9, "cfg_scale": 1, "width": 1024, "height": 1024}},
            {"id": "z-image", "name": "Z-Image", "slots": [
                {"id": "diffusion_model", "required": true, "categories": ["diffusion_models"]},
                {"id": "llm", "required": true, "categories": ["text_encoders"]},
                {"id": "vae", "required": false, "categories": ["vae"]}
            ], "required_any": [["vae"]]},
            {"id": "flux1-kontext", "name": "FLUX.1 Kontext dev", "features": ["ref_images"], "slots": [
                {"id": "diffusion_model", "required": true},
                {"id": "clip_l", "required": true},
                {"id": "t5xxl", "required": true},
                {"id": "vae", "required": false}
            ], "required_any": [["vae"]]},
            {"id": "sdxl-checkpoint", "name": "SDXL", "slots": [{"id": "model", "required": true}]},
            {"id": "sd1-checkpoint", "name": "SD 1.x", "slots": [{"id": "model", "required": true}]},
            {"id": "wan22-t2v", "name": "Wan", "defaults": {"video_frames": 33}, "slots": []}
        ]}))
    }

    fn files() -> Vec<ModelFile> {
        parse_files(&json!({"files": [
            {"id": "diffusion_models/z_image_turbo-Q4_K.gguf", "name": "z_image_turbo-Q4_K.gguf", "category": "diffusion_models"},
            {"id": "diffusion_models/flux1-kontext-dev-Q4_0.gguf", "name": "flux1-kontext-dev-Q4_0.gguf", "category": "diffusion_models"},
            {"id": "text_encoders/Qwen3-4B-Instruct-2507-Q4_K_M.gguf", "name": "Qwen3-4B-Instruct-2507-Q4_K_M.gguf", "category": "text_encoders"},
            {"id": "text_encoders/clip_l.safetensors", "name": "clip_l.safetensors", "category": "text_encoders"},
            {"id": "text_encoders/t5xxl_fp16.safetensors", "name": "t5xxl_fp16.safetensors", "category": "text_encoders"},
            {"id": "vae/ae.safetensors", "name": "ae.safetensors", "category": "vae"},
            {"id": "checkpoints/juggernautXL_v9.safetensors", "name": "juggernautXL_v9.safetensors", "category": "checkpoints"}
        ]}))
    }

    #[test]
    fn leaves_out_video_recipes_and_reads_features() {
        let r = recipes();
        assert!(r.iter().all(|r| r.id != "wan22-t2v"));
        let k = r.iter().find(|r| r.id == "flux1-kontext").unwrap();
        assert!(k.edit && k.img2img);
        assert_eq!(k.slots[1].categories, vec!["text_encoders"]);
        assert_eq!(k.slots[2].label, "T5-XXL");
    }

    #[test]
    fn fills_slots_from_names() {
        let r = recipes();
        let f = files();
        let z = r.iter().find(|r| r.id == "z-image-turbo").unwrap();
        let picked = fill_slots(z, &f);
        assert_eq!(
            picked["diffusion_model"],
            "diffusion_models/z_image_turbo-Q4_K.gguf"
        );
        assert_eq!(
            picked["llm"],
            "text_encoders/Qwen3-4B-Instruct-2507-Q4_K_M.gguf"
        );
        assert_eq!(picked["vae"], "vae/ae.safetensors");
        assert!(!picked.contains_key("tae"));
        assert!(complete(z, &picked));

        let k = r.iter().find(|r| r.id == "flux1-kontext").unwrap();
        let picked = fill_slots(k, &f);
        assert_eq!(picked["clip_l"], "text_encoders/clip_l.safetensors");
        assert_eq!(picked["t5xxl"], "text_encoders/t5xxl_fp16.safetensors");
    }

    #[test]
    fn suggests_one_setup_per_main_file() {
        let s = suggest(&recipes(), &files());
        let ids: Vec<&str> = s.iter().map(|s| s.recipe.as_str()).collect();
        assert_eq!(
            ids,
            vec!["z-image-turbo", "flux1-kontext", "sdxl-checkpoint"]
        );
        assert_eq!(s[0].name, "z image turbo Q4 K");
    }
}

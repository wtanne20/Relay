//! Image projects, image chats, their turns (one prompt and the pictures it
//! made) and saved image model setups.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::db::now;

pub fn ensure_schema(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS image_models (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            recipe TEXT NOT NULL,
            files TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS image_projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            style TEXT NOT NULL DEFAULT '',
            negative TEXT NOT NULL DEFAULT '',
            model_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS image_chats (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES image_projects(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            model_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS image_chats_project ON image_chats(project_id, updated_at);
        CREATE TABLE IF NOT EXISTS image_turns (
            id TEXT PRIMARY KEY,
            chat_id TEXT NOT NULL REFERENCES image_chats(id) ON DELETE CASCADE,
            prompt TEXT NOT NULL,
            full_prompt TEXT NOT NULL,
            negative TEXT NOT NULL DEFAULT '',
            params TEXT NOT NULL DEFAULT '{}',
            base_image TEXT,
            model_id TEXT,
            model_name TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'queued',
            error TEXT,
            images TEXT NOT NULL DEFAULT '[]',
            seed INTEGER,
            duration_ms INTEGER,
            created_at TEXT NOT NULL,
            finished_at TEXT
        );
        CREATE INDEX IF NOT EXISTS image_turns_chat ON image_turns(chat_id, created_at);",
    )?;
    Ok(())
}

/// Marks pictures that were being made when Relay stopped.
pub fn reset_stale(c: &Connection) -> Result<usize> {
    Ok(c.execute(
        "UPDATE image_turns SET status = 'stopped', finished_at = ?1
         WHERE status IN ('queued', 'running')",
        params![now()],
    )?)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------- models ----------

/// A saved image model: a stable-diffusion.cpp recipe plus the file chosen
/// for each of its slots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageModel {
    pub id: String,
    pub name: String,
    pub recipe: String,
    /// Slot id -> sd-server file id.
    pub files: BTreeMap<String, String>,
    pub created_at: String,
}

fn model_row(r: &Row) -> rusqlite::Result<ImageModel> {
    let files: String = r.get(3)?;
    Ok(ImageModel {
        id: r.get(0)?,
        name: r.get(1)?,
        recipe: r.get(2)?,
        files: serde_json::from_str(&files).unwrap_or_default(),
        created_at: r.get(4)?,
    })
}

const MODEL_COLS: &str = "id, name, recipe, files, created_at";

pub fn list_models(c: &Connection) -> Result<Vec<ImageModel>> {
    let mut st = c.prepare(&format!(
        "SELECT {MODEL_COLS} FROM image_models ORDER BY created_at"
    ))?;
    Ok(st
        .query_map([], model_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn get_model(c: &Connection, id: &str) -> Result<Option<ImageModel>> {
    Ok(c.query_row(
        &format!("SELECT {MODEL_COLS} FROM image_models WHERE id = ?1"),
        params![id],
        model_row,
    )
    .optional()?)
}

pub fn insert_model(
    c: &Connection,
    name: &str,
    recipe: &str,
    files: &BTreeMap<String, String>,
) -> Result<ImageModel> {
    let m = ImageModel {
        id: new_id(),
        name: name.to_string(),
        recipe: recipe.to_string(),
        files: files.clone(),
        created_at: now(),
    };
    c.execute(
        "INSERT INTO image_models (id, name, recipe, files, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![m.id, m.name, m.recipe, serde_json::to_string(&m.files)?, m.created_at],
    )?;
    Ok(m)
}

pub fn update_model(
    c: &Connection,
    id: &str,
    name: &str,
    files: &BTreeMap<String, String>,
) -> Result<bool> {
    Ok(c.execute(
        "UPDATE image_models SET name = ?2, files = ?3 WHERE id = ?1",
        params![id, name, serde_json::to_string(files)?],
    )? > 0)
}

pub fn delete_model(c: &Connection, id: &str) -> Result<bool> {
    c.execute(
        "UPDATE image_projects SET model_id = NULL WHERE model_id = ?1",
        params![id],
    )?;
    c.execute(
        "UPDATE image_chats SET model_id = NULL WHERE model_id = ?1",
        params![id],
    )?;
    Ok(c.execute("DELETE FROM image_models WHERE id = ?1", params![id])? > 0)
}

// ---------- projects ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageProject {
    pub id: String,
    pub name: String,
    /// Added to every prompt in the project (a look or subject to keep).
    pub style: String,
    /// Things to keep out of every picture.
    pub negative: String,
    /// The model new chats start with.
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub chat_count: i64,
    /// The newest picture, for the project's thumbnail.
    pub cover: Option<String>,
}

const PROJECT_SELECT: &str =
    "SELECT p.id, p.name, p.style, p.negative, p.model_id, p.created_at, p.updated_at,
        (SELECT COUNT(*) FROM image_chats c WHERE c.project_id = p.id),
        (SELECT t.images FROM image_turns t JOIN image_chats c ON c.id = t.chat_id
          WHERE c.project_id = p.id AND t.status = 'done' ORDER BY t.created_at DESC LIMIT 1)
     FROM image_projects p";

fn project_row(r: &Row) -> rusqlite::Result<ImageProject> {
    let images: Option<String> = r.get(8)?;
    Ok(ImageProject {
        id: r.get(0)?,
        name: r.get(1)?,
        style: r.get(2)?,
        negative: r.get(3)?,
        model_id: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        chat_count: r.get(7)?,
        cover: images.and_then(|s| first_image(&s)),
    })
}

fn first_image(json: &str) -> Option<String> {
    serde_json::from_str::<Vec<String>>(json)
        .ok()
        .and_then(|v| v.into_iter().next())
}

pub fn list_projects(c: &Connection) -> Result<Vec<ImageProject>> {
    let mut st = c.prepare(&format!("{PROJECT_SELECT} ORDER BY p.updated_at DESC"))?;
    Ok(st
        .query_map([], project_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn get_project(c: &Connection, id: &str) -> Result<Option<ImageProject>> {
    Ok(c.query_row(
        &format!("{PROJECT_SELECT} WHERE p.id = ?1"),
        params![id],
        project_row,
    )
    .optional()?)
}

pub fn insert_project(c: &Connection, name: &str, model_id: Option<&str>) -> Result<ImageProject> {
    let id = new_id();
    let t = now();
    c.execute(
        "INSERT INTO image_projects (id, name, model_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![id, name, model_id, t],
    )?;
    Ok(get_project(c, &id)?.expect("just inserted"))
}

#[derive(Debug, Default, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub style: Option<String>,
    pub negative: Option<String>,
    /// Some("") clears it.
    pub model_id: Option<String>,
}

pub fn update_project(c: &Connection, id: &str, p: &ProjectPatch) -> Result<bool> {
    let Some(cur) = get_project(c, id)? else {
        return Ok(false);
    };
    let model = match &p.model_id {
        Some(m) if m.is_empty() => None,
        Some(m) => Some(m.clone()),
        None => cur.model_id,
    };
    c.execute(
        "UPDATE image_projects SET name = ?2, style = ?3, negative = ?4, model_id = ?5, updated_at = ?6 WHERE id = ?1",
        params![
            id,
            p.name.as_deref().unwrap_or(&cur.name),
            p.style.as_deref().unwrap_or(&cur.style),
            p.negative.as_deref().unwrap_or(&cur.negative),
            model,
            now()
        ],
    )?;
    Ok(true)
}

/// Deletes the project and its chats; returns the pictures they held.
pub fn delete_project(c: &mut Connection, id: &str) -> Result<Option<Vec<String>>> {
    let tx = c.transaction()?;
    let pictures = {
        let mut st = tx.prepare(
            "SELECT t.images, t.base_image FROM image_turns t JOIN image_chats c ON c.id = t.chat_id WHERE c.project_id = ?1",
        )?;
        pictures_of(st.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?)?
    };
    tx.execute(
        "DELETE FROM image_turns WHERE chat_id IN (SELECT id FROM image_chats WHERE project_id = ?1)",
        params![id],
    )?;
    tx.execute("DELETE FROM image_chats WHERE project_id = ?1", params![id])?;
    let n = tx.execute("DELETE FROM image_projects WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok((n > 0).then_some(pictures))
}

fn pictures_of(
    rows: impl Iterator<Item = rusqlite::Result<(String, Option<String>)>>,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for r in rows {
        let (images, base) = r?;
        out.extend(serde_json::from_str::<Vec<String>>(&images).unwrap_or_default());
        out.extend(base);
    }
    Ok(out)
}

// ---------- chats ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageChat {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// "running" while a picture is being made, else "idle".
    pub status: String,
    pub cover: Option<String>,
}

const CHAT_SELECT: &str = "SELECT c.id, c.project_id, c.title, c.model_id, c.created_at, c.updated_at,
        EXISTS(SELECT 1 FROM image_turns t WHERE t.chat_id = c.id AND t.status IN ('queued', 'running')),
        (SELECT t.images FROM image_turns t WHERE t.chat_id = c.id AND t.status = 'done' ORDER BY t.created_at DESC LIMIT 1)
     FROM image_chats c";

fn chat_row(r: &Row) -> rusqlite::Result<ImageChat> {
    let busy: bool = r.get(6)?;
    let images: Option<String> = r.get(7)?;
    Ok(ImageChat {
        id: r.get(0)?,
        project_id: r.get(1)?,
        title: r.get(2)?,
        model_id: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
        status: if busy { "running" } else { "idle" }.into(),
        cover: images.and_then(|s| first_image(&s)),
    })
}

pub fn list_chats(c: &Connection, project_id: &str) -> Result<Vec<ImageChat>> {
    let mut st = c.prepare(&format!(
        "{CHAT_SELECT} WHERE c.project_id = ?1 ORDER BY c.updated_at DESC"
    ))?;
    Ok(st
        .query_map(params![project_id], chat_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn get_chat(c: &Connection, id: &str) -> Result<Option<ImageChat>> {
    Ok(c.query_row(
        &format!("{CHAT_SELECT} WHERE c.id = ?1"),
        params![id],
        chat_row,
    )
    .optional()?)
}

pub fn insert_chat(
    c: &Connection,
    project_id: &str,
    title: &str,
    model_id: Option<&str>,
) -> Result<ImageChat> {
    let id = new_id();
    let t = now();
    c.execute(
        "INSERT INTO image_chats (id, project_id, title, model_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id, project_id, title, model_id, t],
    )?;
    touch_project(c, project_id)?;
    Ok(get_chat(c, &id)?.expect("just inserted"))
}

fn touch_project(c: &Connection, id: &str) -> Result<()> {
    c.execute(
        "UPDATE image_projects SET updated_at = ?2 WHERE id = ?1",
        params![id, now()],
    )?;
    Ok(())
}

pub fn update_chat(
    c: &Connection,
    id: &str,
    title: Option<&str>,
    model_id: Option<Option<&str>>,
) -> Result<bool> {
    let Some(cur) = get_chat(c, id)? else {
        return Ok(false);
    };
    let model = match model_id {
        Some(m) => m.map(str::to_string),
        None => cur.model_id,
    };
    c.execute(
        "UPDATE image_chats SET title = ?2, model_id = ?3 WHERE id = ?1",
        params![id, title.unwrap_or(&cur.title), model],
    )?;
    Ok(true)
}

/// Deletes the chat; returns the pictures it held.
pub fn delete_chat(c: &mut Connection, id: &str) -> Result<Option<Vec<String>>> {
    let tx = c.transaction()?;
    let pictures = {
        let mut st = tx.prepare("SELECT images, base_image FROM image_turns WHERE chat_id = ?1")?;
        pictures_of(st.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?)?
    };
    tx.execute("DELETE FROM image_turns WHERE chat_id = ?1", params![id])?;
    let n = tx.execute("DELETE FROM image_chats WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok((n > 0).then_some(pictures))
}

// ---------- turns ----------

/// Generation settings for one turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TurnParams {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub steps: Option<u32>,
    pub cfg_scale: Option<f32>,
    /// How much a refine may change the base picture (0..1).
    pub strength: Option<f32>,
    pub count: u32,
    /// True when the base picture was passed as a reference for an
    /// editing model (instead of as the starting point).
    pub edit: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageTurn {
    pub id: String,
    pub chat_id: String,
    /// What the user typed.
    pub prompt: String,
    /// What the model was given (with the project's style, and earlier
    /// prompts when refining with a model that can't follow edits).
    pub full_prompt: String,
    pub negative: String,
    pub params: TurnParams,
    /// The picture this turn started from.
    pub base_image: Option<String>,
    pub model_id: Option<String>,
    pub model_name: String,
    /// queued, running, done, error, stopped
    pub status: String,
    pub error: Option<String>,
    pub images: Vec<String>,
    pub seed: Option<i64>,
    pub duration_ms: Option<i64>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

const TURN_COLS: &str =
    "id, chat_id, prompt, full_prompt, negative, params, base_image, model_id, model_name,
     status, error, images, seed, duration_ms, created_at, finished_at";

fn turn_row(r: &Row) -> rusqlite::Result<ImageTurn> {
    let params: String = r.get(5)?;
    let images: String = r.get(11)?;
    Ok(ImageTurn {
        id: r.get(0)?,
        chat_id: r.get(1)?,
        prompt: r.get(2)?,
        full_prompt: r.get(3)?,
        negative: r.get(4)?,
        params: serde_json::from_str(&params).unwrap_or_default(),
        base_image: r.get(6)?,
        model_id: r.get(7)?,
        model_name: r.get(8)?,
        status: r.get(9)?,
        error: r.get(10)?,
        images: serde_json::from_str(&images).unwrap_or_default(),
        seed: r.get(12)?,
        duration_ms: r.get(13)?,
        created_at: r.get(14)?,
        finished_at: r.get(15)?,
    })
}

pub fn list_turns(c: &Connection, chat_id: &str) -> Result<Vec<ImageTurn>> {
    let mut st = c.prepare(&format!(
        "SELECT {TURN_COLS} FROM image_turns WHERE chat_id = ?1 ORDER BY created_at, rowid"
    ))?;
    Ok(st
        .query_map(params![chat_id], turn_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn get_turn(c: &Connection, id: &str) -> Result<Option<ImageTurn>> {
    Ok(c.query_row(
        &format!("SELECT {TURN_COLS} FROM image_turns WHERE id = ?1"),
        params![id],
        turn_row,
    )
    .optional()?)
}

pub struct NewTurn {
    pub chat_id: String,
    pub prompt: String,
    pub full_prompt: String,
    pub negative: String,
    pub params: TurnParams,
    pub base_image: Option<String>,
    pub model_id: Option<String>,
    pub model_name: String,
    pub seed: i64,
}

pub fn insert_turn(c: &Connection, t: &NewTurn) -> Result<ImageTurn> {
    let id = new_id();
    c.execute(
        "INSERT INTO image_turns (id, chat_id, prompt, full_prompt, negative, params, base_image, model_id,
           model_name, seed, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'queued', ?11)",
        params![
            id,
            t.chat_id,
            t.prompt,
            t.full_prompt,
            t.negative,
            serde_json::to_string(&t.params)?,
            t.base_image,
            t.model_id,
            t.model_name,
            t.seed,
            now()
        ],
    )?;
    c.execute(
        "UPDATE image_chats SET updated_at = ?2 WHERE id = ?1",
        params![t.chat_id, now()],
    )?;
    if let Some(p) = c
        .query_row(
            "SELECT project_id FROM image_chats WHERE id = ?1",
            params![t.chat_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        touch_project(c, &p)?;
    }
    Ok(get_turn(c, &id)?.expect("just inserted"))
}

pub fn set_turn_running(c: &Connection, id: &str) -> Result<()> {
    c.execute(
        "UPDATE image_turns SET status = 'running' WHERE id = ?1 AND status = 'queued'",
        params![id],
    )?;
    Ok(())
}

pub fn finish_turn(
    c: &Connection,
    id: &str,
    status: &str,
    error: Option<&str>,
    images: &[String],
    duration_ms: Option<i64>,
) -> Result<()> {
    c.execute(
        "UPDATE image_turns SET status = ?2, error = ?3, images = ?4, duration_ms = ?5, finished_at = ?6 WHERE id = ?1",
        params![id, status, error, serde_json::to_string(images)?, duration_ms, now()],
    )?;
    Ok(())
}

/// Deletes one turn; returns its pictures.
pub fn delete_turn(c: &Connection, id: &str) -> Result<Option<Vec<String>>> {
    let Some(t) = get_turn(c, id)? else {
        return Ok(None);
    };
    c.execute("DELETE FROM image_turns WHERE id = ?1", params![id])?;
    let mut pics = t.images;
    pics.extend(t.base_image);
    Ok(Some(pics))
}

/// The most recent finished turn of a chat.
pub fn last_done_turn(c: &Connection, chat_id: &str) -> Result<Option<ImageTurn>> {
    Ok(c.query_row(
        &format!(
            "SELECT {TURN_COLS} FROM image_turns WHERE chat_id = ?1 AND status = 'done' ORDER BY created_at DESC, rowid DESC LIMIT 1"
        ),
        params![chat_id],
        turn_row,
    )
    .optional()?)
}

/// Turn that produced `picture`, if any.
pub fn turn_with_picture(
    c: &Connection,
    chat_id: &str,
    picture: &str,
) -> Result<Option<ImageTurn>> {
    Ok(list_turns(c, chat_id)?
        .into_iter()
        .find(|t| t.images.iter().any(|i| i == picture)))
}

/// Whether any row still refers to the picture (it may be used as a base
/// image in another turn).
pub fn picture_in_use(c: &Connection, picture: &str) -> Result<bool> {
    let like = format!("%\"{picture}\"%");
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM image_turns WHERE base_image = ?1 OR images LIKE ?2)",
        params![picture, like],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        ensure_schema(&c).unwrap();
        c
    }

    #[test]
    fn projects_chats_and_turns() {
        let mut c = conn();
        let m = insert_model(
            &c,
            "Z",
            "z-image-turbo",
            &BTreeMap::from([("diffusion_model".into(), "a.gguf".into())]),
        )
        .unwrap();
        let p = insert_project(&c, "Logo", Some(&m.id)).unwrap();
        let chat = insert_chat(&c, &p.id, "Fox", p.model_id.as_deref()).unwrap();
        assert_eq!(chat.status, "idle");
        let t = insert_turn(
            &c,
            &NewTurn {
                chat_id: chat.id.clone(),
                prompt: "a fox".into(),
                full_prompt: "a fox".into(),
                negative: String::new(),
                params: TurnParams {
                    count: 1,
                    ..Default::default()
                },
                base_image: None,
                model_id: Some(m.id.clone()),
                model_name: "Z".into(),
                seed: 7,
            },
        )
        .unwrap();
        assert_eq!(get_chat(&c, &chat.id).unwrap().unwrap().status, "running");
        finish_turn(&c, &t.id, "done", None, &["a.png".into()], Some(1200)).unwrap();
        let chat = get_chat(&c, &chat.id).unwrap().unwrap();
        assert_eq!(chat.status, "idle");
        assert_eq!(chat.cover.as_deref(), Some("a.png"));
        assert_eq!(
            get_project(&c, &p.id).unwrap().unwrap().cover.as_deref(),
            Some("a.png")
        );
        assert_eq!(last_done_turn(&c, &chat.id).unwrap().unwrap().id, t.id);
        assert!(picture_in_use(&c, "a.png").unwrap());

        delete_model(&c, &m.id).unwrap();
        assert_eq!(get_project(&c, &p.id).unwrap().unwrap().model_id, None);

        let pics = delete_project(&mut c, &p.id).unwrap().unwrap();
        assert_eq!(pics, vec!["a.png".to_string()]);
        assert!(get_chat(&c, &chat.id).unwrap().is_none());
        assert!(!picture_in_use(&c, "a.png").unwrap());
    }
}

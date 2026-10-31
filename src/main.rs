mod push;
mod zones;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use rand::{seq::SliceRandom, RngExt};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const QUESTION_LIFE_S: u64 = 7 * 24 * 3600; // questions disappear after 7 days
const THERE_WINDOW_S: u64 = 30 * 60; // "was there" = seen in that zone within 30 min
const MAX_TEXT: usize = 500;
const MAX_OPTION: usize = 80;

// notification rules
const RANDOM_EXTRA: usize = 10; // a question about a place also goes to this many random people
const NO_PLACE_SHARE: f64 = 0.65; // a question with no place goes to this share of everyone
const REPLY_NOTIFY_CHANCE: f64 = 0.10; // only about 1 in 10 replies notifies the person replied to
const MAX_PER_HOUR: usize = 5; // nobody gets more than this many pushes in an hour

struct Person {
    lat: f64,
    lng: f64,
    zone: Option<&'static str>,
    seen_at: u64,
}

struct Reply {
    id: u64,
    parent: Option<u64>, // None = answers the question, Some(id) = answers that reply
    author: Option<String>,
    text: String,
    at: u64,
    was_there: bool,
    likes: HashSet<String>,
}

struct PollOption {
    label: String,
    votes: HashSet<String>,
}

struct Question {
    id: u64,
    author: Option<String>,
    text: String,
    at: u64,
    place: Option<&'static str>,
    likes: HashSet<String>,
    options: Vec<PollOption>, // empty = not a poll
    replies: Vec<Reply>,
}

struct App {
    people: Mutex<HashMap<String, Person>>,
    feed: Mutex<Vec<Question>>,
    subs: Mutex<HashMap<String, push::Sub>>,
    sent: Mutex<HashMap<String, Vec<u64>>>, // push times per phone, for the hourly limit
    next_id: AtomicU64,
    vapid: push::Vapid,
    agent: ureq::Agent,
    db: Mutex<Connection>, // always lock this last, after any other lock
}
type Shared = Arc<App>;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64
}

// ---------- database ----------

fn add_column(db: &Connection, table: &str, column: &str, ddl: &str) {
    let has = db
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{column}'"))
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has {
        let _ = db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl}"));
    }
}

fn open_db() -> Connection {
    let db = Connection::open("ted.db").expect("could not open ted.db");
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS people (id TEXT PRIMARY KEY, lat REAL NOT NULL, lng REAL NOT NULL, seen_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS questions (id INTEGER PRIMARY KEY, text TEXT NOT NULL, at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS replies (id INTEGER PRIMARY KEY, question_id INTEGER NOT NULL, parent INTEGER, text TEXT NOT NULL, at INTEGER NOT NULL, was_there INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS likes (question_id INTEGER NOT NULL, reply_id INTEGER NOT NULL, who TEXT NOT NULL, PRIMARY KEY (question_id, reply_id, who));
         CREATE TABLE IF NOT EXISTS poll_options (question_id INTEGER NOT NULL, idx INTEGER NOT NULL, label TEXT NOT NULL, PRIMARY KEY (question_id, idx));
         CREATE TABLE IF NOT EXISTS votes (question_id INTEGER NOT NULL, who TEXT NOT NULL, idx INTEGER NOT NULL, PRIMARY KEY (question_id, who));
         CREATE TABLE IF NOT EXISTS push (id TEXT PRIMARY KEY, endpoint TEXT NOT NULL, p256dh TEXT NOT NULL, auth TEXT NOT NULL);",
    )
    .expect("could not set up the database");
    // older databases do not have these two columns yet
    add_column(&db, "questions", "author", "TEXT");
    add_column(&db, "replies", "author", "TEXT");
    db
}

// delete questions older than the cutoff, plus everything that belongs to them
fn purge(db: &Connection, cutoff: u64) {
    let _ = db.execute_batch(&format!(
        "DELETE FROM questions WHERE at <= {c};
         DELETE FROM replies WHERE question_id NOT IN (SELECT id FROM questions);
         DELETE FROM likes WHERE question_id NOT IN (SELECT id FROM questions);
         DELETE FROM poll_options WHERE question_id NOT IN (SELECT id FROM questions);
         DELETE FROM votes WHERE question_id NOT IN (SELECT id FROM questions);",
        c = cutoff
    ));
}

struct Loaded {
    people: HashMap<String, Person>,
    feed: Vec<Question>,
    subs: HashMap<String, push::Sub>,
    next: u64,
}

// read everything back into memory when the server starts
fn load(db: &Connection) -> Loaded {
    let mut people = HashMap::new();
    {
        let mut stmt = db.prepare("SELECT id, lat, lng, seen_at FROM people").unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?))
            })
            .unwrap();
        for (id, lat, lng, seen) in rows.flatten() {
            people.insert(id, Person { lat, lng, zone: zones::find_zone(lat, lng), seen_at: seen as u64 });
        }
    }

    let mut subs = HashMap::new();
    {
        let mut stmt = db.prepare("SELECT id, endpoint, p256dh, auth FROM push").unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })
            .unwrap();
        for (id, endpoint, p256dh, auth) in rows.flatten() {
            subs.insert(id, push::Sub { endpoint, p256dh, auth });
        }
    }

    let mut feed: Vec<Question> = Vec::new();
    let mut next = 0u64;
    {
        let mut stmt = db.prepare("SELECT id, text, at, author FROM questions ORDER BY id").unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?))
            })
            .unwrap();
        for (id, text, at, author) in rows.flatten() {
            next = next.max(id as u64 + 1);
            feed.push(Question {
                id: id as u64,
                author,
                place: zones::place_in_text(&text),
                text,
                at: at as u64,
                likes: HashSet::new(),
                options: Vec::new(),
                replies: Vec::new(),
            });
        }
    }
    {
        let mut stmt = db
            .prepare("SELECT question_id, label FROM poll_options ORDER BY question_id, idx")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap();
        for (qid, label) in rows.flatten() {
            if let Some(q) = feed.iter_mut().find(|q| q.id == qid as u64) {
                q.options.push(PollOption { label, votes: HashSet::new() });
            }
        }
    }
    {
        let mut stmt = db.prepare("SELECT question_id, who, idx FROM votes").unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .unwrap();
        for (qid, who, idx) in rows.flatten() {
            if let Some(o) = feed
                .iter_mut()
                .find(|q| q.id == qid as u64)
                .and_then(|q| q.options.get_mut(idx as usize))
            {
                o.votes.insert(who);
            }
        }
    }
    {
        let mut stmt = db
            .prepare("SELECT id, question_id, parent, text, at, was_there, author FROM replies ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            })
            .unwrap();
        for (id, qid, parent, text, at, was_there, author) in rows.flatten() {
            next = next.max(id as u64 + 1);
            if let Some(q) = feed.iter_mut().find(|q| q.id == qid as u64) {
                q.replies.push(Reply {
                    id: id as u64,
                    parent: parent.map(|p| p as u64),
                    author,
                    text,
                    at: at as u64,
                    was_there,
                    likes: HashSet::new(),
                });
            }
        }
    }
    {
        let mut stmt = db.prepare("SELECT question_id, reply_id, who FROM likes").unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))
            .unwrap();
        for (qid, rid, who) in rows.flatten() {
            let Some(q) = feed.iter_mut().find(|q| q.id == qid as u64) else { continue };
            if rid < 0 {
                q.likes.insert(who);
            } else if let Some(r) = q.replies.iter_mut().find(|r| r.id == rid as u64) {
                r.likes.insert(who);
            }
        }
    }
    Loaded { people, feed, subs, next }
}

// ---------- map ----------

#[derive(Deserialize)]
struct Ping {
    id: String,
    lat: f64,
    lng: f64,
}

#[derive(Serialize)]
struct PingReply {
    zone: Option<&'static str>,
}

#[derive(Serialize)]
struct Dot {
    lat: f64,
    lng: f64,
    zone: Option<&'static str>,
    seconds_ago: u64,
}

#[derive(Deserialize)]
struct Me {
    id: Option<String>,
}

async fn ping(
    State(app): State<Shared>,
    Json(p): Json<Ping>,
) -> Result<Json<PingReply>, StatusCode> {
    let valid = (-90.0..=90.0).contains(&p.lat)
        && (-180.0..=180.0).contains(&p.lng)
        && valid_id(&p.id);
    if !valid {
        return Err(StatusCode::BAD_REQUEST);
    }
    let zone = zones::find_zone(p.lat, p.lng);
    let t = now();
    app.people.lock().unwrap().insert(
        p.id.clone(),
        Person { lat: p.lat, lng: p.lng, zone, seen_at: t },
    );
    let _ = app.db.lock().unwrap().execute(
        "INSERT INTO people (id, lat, lng, seen_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET lat = ?2, lng = ?3, seen_at = ?4",
        params![p.id, p.lat, p.lng, t as i64],
    );
    Ok(Json(PingReply { zone }))
}

async fn dots(State(app): State<Shared>, Query(me): Query<Me>) -> Json<Vec<Dot>> {
    let t = now();
    let map = app.people.lock().unwrap();
    let list = map
        .iter()
        .filter(|(id, _)| Some(id.as_str()) != me.id.as_deref())
        .map(|(_, p)| Dot {
            lat: p.lat,
            lng: p.lng,
            zone: p.zone,
            seconds_ago: t.saturating_sub(p.seen_at),
        })
        .collect();
    Json(list)
}

async fn zone_list() -> Json<&'static [zones::Zone]> {
    Json(zones::ZONES)
}

// ---------- push ----------

#[derive(Deserialize)]
struct SubscribeIn {
    id: String,
    endpoint: String,
    keys: push::Keys,
}

#[derive(Deserialize)]
struct UnsubscribeIn {
    id: String,
}

async fn push_key(State(app): State<Shared>) -> Json<String> {
    Json(app.vapid.public.clone())
}

async fn subscribe(State(app): State<Shared>, Json(s): Json<SubscribeIn>) -> StatusCode {
    if !valid_id(&s.id) || !s.endpoint.starts_with("http") || s.endpoint.len() > 1000 {
        return StatusCode::BAD_REQUEST;
    }
    let sub = push::Sub { endpoint: s.endpoint, p256dh: s.keys.p256dh, auth: s.keys.auth };
    let _ = app.db.lock().unwrap().execute(
        "INSERT OR REPLACE INTO push (id, endpoint, p256dh, auth) VALUES (?1, ?2, ?3, ?4)",
        params![s.id, sub.endpoint, sub.p256dh, sub.auth],
    );
    app.subs.lock().unwrap().insert(s.id, sub);
    StatusCode::NO_CONTENT
}

async fn unsubscribe(State(app): State<Shared>, Json(u): Json<UnsubscribeIn>) -> StatusCode {
    app.subs.lock().unwrap().remove(&u.id);
    let _ = app.db.lock().unwrap().execute("DELETE FROM push WHERE id = ?1", params![u.id]);
    StatusCode::NO_CONTENT
}

// keep only the people who have not hit the hourly limit, and count this push for them
fn within_limit(app: &Shared, ids: Vec<String>, t: u64) -> Vec<String> {
    let mut sent = app.sent.lock().unwrap();
    ids.into_iter()
        .filter(|id| {
            let times = sent.entry(id.clone()).or_default();
            times.retain(|&s| t.saturating_sub(s) < 3600);
            if times.len() >= MAX_PER_HOUR {
                false
            } else {
                times.push(t);
                true
            }
        })
        .collect()
}

// who hears about a new question
fn question_targets(app: &Shared, place: Option<&str>, asker: &str, t: u64) -> Vec<String> {
    let subscribed: Vec<String> = app
        .subs
        .lock()
        .unwrap()
        .keys()
        .filter(|id| id.as_str() != asker)
        .cloned()
        .collect();
    let mut rng = rand::rng();
    let picked = match place {
        Some(place) => {
            // everyone there now or in the last 30 minutes, plus a few random others
            let people = app.people.lock().unwrap();
            let (near, mut rest): (Vec<String>, Vec<String>) = subscribed.into_iter().partition(|id| {
                people
                    .get(id)
                    .map_or(false, |p| p.zone == Some(place) && t.saturating_sub(p.seen_at) <= THERE_WINDOW_S)
            });
            drop(people);
            rest.shuffle(&mut rng);
            rest.truncate(RANDOM_EXTRA);
            near.into_iter().chain(rest).collect()
        }
        None => {
            // no place to aim at, so most people get it
            let mut all = subscribed;
            all.shuffle(&mut rng);
            let n = (all.len() as f64 * NO_PLACE_SHARE).ceil() as usize;
            all.truncate(n);
            all
        }
    };
    within_limit(app, picked, t)
}

fn short(text: &str) -> String {
    if text.chars().count() > 140 {
        let cut: String = text.chars().take(140).collect();
        format!("{cut}...")
    } else {
        text.to_string()
    }
}

// send one message to each phone, on background threads so requests never wait for it
fn dispatch(app: &Shared, targets: Vec<String>, payload: String) {
    let jobs: Vec<(String, push::Sub)> = {
        let subs = app.subs.lock().unwrap();
        targets
            .into_iter()
            .filter_map(|id| subs.get(&id).map(|s| (id, s.clone())))
            .collect()
    };
    for (id, sub) in jobs {
        let app = app.clone();
        let payload = payload.clone();
        tokio::task::spawn_blocking(move || {
            if let push::Outcome::Gone = push::send(&app.vapid.pair, &app.agent, &sub, &payload) {
                let mut subs = app.subs.lock().unwrap();
                if subs.get(&id).map_or(false, |s| s.endpoint == sub.endpoint) {
                    subs.remove(&id);
                    drop(subs);
                    let _ = app.db.lock().unwrap().execute("DELETE FROM push WHERE id = ?1", params![id]);
                }
            }
        });
    }
}

// ---------- feed ----------

#[derive(Deserialize)]
struct Ask {
    id: String,
    text: String,
    options: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ReplyIn {
    id: String,
    question_id: u64,
    parent_id: Option<u64>,
    text: String,
}

#[derive(Deserialize)]
struct LikeIn {
    id: String,
    question_id: u64,
    reply_id: Option<u64>,
}

#[derive(Deserialize)]
struct VoteIn {
    id: String,
    question_id: u64,
    option: usize,
}

#[derive(Serialize)]
struct ReplyView {
    id: u64,
    parent: Option<u64>,
    text: String,
    seconds_ago: u64,
    was_there: bool,
    likes: usize,
    liked: bool,
}

#[derive(Serialize)]
struct OptionView {
    label: String,
    votes: usize,
}

#[derive(Serialize)]
struct PollView {
    options: Vec<OptionView>,
    total: usize,
    mine: Option<usize>,
}

#[derive(Serialize)]
struct QuestionView {
    id: u64,
    text: String,
    seconds_ago: u64,
    place: Option<&'static str>,
    likes: usize,
    liked: bool,
    poll: Option<PollView>,
    replies: Vec<ReplyView>,
}

fn clean(text: &str, max: usize) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > max {
        None
    } else {
        Some(t.to_string())
    }
}

async fn ask(State(app): State<Shared>, Json(a): Json<Ask>) -> StatusCode {
    if !valid_id(&a.id) {
        return StatusCode::BAD_REQUEST;
    }
    let Some(text) = clean(&a.text, MAX_TEXT) else {
        return StatusCode::BAD_REQUEST;
    };

    // a poll has 2 to 4 options
    let mut options = Vec::new();
    if let Some(list) = &a.options {
        if !(2..=4).contains(&list.len()) {
            return StatusCode::BAD_REQUEST;
        }
        for label in list {
            let Some(label) = clean(label, MAX_OPTION) else {
                return StatusCode::BAD_REQUEST;
            };
            options.push(PollOption { label, votes: HashSet::new() });
        }
    }

    let t = now();
    let q = Question {
        id: app.next_id.fetch_add(1, Ordering::Relaxed),
        author: Some(a.id.clone()),
        place: zones::place_in_text(&text),
        text,
        at: t,
        likes: HashSet::new(),
        options,
        replies: Vec::new(),
    };
    let (place, qid, body) = (q.place, q.id, short(&q.text));

    {
        let mut feed = app.feed.lock().unwrap();
        {
            let db = app.db.lock().unwrap();
            let _ = db.execute(
                "INSERT INTO questions (id, text, at, author) VALUES (?1, ?2, ?3, ?4)",
                params![q.id as i64, q.text, q.at as i64, a.id],
            );
            for (i, o) in q.options.iter().enumerate() {
                let _ = db.execute(
                    "INSERT INTO poll_options (question_id, idx, label) VALUES (?1, ?2, ?3)",
                    params![q.id as i64, i as i64, o.label],
                );
            }
            purge(&db, t.saturating_sub(QUESTION_LIFE_S));
        }
        feed.retain(|q| t.saturating_sub(q.at) < QUESTION_LIFE_S);
        feed.push(q);
    }

    let targets = question_targets(&app, place, &a.id, t);
    if !targets.is_empty() {
        let payload = serde_json::json!({ "kind": "question", "text": body, "qid": qid }).to_string();
        dispatch(&app, targets, payload);
    }
    StatusCode::NO_CONTENT
}

async fn reply(State(app): State<Shared>, Json(r): Json<ReplyIn>) -> StatusCode {
    if !valid_id(&r.id) {
        return StatusCode::BAD_REQUEST;
    }
    let Some(text) = clean(&r.text, MAX_TEXT) else {
        return StatusCode::BAD_REQUEST;
    };
    let t = now();

    // where was the replier last seen (only if recently)?
    let zone_now = {
        let people = app.people.lock().unwrap();
        people
            .get(&r.id)
            .filter(|p| t.saturating_sub(p.seen_at) <= THERE_WINDOW_S)
            .and_then(|p| p.zone)
    };

    let notify: Option<String>;
    {
        let mut feed = app.feed.lock().unwrap();
        let Some(q) = feed.iter_mut().find(|q| q.id == r.question_id) else {
            return StatusCode::NOT_FOUND;
        };
        // who wrote the thing being answered
        let answered_author = match r.parent_id {
            None => q.author.clone(),
            Some(pid) => match q.replies.iter().find(|x| x.id == pid) {
                Some(p) => p.author.clone(),
                None => return StatusCode::NOT_FOUND,
            },
        };
        let was_there = q.place.is_some() && q.place == zone_now;
        let id = app.next_id.fetch_add(1, Ordering::Relaxed);
        let _ = app.db.lock().unwrap().execute(
            "INSERT INTO replies (id, question_id, parent, text, at, was_there, author) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id as i64, q.id as i64, r.parent_id.map(|p| p as i64), text, t as i64, was_there, r.id],
        );
        q.replies.push(Reply {
            id,
            parent: r.parent_id,
            author: Some(r.id.clone()),
            text: text.clone(),
            at: t,
            was_there,
            likes: HashSet::new(),
        });
        notify = answered_author.filter(|a| *a != r.id);
    }

    // only some replies notify, picked at random
    if let Some(target) = notify {
        let lucky = rand::rng().random::<f64>() < REPLY_NOTIFY_CHANCE;
        if lucky && app.subs.lock().unwrap().contains_key(&target) {
            let ok = within_limit(&app, vec![target], t);
            if !ok.is_empty() {
                let payload = serde_json::json!({ "kind": "reply", "text": short(&text), "qid": r.question_id }).to_string();
                dispatch(&app, ok, payload);
            }
        }
    }
    StatusCode::NO_CONTENT
}

// tapping like twice takes it back. likes never notify anyone.
async fn like(State(app): State<Shared>, Json(l): Json<LikeIn>) -> StatusCode {
    if !valid_id(&l.id) {
        return StatusCode::BAD_REQUEST;
    }
    let mut feed = app.feed.lock().unwrap();
    let Some(q) = feed.iter_mut().find(|q| q.id == l.question_id) else {
        return StatusCode::NOT_FOUND;
    };
    let set = match l.reply_id {
        None => &mut q.likes,
        Some(rid) => match q.replies.iter_mut().find(|r| r.id == rid) {
            Some(r) => &mut r.likes,
            None => return StatusCode::NOT_FOUND,
        },
    };
    let added = if set.remove(&l.id) {
        false
    } else {
        set.insert(l.id.clone());
        true
    };
    let rid = l.reply_id.map_or(-1, |x| x as i64);
    let db = app.db.lock().unwrap();
    let _ = if added {
        db.execute(
            "INSERT OR IGNORE INTO likes (question_id, reply_id, who) VALUES (?1, ?2, ?3)",
            params![l.question_id as i64, rid, l.id],
        )
    } else {
        db.execute(
            "DELETE FROM likes WHERE question_id = ?1 AND reply_id = ?2 AND who = ?3",
            params![l.question_id as i64, rid, l.id],
        )
    };
    StatusCode::NO_CONTENT
}

// one vote per phone, and it cannot be changed
async fn vote(State(app): State<Shared>, Json(v): Json<VoteIn>) -> StatusCode {
    if !valid_id(&v.id) {
        return StatusCode::BAD_REQUEST;
    }
    let mut feed = app.feed.lock().unwrap();
    let Some(q) = feed.iter_mut().find(|q| q.id == v.question_id) else {
        return StatusCode::NOT_FOUND;
    };
    if v.option >= q.options.len() {
        return StatusCode::BAD_REQUEST;
    }
    if q.options.iter().any(|o| o.votes.contains(&v.id)) {
        return StatusCode::CONFLICT;
    }
    q.options[v.option].votes.insert(v.id.clone());
    let _ = app.db.lock().unwrap().execute(
        "INSERT OR IGNORE INTO votes (question_id, who, idx) VALUES (?1, ?2, ?3)",
        params![v.question_id as i64, v.id, v.option as i64],
    );
    StatusCode::NO_CONTENT
}

async fn questions(State(app): State<Shared>, Query(me): Query<Me>) -> Json<Vec<QuestionView>> {
    let t = now();
    let who = me.id.as_deref();
    let liked = |s: &HashSet<String>| who.map_or(false, |w| s.contains(w));
    let feed = app.feed.lock().unwrap();
    let mut list: Vec<QuestionView> = feed
        .iter()
        .filter(|q| t.saturating_sub(q.at) < QUESTION_LIFE_S)
        .map(|q| QuestionView {
            id: q.id,
            text: q.text.clone(),
            seconds_ago: t.saturating_sub(q.at),
            place: q.place,
            likes: q.likes.len(),
            liked: liked(&q.likes),
            poll: if q.options.is_empty() {
                None
            } else {
                Some(PollView {
                    options: q
                        .options
                        .iter()
                        .map(|o| OptionView { label: o.label.clone(), votes: o.votes.len() })
                        .collect(),
                    total: q.options.iter().map(|o| o.votes.len()).sum(),
                    mine: who.and_then(|w| q.options.iter().position(|o| o.votes.contains(w))),
                })
            },
            replies: q
                .replies
                .iter()
                .map(|r| ReplyView {
                    id: r.id,
                    parent: r.parent,
                    text: r.text.clone(),
                    seconds_ago: t.saturating_sub(r.at),
                    was_there: r.was_there,
                    likes: r.likes.len(),
                    liked: liked(&r.likes),
                })
                .collect(),
        })
        .collect();
    list.reverse(); // newest first
    Json(list)
}

#[tokio::main]
async fn main() {
    let db = open_db();
    let vapid = push::vapid(&db);
    let loaded = load(&db);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let app: Shared = Arc::new(App {
        people: Mutex::new(loaded.people),
        feed: Mutex::new(loaded.feed),
        subs: Mutex::new(loaded.subs),
        sent: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(loaded.next),
        vapid,
        agent,
        db: Mutex::new(db),
    });

    let router = Router::new()
        .route("/api/ping", post(ping))
        .route("/api/dots", get(dots))
        .route("/api/zones", get(zone_list))
        .route("/api/questions", get(questions).post(ask))
        .route("/api/reply", post(reply))
        .route("/api/like", post(like))
        .route("/api/vote", post(vote))
        .route("/api/push/key", get(push_key))
        .route("/api/push/subscribe", post(subscribe))
        .route("/api/push/unsubscribe", post(unsubscribe))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("ted is running on http://localhost:3000");
    axum::serve(listener, router).await.unwrap();
}
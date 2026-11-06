mod push;
mod zones;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    routing::{get, post},
    Json, Router,
};
use base64ct::{Base64, Encoding};
use postgres::{Client, NoTls};
use rand::{seq::SliceRandom, RngExt};
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
    deleted: bool,
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

struct Prof {
    nick: Option<String>,
    color: Option<String>,
    photo: Option<Vec<u8>>,
    v: u64, // changes when the profile is saved, so browsers fetch the new picture
}

struct App {
    people: Mutex<HashMap<String, Person>>,
    profiles: Mutex<HashMap<String, Prof>>,
    feed: Mutex<Vec<Question>>,
    subs: Mutex<HashMap<String, push::Sub>>,
    sent: Mutex<HashMap<String, Vec<u64>>>, // push times per phone, for the hourly limit
    next_id: AtomicU64,
    vapid: push::Vapid,
    agent: ureq::Agent,
    db: std::sync::mpsc::Sender<DbJob>, // database writes go to one background worker
}
type Shared = Arc<App>;
type DbJob = Box<dyn FnOnce(&mut Client) + Send>;

impl App {
    fn save(&self, job: impl FnOnce(&mut Client) + Send + 'static) {
        let _ = self.db.send(Box::new(job));
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64
}

// ---------- database ----------

fn connect() -> Result<Client, postgres::Error> {
    let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL");
    Client::connect(&url, NoTls)
}

fn open_db() -> Client {
    let mut db = connect().expect("could not connect to the database");
    db.batch_execute(
        "CREATE TABLE IF NOT EXISTS people (id TEXT PRIMARY KEY, lat DOUBLE PRECISION NOT NULL, lng DOUBLE PRECISION NOT NULL, seen_at BIGINT NOT NULL);
         CREATE TABLE IF NOT EXISTS questions (id BIGINT PRIMARY KEY, text TEXT NOT NULL, made_at BIGINT NOT NULL, author TEXT);
         CREATE TABLE IF NOT EXISTS replies (id BIGINT PRIMARY KEY, question_id BIGINT NOT NULL, parent BIGINT, text TEXT NOT NULL, made_at BIGINT NOT NULL, was_there BOOLEAN NOT NULL, author TEXT);
         CREATE TABLE IF NOT EXISTS likes (question_id BIGINT NOT NULL, reply_id BIGINT NOT NULL, who TEXT NOT NULL, PRIMARY KEY (question_id, reply_id, who));
         CREATE TABLE IF NOT EXISTS poll_options (question_id BIGINT NOT NULL, idx BIGINT NOT NULL, label TEXT NOT NULL, PRIMARY KEY (question_id, idx));
         CREATE TABLE IF NOT EXISTS votes (question_id BIGINT NOT NULL, who TEXT NOT NULL, idx BIGINT NOT NULL, PRIMARY KEY (question_id, who));
         CREATE TABLE IF NOT EXISTS push (id TEXT PRIMARY KEY, endpoint TEXT NOT NULL, p256dh TEXT NOT NULL, auth TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS profiles (id TEXT PRIMARY KEY, nick TEXT, color TEXT, photo BYTEA, v BIGINT NOT NULL DEFAULT 0);
         ALTER TABLE replies ADD COLUMN IF NOT EXISTS deleted BOOLEAN NOT NULL DEFAULT FALSE;",
    )
    .expect("could not set up the database");
    db
}

// delete questions older than the cutoff, plus everything that belongs to them
fn purge(db: &mut Client, cutoff: u64) {
    let _ = db.execute("DELETE FROM questions WHERE made_at <= $1", &[&(cutoff as i64)]);
    let _ = db.batch_execute(
        "DELETE FROM replies WHERE question_id NOT IN (SELECT id FROM questions);
         DELETE FROM likes WHERE question_id NOT IN (SELECT id FROM questions);
         DELETE FROM poll_options WHERE question_id NOT IN (SELECT id FROM questions);
         DELETE FROM votes WHERE question_id NOT IN (SELECT id FROM questions);",
    );
}

struct Loaded {
    people: HashMap<String, Person>,
    feed: Vec<Question>,
    subs: HashMap<String, push::Sub>,
    profiles: HashMap<String, Prof>,
    next: u64,
}

// read everything back into memory when the server starts
fn load(db: &mut Client) -> Loaded {
    let mut people = HashMap::new();
    for r in db.query("SELECT id, lat, lng, seen_at FROM people", &[]).unwrap() {
        let id: String = r.get(0);
        let lat: f64 = r.get(1);
        let lng: f64 = r.get(2);
        let seen: i64 = r.get(3);
        people.insert(id, Person { lat, lng, zone: zones::find_zone(lat, lng), seen_at: seen as u64 });
    }

    let mut subs = HashMap::new();
    for r in db.query("SELECT id, endpoint, p256dh, auth FROM push", &[]).unwrap() {
        let id: String = r.get(0);
        subs.insert(id, push::Sub { endpoint: r.get(1), p256dh: r.get(2), auth: r.get(3) });
    }

    let mut feed: Vec<Question> = Vec::new();
    let mut next = 0u64;
    for r in db.query("SELECT id, text, made_at, author FROM questions ORDER BY id", &[]).unwrap() {
        let id: i64 = r.get(0);
        let text: String = r.get(1);
        let at: i64 = r.get(2);
        let author: Option<String> = r.get(3);
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
    for r in db.query("SELECT question_id, label FROM poll_options ORDER BY question_id, idx", &[]).unwrap() {
        let qid: i64 = r.get(0);
        let label: String = r.get(1);
        if let Some(q) = feed.iter_mut().find(|q| q.id == qid as u64) {
            q.options.push(PollOption { label, votes: HashSet::new() });
        }
    }
    for r in db.query("SELECT question_id, who, idx FROM votes", &[]).unwrap() {
        let qid: i64 = r.get(0);
        let who: String = r.get(1);
        let idx: i64 = r.get(2);
        if let Some(o) = feed
            .iter_mut()
            .find(|q| q.id == qid as u64)
            .and_then(|q| q.options.get_mut(idx as usize))
        {
            o.votes.insert(who);
        }
    }
    for r in db
        .query(
            "SELECT id, question_id, parent, text, made_at, was_there, author, deleted FROM replies ORDER BY id",
            &[],
        )
        .unwrap()
    {
        let id: i64 = r.get(0);
        let qid: i64 = r.get(1);
        let parent: Option<i64> = r.get(2);
        let text: String = r.get(3);
        let at: i64 = r.get(4);
        let was_there: bool = r.get(5);
        let author: Option<String> = r.get(6);
        let deleted: bool = r.get(7);
        next = next.max(id as u64 + 1);
        if let Some(q) = feed.iter_mut().find(|q| q.id == qid as u64) {
            q.replies.push(Reply {
                id: id as u64,
                parent: parent.map(|p| p as u64),
                author,
                text,
                at: at as u64,
                deleted,
                was_there,
                likes: HashSet::new(),
            });
        }
    }
    for r in db.query("SELECT question_id, reply_id, who FROM likes", &[]).unwrap() {
        let qid: i64 = r.get(0);
        let rid: i64 = r.get(1);
        let who: String = r.get(2);
        let Some(q) = feed.iter_mut().find(|q| q.id == qid as u64) else { continue };
        if rid < 0 {
            q.likes.insert(who);
        } else if let Some(rep) = q.replies.iter_mut().find(|x| x.id == rid as u64) {
            rep.likes.insert(who);
        }
    }

    let mut profiles = HashMap::new();
    for r in db.query("SELECT id, nick, color, photo, v FROM profiles", &[]).unwrap() {
        let id: String = r.get(0);
        let v: i64 = r.get(4);
        profiles.insert(id, Prof { nick: r.get(1), color: r.get(2), photo: r.get(3), v: v as u64 });
    }
    Loaded { people, feed, subs, profiles, next }
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
    let (pid, plat, plng) = (p.id.clone(), p.lat, p.lng);
    app.save(move |db| {
        let _ = db.execute(
            "INSERT INTO people (id, lat, lng, seen_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET lat = $2, lng = $3, seen_at = $4",
            &[&pid, &plat, &plng, &(t as i64)],
        );
    });
    Ok(Json(PingReply { zone }))
}

#[derive(Deserialize)]
struct LeaveIn {
    id: String,
}

// location switched off: forget this phone's dot right away
async fn leave(State(app): State<Shared>, Json(l): Json<LeaveIn>) -> StatusCode {
    if !valid_id(&l.id) {
        return StatusCode::BAD_REQUEST;
    }
    app.people.lock().unwrap().remove(&l.id);
    let id = l.id.clone();
    app.save(move |db| {
        let _ = db.execute("DELETE FROM people WHERE id = $1", &[&id]);
    });
    StatusCode::NO_CONTENT
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
    let (sid, e, p, a) = (s.id.clone(), sub.endpoint.clone(), sub.p256dh.clone(), sub.auth.clone());
    app.save(move |db| {
        let _ = db.execute(
            "INSERT INTO push (id, endpoint, p256dh, auth) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET endpoint = $2, p256dh = $3, auth = $4",
            &[&sid, &e, &p, &a],
        );
    });
    app.subs.lock().unwrap().insert(s.id, sub);
    StatusCode::NO_CONTENT
}

async fn unsubscribe(State(app): State<Shared>, Json(u): Json<UnsubscribeIn>) -> StatusCode {
    app.subs.lock().unwrap().remove(&u.id);
    let uid = u.id.clone();
    app.save(move |db| {
        let _ = db.execute("DELETE FROM push WHERE id = $1", &[&uid]);
    });
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
                    let gone = id.clone();
                    app.save(move |db| {
                        let _ = db.execute("DELETE FROM push WHERE id = $1", &[&gone]);
                    });
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
    deleted: bool,
    mine: bool,
    nick: Option<String>,
    color: Option<String>,
    photo: Option<String>,
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
    mine: bool,
    nick: Option<String>,
    color: Option<String>,
    photo: Option<String>,
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
            let (q_id, q_text, q_at, who) = (q.id as i64, q.text.clone(), q.at as i64, a.id.clone());
            let labels: Vec<String> = q.options.iter().map(|o| o.label.clone()).collect();
            app.save(move |db| {
                let _ = db.execute(
                    "INSERT INTO questions (id, text, made_at, author) VALUES ($1, $2, $3, $4)",
                    &[&q_id, &q_text, &q_at, &who],
                );
                for (i, label) in labels.iter().enumerate() {
                    let _ = db.execute(
                        "INSERT INTO poll_options (question_id, idx, label) VALUES ($1, $2, $3)",
                        &[&q_id, &(i as i64), label],
                    );
                }
                purge(db, t.saturating_sub(QUESTION_LIFE_S));
            });
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
                Some(p) if !p.deleted => p.author.clone(),
                _ => return StatusCode::NOT_FOUND,
            },
        };
        let was_there = q.place.is_some() && q.place == zone_now;
        let id = app.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let (rep_id, q_id, parent, rep_text, who) =
                (id as i64, q.id as i64, r.parent_id.map(|p| p as i64), text.clone(), r.id.clone());
            app.save(move |db| {
                let _ = db.execute(
                    "INSERT INTO replies (id, question_id, parent, text, made_at, was_there, author) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[&rep_id, &q_id, &parent, &rep_text, &(t as i64), &was_there, &who],
                );
            });
        }
        q.replies.push(Reply {
            id,
            parent: r.parent_id,
            author: Some(r.id.clone()),
            text: text.clone(),
            at: t,
            deleted: false,
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
    let (q_id, who) = (l.question_id as i64, l.id.clone());
    app.save(move |db| {
        let _ = if added {
            db.execute(
                "INSERT INTO likes (question_id, reply_id, who) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                &[&q_id, &rid, &who],
            )
        } else {
            db.execute(
                "DELETE FROM likes WHERE question_id = $1 AND reply_id = $2 AND who = $3",
                &[&q_id, &rid, &who],
            )
        };
    });
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
    let (q_id, who, idx) = (v.question_id as i64, v.id.clone(), v.option as i64);
    app.save(move |db| {
        let _ = db.execute(
            "INSERT INTO votes (question_id, who, idx) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            &[&q_id, &who, &idx],
        );
    });
    StatusCode::NO_CONTENT
}

async fn questions(State(app): State<Shared>, Query(me): Query<Me>) -> Json<Vec<QuestionView>> {
    let t = now();
    let who = me.id.as_deref();
    let liked = |s: &HashSet<String>| who.map_or(false, |w| s.contains(w));
    let profiles = app.profiles.lock().unwrap();
    let feed = app.feed.lock().unwrap();
    let mut list: Vec<QuestionView> = feed
        .iter()
        .filter(|q| t.saturating_sub(q.at) < QUESTION_LIFE_S)
        .map(|q| QuestionView {
            mine: who.is_some() && q.author.as_deref() == who,
            nick: face(&profiles, q.author.as_deref()).nick,
            color: face(&profiles, q.author.as_deref()).color,
            photo: face(&profiles, q.author.as_deref()).photo,
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
                    deleted: r.deleted,
                    mine: who.is_some() && r.author.as_deref() == who,
                    nick: face(&profiles, r.author.as_deref()).nick,
                    color: face(&profiles, r.author.as_deref()).color,
                    photo: face(&profiles, r.author.as_deref()).photo,
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

// ---------- profiles and delete ----------

struct Face {
    nick: Option<String>,
    color: Option<String>,
    photo: Option<String>,
}

// a short code for the picture link, so a phone id is never sent to other people
fn key_of(id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("ted", id).hash(&mut h);
    format!("{:016x}", h.finish())
}

fn face(profiles: &HashMap<String, Prof>, author: Option<&str>) -> Face {
    match author.and_then(|a| profiles.get(a).map(|p| (a, p))) {
        Some((a, p)) => Face {
            nick: p.nick.clone(),
            color: p.color.clone(),
            photo: p.photo.as_ref().map(|_| format!("/api/avatar?k={}&v={}", key_of(a), p.v)),
        },
        None => Face { nick: None, color: None, photo: None },
    }
}

#[derive(Deserialize)]
struct ProfileIn {
    id: String,
    nick: Option<String>,
    color: Option<String>,
    photo: Option<String>, // a small jpeg as a data url
}

async fn set_profile(State(app): State<Shared>, Json(p): Json<ProfileIn>) -> StatusCode {
    if !valid_id(&p.id) {
        return StatusCode::BAD_REQUEST;
    }
    let nick = p.nick.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if nick.as_ref().map_or(false, |n| n.chars().count() > 20) {
        return StatusCode::BAD_REQUEST;
    }
    let color = p
        .color
        .filter(|c| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()));
    let photo: Option<Vec<u8>> = match p.photo {
        None => None,
        Some(s) => {
            let Some(b64) = s.strip_prefix("data:image/jpeg;base64,") else {
                return StatusCode::BAD_REQUEST;
            };
            let Ok(bytes) = Base64::decode_vec(b64) else {
                return StatusCode::BAD_REQUEST;
            };
            if bytes.len() > 60_000 || !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return StatusCode::BAD_REQUEST;
            }
            Some(bytes)
        }
    };
    let t = now();
    app.profiles.lock().unwrap().insert(
        p.id.clone(),
        Prof { nick: nick.clone(), color: color.clone(), photo: photo.clone(), v: t },
    );
    let id = p.id.clone();
    app.save(move |db| {
        let _ = db.execute(
            "INSERT INTO profiles (id, nick, color, photo, v) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET nick = $2, color = $3, photo = $4, v = $5",
            &[&id, &nick, &color, &photo, &(t as i64)],
        );
    });
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct AvatarQ {
    k: String,
}

async fn avatar(
    State(app): State<Shared>,
    Query(q): Query<AvatarQ>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), StatusCode> {
    let profiles = app.profiles.lock().unwrap();
    let bytes = profiles
        .iter()
        .find(|(id, _)| key_of(id) == q.k)
        .and_then(|(_, p)| p.photo.clone())
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    ))
}

#[derive(Deserialize)]
struct DeleteIn {
    id: String,
    question_id: u64,
    reply_id: Option<u64>,
}

// only the phone that wrote it can delete it
async fn delete_post(State(app): State<Shared>, Json(d): Json<DeleteIn>) -> StatusCode {
    if !valid_id(&d.id) {
        return StatusCode::BAD_REQUEST;
    }
    let mut feed = app.feed.lock().unwrap();
    let Some(pos) = feed.iter().position(|q| q.id == d.question_id) else {
        return StatusCode::NOT_FOUND;
    };
    let qid = d.question_id as i64;
    match d.reply_id {
        None => {
            if feed[pos].author.as_deref() != Some(d.id.as_str()) {
                return StatusCode::FORBIDDEN;
            }
            feed.remove(pos);
            app.save(move |db| {
                let _ = db.execute("DELETE FROM questions WHERE id = $1", &[&qid]);
                let _ = db.batch_execute(
                    "DELETE FROM replies WHERE question_id NOT IN (SELECT id FROM questions);
                     DELETE FROM likes WHERE question_id NOT IN (SELECT id FROM questions);
                     DELETE FROM poll_options WHERE question_id NOT IN (SELECT id FROM questions);
                     DELETE FROM votes WHERE question_id NOT IN (SELECT id FROM questions);",
                );
            });
        }
        Some(rid) => {
            let q = &mut feed[pos];
            let Some(i) = q.replies.iter().position(|r| r.id == rid) else {
                return StatusCode::NOT_FOUND;
            };
            if q.replies[i].author.as_deref() != Some(d.id.as_str()) {
                return StatusCode::FORBIDDEN;
            }
            if q.replies.iter().any(|r| r.parent == Some(rid)) {
                // it has answers under it, so keep a grey "deleted" line in its place
                let r = &mut q.replies[i];
                r.deleted = true;
                r.text = String::new();
                r.author = None;
                r.likes.clear();
                let r64 = rid as i64;
                app.save(move |db| {
                    let _ = db.execute(
                        "UPDATE replies SET deleted = TRUE, text = '', author = NULL WHERE id = $1",
                        &[&r64],
                    );
                    let _ = db.execute("DELETE FROM likes WHERE question_id = $1 AND reply_id = $2", &[&qid, &r64]);
                });
            } else {
                // nothing under it: remove it, and any empty "deleted" lines above it
                let mut gone = vec![rid];
                let mut cur = q.replies[i].parent;
                q.replies.remove(i);
                while let Some(pid) = cur {
                    let Some(j) = q.replies.iter().position(|r| r.id == pid) else { break };
                    if q.replies[j].deleted && !q.replies.iter().any(|r| r.parent == Some(pid)) {
                        cur = q.replies[j].parent;
                        q.replies.remove(j);
                        gone.push(pid);
                    } else {
                        break;
                    }
                }
                app.save(move |db| {
                    for g in gone {
                        let g = g as i64;
                        let _ = db.execute("DELETE FROM replies WHERE id = $1", &[&g]);
                        let _ = db.execute("DELETE FROM likes WHERE question_id = $1 AND reply_id = $2", &[&qid, &g]);
                    }
                });
            }
        }
    }
    StatusCode::NO_CONTENT
}

fn main() {
    let mut db = open_db();
    let vapid = push::vapid(&mut db);
    let loaded = load(&mut db);
    let (tx, rx) = std::sync::mpsc::channel::<DbJob>();
    std::thread::spawn(move || {
        let mut db = db;
        for job in rx {
            if db.is_closed() {
                if let Ok(c) = connect() {
                    db = c;
                }
            }
            job(&mut db);
        }
    });
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(serve(tx, vapid, loaded));
}

async fn serve(tx: std::sync::mpsc::Sender<DbJob>, vapid: push::Vapid, loaded: Loaded) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let app: Shared = Arc::new(App {
        people: Mutex::new(loaded.people),
        profiles: Mutex::new(loaded.profiles),
        feed: Mutex::new(loaded.feed),
        subs: Mutex::new(loaded.subs),
        sent: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(loaded.next),
        vapid,
        agent,
        db: tx,
    });

    let router = Router::new()
        .route("/api/ping", post(ping))
        .route("/api/leave", post(leave))
        .route("/api/dots", get(dots))
        .route("/api/zones", get(zone_list))
        .route("/api/questions", get(questions).post(ask))
        .route("/api/reply", post(reply))
        .route("/api/like", post(like))
        .route("/api/vote", post(vote))
        .route("/api/profile", post(set_profile))
        .route("/api/avatar", get(avatar))
        .route("/api/delete", post(delete_post))
        .route("/api/push/key", get(push_key))
        .route("/api/push/subscribe", post(subscribe))
        .route("/api/push/unsubscribe", post(unsubscribe))
        .with_state(app);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await.unwrap();
    println!("ted is running on port {port}");
    axum::serve(listener, router).await.unwrap();
}